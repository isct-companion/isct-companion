//! Rust owns school refresh timing. UI events contain facts, never TTL decisions.
use crate::{
    data::DataStore, lifecycle::SchoolSite, school_auth::SchoolAuthProfile,
    school_web::SchoolWebController,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{HashSet, VecDeque},
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::Manager;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshEvent {
    pub kind: String,
    pub profile: Option<SchoolAuthProfile>,
    pub course_id: Option<u64>,
    pub network: Option<String>,
}
struct Queue {
    profiles: HashSet<SchoolAuthProfile>,
    pending: HashSet<SchoolAuthProfile>,
    details: VecDeque<(u64, Instant)>,
    seen: HashSet<u64>,
    network: String,
    foreground: bool,
    running: bool,
}
fn can_run(q: &Queue, allow_cellular: bool, visible: bool) -> bool {
    !q.running
        && q.foreground
        && q.network != "offline"
        && (q.network != "cellular" || allow_cellular)
        && !visible
}
pub struct SchoolRefresh(Mutex<Queue>);
impl Default for SchoolRefresh {
    fn default() -> Self {
        Self(Mutex::new(Queue {
            profiles: HashSet::new(),
            pending: HashSet::new(),
            details: VecDeque::new(),
            seen: HashSet::new(),
            network: if cfg!(mobile) { "cellular" } else { "wifi" }.into(),
            foreground: true,
            running: false,
        }))
    }
}
fn stale(last: Option<&str>, ttl: u64, now: DateTime<Utc>) -> bool {
    last.and_then(|v| DateTime::parse_from_rfc3339(v).ok())
        .is_none_or(|last| now.signed_duration_since(last).num_seconds() >= ttl as i64)
}
fn due(profile: SchoolAuthProfile, runs: &Value, now: DateTime<Utc>) -> bool {
    match profile {
        SchoolAuthProfile::Science => stale(
            runs["lms:full"].as_str(),
            crate::contracts::LMS_STALE_SECONDS,
            now,
        ),
        SchoolAuthProfile::TokyoTech => {
            ["gpa", "registrations", "declared", "grades", "requirements"]
                .iter()
                .any(|page| {
                    stale(
                        runs[format!("kyomu:{page}")].as_str(),
                        crate::contracts::KYOMU_STALE_SECONDS,
                        now,
                    )
                })
        }
    }
}
impl SchoolRefresh {
    pub fn event(&self, event: RefreshEvent) {
        let Ok(mut q) = self.0.lock() else { return };
        if let Some(network) = event.network {
            q.network = network;
        }
        match event.kind.as_str() {
            "account-active" => {
                if let Some(profile) = event.profile {
                    q.profiles.insert(profile);
                    q.pending.insert(profile);
                }
            }
            "foreground" => {
                q.foreground = true;
                q.pending = q.profiles.clone();
            }
            "background" => q.foreground = false,
            "lecture-opened" => {
                if let Some(id) = event.course_id.filter(|id| *id > 0) {
                    if q.seen.insert(id) {
                        q.details
                            .push_back((id, Instant::now() + Duration::from_secs(2)));
                    }
                }
            }
            _ => (),
        }
    }
    pub fn enqueue_scheduled(&self, site: SchoolSite) {
        self.event(RefreshEvent {
            kind: "account-active".into(),
            profile: Some(if site == SchoolSite::ScienceLms {
                SchoolAuthProfile::Science
            } else {
                SchoolAuthProfile::TokyoTech
            }),
            course_id: None,
            network: None,
        });
    }
    pub fn start(app: tauri::AppHandle) {
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let refresh = app.state::<SchoolRefresh>();
                let _ = refresh.tick(&app);
            }
        });
    }
    fn tick(&self, app: &tauri::AppHandle) -> Result<(), String> {
        let store = app.state::<DataStore>();
        let allow_cellular = store
            .load_workspace()?
            .pointer("/settings/subscriptionRefresh/allowCellular")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let runs = store.last_import_runs()?;
        let mut q = self.0.lock().map_err(|_| "refresh queue unavailable")?;
        if !can_run(
            &q,
            allow_cellular,
            app.state::<SchoolWebController>().is_visible(),
        ) {
            return Ok(());
        }
        let pending = std::mem::take(&mut q.pending);
        let mut work: Vec<_> = pending
            .into_iter()
            .filter(|profile| due(*profile, &runs, Utc::now()))
            .collect();
        work.sort_by_key(|p| {
            if *p == SchoolAuthProfile::Science {
                0
            } else {
                1
            }
        });
        let job = if let Some(profile) = work.first().copied() {
            q.pending.extend(work.into_iter().skip(1));
            Some((
                if profile == SchoolAuthProfile::Science {
                    SchoolSite::ScienceLms
                } else {
                    SchoolSite::Kyomu
                },
                None,
            ))
        } else if q
            .details
            .front()
            .is_some_and(|(_, at)| *at <= Instant::now())
        {
            Some((
                SchoolSite::ScienceLms,
                Some(q.details.pop_front().unwrap().0),
            ))
        } else {
            None
        };
        let Some((site, course)) = job else {
            return Ok(());
        };
        q.running = true;
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let result = app
                .state::<SchoolWebController>()
                .request_pull_background(&app, site, course);
            if let Ok(mut q) = app.state::<SchoolRefresh>().0.lock() {
                q.running = false;
                if result.as_ref().map_or(true, |rows| {
                    rows.is_empty() || rows.iter().any(|row| row.state != "ok")
                }) {
                    if let Some(id) = course {
                        q.seen.remove(&id);
                    }
                }
            };
        });
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cellular_and_visible_navigation_defer_without_losing_requests() {
        let refresh = SchoolRefresh::default();
        refresh.enqueue_scheduled(SchoolSite::ScienceLms);
        let mut q = refresh.0.lock().unwrap();
        q.network = "cellular".into();
        assert!(!can_run(&q, false, false));
        assert!(can_run(&q, true, false));
        assert!(!can_run(&q, true, true));
        assert_eq!(q.pending.len(), 1);
        q.foreground = false;
        assert!(!can_run(&q, true, false));
        q.foreground = true;
        q.network = "offline".into();
        assert!(!can_run(&q, true, false));
    }
    #[test]
    fn freshness_checks_every_kyomu_page_and_the_exact_ttl_boundary() {
        let now: DateTime<Utc> = "2026-10-12T00:00:00Z".parse().unwrap();
        let mut runs = serde_json::json!({});
        for page in ["gpa", "registrations", "declared", "grades", "requirements"] {
            runs[format!("kyomu:{page}")] = serde_json::json!(now.to_rfc3339());
        }
        assert!(!due(SchoolAuthProfile::TokyoTech, &runs, now));
        assert!(due(
            SchoolAuthProfile::TokyoTech,
            &runs,
            now + chrono::Duration::days(7)
        ));
        runs.as_object_mut().unwrap().remove("kyomu:grades");
        assert!(due(SchoolAuthProfile::TokyoTech, &runs, now));
        assert!(due(
            SchoolAuthProfile::Science,
            &serde_json::json!({"lms:full":"2026-10-11T12:00:00Z"}),
            now
        ));
    }

    #[test]
    fn ttl_and_session_detail_deduplication() {
        let now = "2026-10-12T00:00:00Z".parse().unwrap();
        assert!(due(SchoolAuthProfile::Science, &serde_json::json!({}), now));
        assert!(!due(
            SchoolAuthProfile::Science,
            &serde_json::json!({"lms:full":"2026-10-12T00:00:00Z"}),
            now
        ));
        let refresh = SchoolRefresh::default();
        for _ in 0..3 {
            refresh.event(RefreshEvent {
                kind: "lecture-opened".into(),
                profile: None,
                course_id: Some(42),
                network: None,
            });
        }
        let q = refresh.0.lock().unwrap();
        assert_eq!(q.details.len(), 1);
        assert!(q.details[0].1 > Instant::now());
    }
}
