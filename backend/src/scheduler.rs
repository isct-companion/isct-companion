use crate::{
    data::DataStore, lifecycle::SchoolSite, pull_coordinator::PullOptions,
    write_scheduled_export_inner,
};
use chrono::{DateTime, Duration, Utc};
use rrule::{RRuleSet, Tz};
use serde_json::{json, Value};
use std::sync::Mutex;
use tauri::{Emitter, Manager};

pub struct Scheduler {
    started_at: DateTime<Utc>,
    running: tokio::sync::Mutex<()>,
    seen_agent_write: Mutex<Option<String>>,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self {
            started_at: Utc::now(),
            running: tokio::sync::Mutex::new(()),
            seen_agent_write: Mutex::new(None),
        }
    }
}

fn timestamp(value: Option<&str>) -> Option<DateTime<Utc>> {
    value
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
}

fn due_occurrence(
    task: &Value,
    now: DateTime<Utc>,
    started_at: DateTime<Utc>,
) -> Result<Option<String>, String> {
    if task.get("enabled").and_then(Value::as_bool) != Some(true) || task.get("deletedAt").is_some()
    {
        return Ok(None);
    }
    let schedule = task
        .get("schedule")
        .and_then(Value::as_str)
        .ok_or("scheduled task has no schedule")?;
    let last = timestamp(task.get("lastOccurrenceAt").and_then(Value::as_str));
    // Check public data immediately on each launch; the hourly recurrence
    // remains the retry path, and refresh_references enforces cache freshness.
    if isct_core::reference_refresh::reference_due_at_launch(task, started_at) {
        return Ok(Some(started_at.to_rfc3339()));
    }
    if schedule == "launch" {
        return Ok(last
            .is_none_or(|last| last < started_at)
            .then(|| started_at.to_rfc3339()));
    }
    let updated = timestamp(task.get("updatedAt").and_then(Value::as_str))
        .ok_or("scheduled task has an invalid updatedAt")?;
    let calendar = format!(
        "DTSTART:{}\n{}",
        updated.format("%Y%m%dT%H%M%SZ"),
        if schedule.starts_with("RRULE:") {
            schedule.to_string()
        } else {
            format!("RRULE:{schedule}")
        }
    );
    let set: RRuleSet = calendar
        .parse()
        .map_err(|error| format!("invalid scheduled task RRULE: {error}"))?;
    let floor = last
        .map(|value| value + Duration::seconds(1))
        .unwrap_or(started_at)
        .max(started_at)
        .with_timezone(&Tz::UTC);
    let dates = set
        .after(floor)
        .before(now.with_timezone(&Tz::UTC))
        .all(1)
        .dates;
    Ok(dates.first().map(DateTime::to_rfc3339))
}

fn task_result_problem(outcomes: &[crate::school_web::IngestOutcome]) -> Result<(), String> {
    if let Some(problem) = outcomes.iter().find(|outcome| outcome.state != "ok") {
        return Err(problem.message.clone());
    }
    Ok(())
}

async fn pull_site(app: &tauri::AppHandle, site: SchoolSite, force: bool) -> Result<(), String> {
    if !force {
        app.state::<crate::school_refresh::SchoolRefresh>()
            .enqueue_scheduled(site);
        return Ok(());
    }
    let outcomes =
        crate::pull_service::school(app, site, None, PullOptions::manual(), true).await?;
    task_result_problem(&outcomes)
}

async fn execute(app: &tauri::AppHandle, action: &str, force: bool) -> Result<String, String> {
    let store = app.state::<DataStore>().inner().clone();
    match action {
        "export" => write_scheduled_export_inner(app, &store).map(|_| "Export completed".into()),
        "reference-refresh" => crate::core_ports::references(app)
            .refresh_all(force, false)
            .await
            .map_err(|error| error.to_string()),
        "lms-refresh" => {
            if !force
                && app
                    .get_webview_window("main")
                    .and_then(|window| window.is_focused().ok())
                    != Some(true)
            {
                return Ok("Paused: app is not active".into());
            }
            pull_site(app, SchoolSite::ScienceLms, force).await?;
            Ok("LMS background refresh completed".into())
        }
        "school-refresh" => {
            for site in [SchoolSite::ScienceLms, SchoolSite::Kyomu] {
                pull_site(app, site, force).await?;
            }
            Ok("School refresh completed".into())
        }
        "hook" => Err("Scheduled custom hooks must select a built-in action.".into()),
        _ => Err("Scheduled task action is unsupported.".into()),
    }
}

impl Scheduler {
    pub fn start(app: tauri::AppHandle) {
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                interval.tick().await;
                let _ = app.state::<Scheduler>().tick(&app).await;
            }
        });
    }

    async fn tick(&self, app: &tauri::AppHandle) -> Result<(), String> {
        let store = app.state::<DataStore>();
        let tasks = store
            .user_rows("scheduled_tasks")
            .map_err(|error| error.to_string())?;
        let now = Utc::now();
        for task in tasks {
            if let Some(occurrence) = due_occurrence(&task, now, self.started_at)? {
                let id = task
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or("task has no id")?;
                let _ = self.run(app, id, Some(occurrence), false).await;
            }
        }
        // MCP is a separate process, so it cannot emit into this Tauri event
        // loop. Its durable write audit is the cross-process notification.
        let latest_agent_write = store
            .load_workspace()
            .map_err(|error| error.to_string())?
            .pointer("/settings/agentAccess/lastWrites")
            .and_then(Value::as_array)
            .and_then(|writes| writes.last())
            .and_then(|write| write.get("at"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut seen = self
            .seen_agent_write
            .lock()
            .map_err(|_| "scheduler is unavailable")?;
        if latest_agent_write != *seen {
            app.emit("workspace-changed", json!({"source":"mcp"}))
                .map_err(|error| error.to_string())?;
        }
        *seen = latest_agent_write;
        Ok(())
    }

    pub async fn run_now(&self, app: &tauri::AppHandle, id: &str) -> Result<String, String> {
        self.run(app, id, Some(Utc::now().to_rfc3339()), true).await
    }

    async fn run(
        &self,
        app: &tauri::AppHandle,
        id: &str,
        occurrence: Option<String>,
        force: bool,
    ) -> Result<String, String> {
        let _guard = if force {
            self.running.lock().await
        } else {
            match self.running.try_lock() {
                Ok(guard) => guard,
                Err(_) => return Ok("Deferred: scheduler queue is busy".into()),
            }
        };
        let result = async {
            let store = app.state::<DataStore>().inner().clone();
            let mut task = store
                .user_rows("scheduled_tasks")
                .map_err(|error| error.to_string())?
                .into_iter()
                .find(|task| task.get("id").and_then(Value::as_str) == Some(id))
                .ok_or("Scheduled task no longer exists.")?;
            let expected = task.get("rev").and_then(Value::as_u64).unwrap_or(0);
            let occurrence = occurrence.unwrap_or_else(|| Utc::now().to_rfc3339());
            task["lastOccurrenceAt"] = Value::String(occurrence);
            task["lastRunAt"] = Value::String(Utc::now().to_rfc3339());
            task["lastResult"] = Value::String("Running".into());
            let mut task = store
                .put_user_record_checked("scheduled_tasks", task, "scheduler", Some(expected))
                .map_err(|error| error.to_string())?;
            app.emit("workspace-changed", json!({"source":"scheduler"}))
                .map_err(|error| error.to_string())?;
            let action = task
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let message = execute(app, action, force)
                .await
                .unwrap_or_else(|error| format!("Failed: {error}"));
            let expected = task.get("rev").and_then(Value::as_u64).unwrap_or(0);
            task["lastRunAt"] = Value::String(Utc::now().to_rfc3339());
            task["lastResult"] = Value::String(message.clone());
            store
                .put_user_record_checked("scheduled_tasks", task, "scheduler", Some(expected))
                .map_err(|error| error.to_string())?;
            app.emit("workspace-changed", json!({"source":"scheduler"}))
                .map_err(|error| error.to_string())?;
            Ok(message)
        }
        .await;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rrule_claims_only_new_occurrences() {
        let task = json!({
            "enabled":true,
            "updatedAt":"2026-10-08T00:00:00Z",
            "schedule":"RRULE:FREQ=HOURLY;INTERVAL=4;BYMINUTE=0"
        });
        let started = "2026-10-08T07:59:00Z".parse().unwrap();
        let now = "2026-10-08T08:01:00Z".parse().unwrap();
        assert!(due_occurrence(&task, now, started).unwrap().is_some());
        let mut claimed = task;
        claimed["lastOccurrenceAt"] = Value::String("2026-10-08T08:00:00Z".into());
        assert!(due_occurrence(&claimed, now, started).unwrap().is_none());
    }
}
