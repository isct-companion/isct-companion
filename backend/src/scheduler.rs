use crate::{
    data::DataStore, lifecycle::SchoolSite, probe_reference_source_inner,
    refresh_reference_source_inner, school_auth::SchoolAuthProfile, write_scheduled_export_inner,
    OsSecretVault, SchoolAuthService, SchoolWebController,
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use rrule::{RRuleSet, Tz};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Mutex};
use tauri::{Emitter, Manager};

const PUBLIC_INDEX: &str =
    "https://raw.githubusercontent.com/isct-companion/Isct-Data/main/index.json";

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
    if task.get("id").and_then(Value::as_str) == Some("builtin:reference-subscriptions")
        && timestamp(task.get("lastRunAt").and_then(Value::as_str))
            .is_none_or(|last| last < started_at)
    {
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

fn pull_site(app: &tauri::AppHandle, site: SchoolSite, force: bool) -> Result<(), String> {
    if !force {
        app.state::<crate::school_refresh::SchoolRefresh>()
            .enqueue_scheduled(site);
        return Ok(());
    }
    let profile = match site {
        SchoolSite::ScienceLms => SchoolAuthProfile::Science,
        SchoolSite::Kyomu => SchoolAuthProfile::TokyoTech,
        _ => return Err("scheduled school pull uses an unsupported site".into()),
    };
    let credentials = app
        .state::<SchoolAuthService<OsSecretVault>>()
        .load(profile)?
        .ok_or("School credentials are not configured.")?;
    let controller = app.state::<SchoolWebController>().inner().clone();
    controller.request_login(app, profile, credentials)?;
    let outcomes = controller.request_pull(app, site, None)?;
    task_result_problem(&outcomes)
}

fn has_reference_cache(workspace: &Value, kind: &str, year: i64) -> bool {
    // Subscription metadata survives backup/restore, but reference caches do
    // not. A saved hash cannot stand in for missing quarter boundaries.
    if kind != "calendar" {
        return true;
    }
    let boundaries = workspace
        .get("academicCalendar")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let kind = row.get("type").and_then(Value::as_str)?;
            let quarter = row.get("quarter").and_then(Value::as_str)?;
            let raw_date = row.get("date").and_then(Value::as_str)?;
            let valid_date = NaiveDate::parse_from_str(raw_date, "%Y-%m-%d")
                .is_ok_and(|date| date.format("%Y-%m-%d").to_string() == raw_date);
            (matches!(kind, "quarter-start" | "quarter-end")
                && valid_date
                && row.get("academicYear").and_then(Value::as_i64) == Some(year)
                && row.get("deletedAt").is_none_or(Value::is_null)
                && row.get("missingSince").is_none_or(Value::is_null))
            .then(|| format!("{kind}:{quarter}"))
        })
        .collect::<std::collections::HashSet<_>>();
    ["1Q", "2Q", "3Q", "4Q"].into_iter().all(|quarter| {
        boundaries.contains(&format!("quarter-start:{quarter}"))
            && boundaries.contains(&format!("quarter-end:{quarter}"))
    })
}

fn reference_is_fresh(row: &Value, year: i64, now: DateTime<Utc>) -> bool {
    row.get("year").and_then(Value::as_i64) == Some(year)
        && row.get("lastError").is_none_or(Value::is_null)
        && timestamp(row.get("fetchedAt").and_then(Value::as_str))
            .is_some_and(|last| now - last < Duration::hours(24))
}

async fn refresh_references(store: &DataStore, force: bool) -> Result<String, String> {
    let workspace = store.load_workspace()?;
    let mut probes = HashMap::<String, Result<String, String>>::new();
    let mut changed = 0u64;
    let mut failures = 0usize;
    for kind in ["calendar", "departments", "important-dates", "exams"] {
        let existing = workspace
            .get("subscriptions")
            .and_then(Value::as_array)
            .and_then(|rows| {
                rows.iter().find(|row| {
                    row.get("kind").and_then(Value::as_str) == Some(kind)
                        && row.get("deletedAt").is_none_or(Value::is_null)
                })
            });
        if existing
            .and_then(|row| row.get("enabled"))
            .and_then(Value::as_bool)
            == Some(false)
        {
            continue;
        }
        let year = if kind == "departments" {
            workspace
                .pointer("/academicProfile/admissionYear")
                .and_then(Value::as_i64)
        } else {
            workspace
                .get("currentTerm")
                .and_then(Value::as_str)
                .and_then(|term| term.get(..4))
                .and_then(|year| year.parse::<i64>().ok())
        };
        let Some(year) = year else { continue };
        let has_cache = has_reference_cache(&workspace, kind, year);
        if !force
            && has_cache
            && existing.is_some_and(|row| reference_is_fresh(row, year, Utc::now()))
        {
            continue;
        }
        let url = existing
            .and_then(|row| row.get("url"))
            .and_then(Value::as_str)
            .filter(|url| url.starts_with("https://"))
            .unwrap_or(PUBLIC_INDEX)
            .to_string();
        let same_year = existing
            .and_then(|row| row.get("year"))
            .and_then(Value::as_i64)
            == Some(year);
        let mut row = existing.cloned().unwrap_or_else(|| {
            json!({
                "id": format!("subscription:public:{kind}"),
                "kind": kind, "url": url, "enabled": true
            })
        });
        let expected_rev = row.get("rev").and_then(Value::as_u64).unwrap_or(0);
        row["url"] = Value::String(url.clone());
        let probe = if let Some(result) = probes.get(&url) {
            result.clone()
        } else {
            let result = probe_reference_source_inner(&url).await;
            probes.insert(url.clone(), result.clone());
            result
        };
        let index_hash = match probe {
            Ok(hash) => hash,
            Err(error) => {
                failures += 1;
                row["lastError"] = Value::String(error);
                store.put_user_record_checked(
                    "subscriptions",
                    row,
                    "scheduler",
                    Some(expected_rev),
                )?;
                continue;
            }
        };
        if same_year
            && has_cache
            && row.get("indexHash").and_then(Value::as_str) == Some(index_hash.as_str())
        {
            row["fetchedAt"] = Value::String(Utc::now().to_rfc3339());
            row["lastError"] = Value::Null;
            store.put_user_record_checked("subscriptions", row, "scheduler", Some(expected_rev))?;
            continue;
        }
        let known_hash = (same_year && has_cache)
            .then(|| {
                row.get("remoteHash")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .flatten();
        match refresh_reference_source_inner(
            store,
            kind,
            &url,
            year as i32,
            Some(index_hash.clone()),
            known_hash,
        )
        .await
        {
            Ok(result) => {
                changed += result.get("changed").and_then(Value::as_u64).unwrap_or(0);
                row["year"] = Value::from(year);
                row["indexHash"] = Value::String(index_hash);
                for (target, source) in [
                    ("dataUrl", "dataUrl"),
                    ("remoteHash", "remoteHash"),
                    ("sourceTs", "sourceTs"),
                ] {
                    if let Some(value) = result.get(source) {
                        row[target] = value.clone();
                    }
                }
                row["fetchedAt"] = Value::String(Utc::now().to_rfc3339());
                row["lastError"] = Value::Null;
            }
            Err(error) => {
                failures += 1;
                row["lastError"] = Value::String(error);
            }
        }
        store.put_user_record_checked("subscriptions", row, "scheduler", Some(expected_rev))?;
    }
    if failures > 0 {
        Err(format!("{failures} reference source(s) failed"))
    } else if changed > 0 {
        Ok(format!("Updated {changed} reference dataset(s)"))
    } else {
        Ok("Reference data is up to date".into())
    }
}

async fn execute(app: &tauri::AppHandle, action: &str, force: bool) -> Result<String, String> {
    let store = app.state::<DataStore>().inner().clone();
    match action {
        "export" => write_scheduled_export_inner(app, &store).map(|_| "Export completed".into()),
        "reference-refresh" => refresh_references(&store, force).await,
        "lms-refresh" => {
            if !force
                && app
                    .get_webview_window("main")
                    .and_then(|window| window.is_focused().ok())
                    != Some(true)
            {
                return Ok("Paused: app is not active".into());
            }
            tauri::async_runtime::spawn_blocking({
                let app = app.clone();
                move || pull_site(&app, SchoolSite::ScienceLms, force)
            })
            .await
            .map_err(|_| "LMS refresh stopped unexpectedly".to_string())??;
            Ok("LMS background refresh completed".into())
        }
        "school-refresh" => {
            for site in [SchoolSite::ScienceLms, SchoolSite::Kyomu] {
                tauri::async_runtime::spawn_blocking({
                    let app = app.clone();
                    move || pull_site(&app, site, force)
                })
                .await
                .map_err(|_| "School refresh stopped unexpectedly".to_string())??;
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
        let tasks = store.user_rows("scheduled_tasks")?;
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
            .load_workspace()?
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
                .user_rows("scheduled_tasks")?
                .into_iter()
                .find(|task| task.get("id").and_then(Value::as_str) == Some(id))
                .ok_or("Scheduled task no longer exists.")?;
            let expected = task.get("rev").and_then(Value::as_u64).unwrap_or(0);
            let occurrence = occurrence.unwrap_or_else(|| Utc::now().to_rfc3339());
            task["lastOccurrenceAt"] = Value::String(occurrence);
            task["lastRunAt"] = Value::String(Utc::now().to_rfc3339());
            task["lastResult"] = Value::String("Running".into());
            let mut task = store.put_user_record_checked(
                "scheduled_tasks",
                task,
                "scheduler",
                Some(expected),
            )?;
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
            store.put_user_record_checked("scheduled_tasks", task, "scheduler", Some(expected))?;
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
    fn reference_subscriptions_run_at_launch_before_the_next_hour() {
        let started = "2026-10-13T03:15:00Z".parse().unwrap();
        let now = "2026-10-13T03:15:01Z".parse().unwrap();
        let mut task = json!({
            "id":"builtin:reference-subscriptions", "enabled":true,
            "updatedAt":"2026-10-12T00:00:00Z",
            "schedule":"RRULE:FREQ=HOURLY;INTERVAL=1;BYMINUTE=0",
            "lastRunAt":"2026-10-12T03:00:00Z"
        });
        assert_eq!(
            due_occurrence(&task, now, started).unwrap(),
            Some(started.to_rfc3339())
        );
        task["lastRunAt"] = json!(now.to_rfc3339());
        task["lastOccurrenceAt"] = json!(started.to_rfc3339());
        assert!(due_occurrence(&task, now, started).unwrap().is_none());
        task["lastRunAt"] = Value::Null;
        task["enabled"] = json!(false);
        assert!(due_occurrence(&task, now, started).unwrap().is_none());
    }

    #[test]
    fn reference_freshness_is_per_source_and_requires_the_requested_year() {
        let now = "2026-10-13T03:15:00Z".parse().unwrap();
        let mut row = json!({"year":2026,"fetchedAt":"2026-10-13T03:00:00Z"});
        assert!(reference_is_fresh(&row, 2026, now));
        assert!(!reference_is_fresh(&row, 2025, now));
        assert!(!reference_is_fresh(&json!({}), 2026, now));
        row["lastError"] = json!("Download failed");
        assert!(!reference_is_fresh(&row, 2026, now));
    }

    #[test]
    fn restored_subscription_hashes_do_not_replace_missing_quarter_data() {
        let mut workspace = json!({"academicCalendar":[]});
        assert!(!has_reference_cache(&workspace, "calendar", 2026));
        workspace["academicCalendar"] = json!([
            {"type":"quarter-start","quarter":"1Q","academicYear":2025,"date":"2025-04-07"}
        ]);
        assert!(!has_reference_cache(&workspace, "calendar", 2026));
        workspace["academicCalendar"][0]["academicYear"] = json!(2026);
        assert!(!has_reference_cache(&workspace, "calendar", 2026));
        workspace["academicCalendar"] = Value::Array(
            ["1Q", "2Q", "3Q", "4Q"]
                .into_iter()
                .flat_map(|quarter| {
                    ["quarter-start", "quarter-end"]
                        .into_iter()
                        .map(move |kind| {
                            json!({
                                "type":kind,"quarter":quarter,"academicYear":2026,
                                "date":"2026-04-01"
                            })
                        })
                })
                .collect(),
        );
        assert!(has_reference_cache(&workspace, "calendar", 2026));
        workspace["academicCalendar"][0]["missingSince"] = json!("2026-10-13T00:00:00Z");
        assert!(!has_reference_cache(&workspace, "calendar", 2026));
        workspace["academicCalendar"][0]["missingSince"] = Value::Null;
        workspace["academicCalendar"][0]["date"] = json!("2026-02-30");
        assert!(!has_reference_cache(&workspace, "calendar", 2026));
    }

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
