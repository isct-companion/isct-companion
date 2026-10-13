//! Rust owns school refresh timing. UI events contain facts, never TTL decisions.
use crate::{
    data::DataStore,
    lifecycle::SchoolSite,
    pull_coordinator::{PullConditions, PullCoordinator, PullNetwork, PullOptions},
    pull_service,
    school_auth::SchoolAuthProfile,
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

struct RefreshIntent {
    profiles: HashSet<SchoolAuthProfile>,
    pending: HashSet<SchoolAuthProfile>,
    details: VecDeque<(u64, Instant)>,
    seen: HashSet<u64>,
    network: String,
    foreground: bool,
}

pub struct SchoolRefresh(Mutex<RefreshIntent>);

impl Default for SchoolRefresh {
    fn default() -> Self {
        Self(Mutex::new(RefreshIntent {
            profiles: HashSet::new(),
            pending: HashSet::new(),
            details: VecDeque::new(),
            seen: HashSet::new(),
            network: if cfg!(mobile) { "cellular" } else { "wifi" }.into(),
            foreground: true,
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

fn network(value: &str) -> PullNetwork {
    match value {
        "offline" => PullNetwork::Offline,
        "wifi" => PullNetwork::Wifi,
        "cellular" => PullNetwork::Cellular,
        _ => PullNetwork::Unknown,
    }
}

fn pull_conditions(
    queue: &RefreshIntent,
    allow_cellular: bool,
    school_visible: bool,
) -> PullConditions {
    PullConditions {
        network: network(&queue.network),
        foreground: queue.foreground,
        school_visible,
        allow_cellular,
    }
}

fn drain_ready_details(queue: &mut RefreshIntent, now: Instant) -> Vec<u64> {
    let mut ready = Vec::new();
    while queue.details.front().is_some_and(|(_, at)| *at <= now) {
        if let Some((course_id, _)) = queue.details.pop_front() {
            ready.push(course_id);
        }
    }
    ready
}

impl SchoolRefresh {
    pub fn event(&self, event: RefreshEvent) {
        let Ok(mut queue) = self.0.lock() else {
            return;
        };
        if let Some(network) = event.network {
            queue.network = network;
        }
        match event.kind.as_str() {
            "account-active" => {
                if let Some(profile) = event.profile {
                    queue.profiles.insert(profile);
                    queue.pending.insert(profile);
                }
            }
            "foreground" => {
                queue.foreground = true;
                queue.pending = queue.profiles.clone();
            }
            "background" => queue.foreground = false,
            "lecture-opened" => {
                if let Some(id) = event.course_id.filter(|id| *id > 0) {
                    if queue.seen.insert(id) {
                        queue
                            .details
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
        app.state::<SchoolRefresh>().publish_conditions(&app);
        tauri::async_runtime::spawn(async move {
            loop {
                let refresh = app.state::<SchoolRefresh>();
                let _ = refresh.tick(&app).await;
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
    }

    /// Publishes lifecycle/network facts synchronously so an automatic pull
    /// cannot be admitted during the interval before the next one-second tick.
    pub fn publish_conditions(&self, app: &tauri::AppHandle) {
        let conditions = self.current_conditions(app);
        let coordinator = app.state::<PullCoordinator>().inner().clone();
        tauri::async_runtime::block_on(coordinator.set_conditions(conditions));
    }

    fn current_conditions(&self, app: &tauri::AppHandle) -> PullConditions {
        let allow_cellular = app
            .state::<DataStore>()
            .load_workspace()
            .ok()
            .and_then(|workspace| {
                workspace
                    .pointer("/settings/subscriptionRefresh/allowCellular")
                    .and_then(Value::as_bool)
            })
            .unwrap_or(false);
        let school_visible = app.state::<SchoolWebController>().is_visible();
        self.0
            .lock()
            .map(|queue| pull_conditions(&queue, allow_cellular, school_visible))
            .unwrap_or(PullConditions {
                network: PullNetwork::Offline,
                foreground: false,
                school_visible: true,
                allow_cellular: false,
            })
    }

    async fn tick(&self, app: &tauri::AppHandle) -> Result<(), String> {
        let conditions = self.current_conditions(app);
        app.state::<PullCoordinator>()
            .set_conditions(conditions)
            .await;

        let store = app.state::<DataStore>().inner().clone();
        let runs = store
            .last_import_runs()
            .map_err(|error| error.to_string())?;
        let (mut profiles, details) = {
            let mut queue = self.0.lock().map_err(|_| "refresh intent unavailable")?;
            let pending = std::mem::take(&mut queue.pending);
            let profiles = pending
                .into_iter()
                .filter(|profile| due(*profile, &runs, Utc::now()))
                .collect::<Vec<_>>();
            let details = drain_ready_details(&mut queue, Instant::now());
            (profiles, details)
        };
        profiles.sort_by_key(|profile| {
            if *profile == SchoolAuthProfile::Science {
                0
            } else {
                1
            }
        });
        for profile in profiles {
            let site = if profile == SchoolAuthProfile::Science {
                SchoolSite::ScienceLms
            } else {
                SchoolSite::Kyomu
            };
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ =
                    pull_service::school(&app, site, None, PullOptions::automatic(), false).await;
            });
        }
        for course_id in details {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let result = pull_service::school(
                    &app,
                    SchoolSite::ScienceLms,
                    Some(course_id),
                    PullOptions::automatic(),
                    false,
                )
                .await;
                if result.as_ref().map_or(true, |rows| {
                    rows.is_empty() || rows.iter().any(|row| row.state != "ok")
                }) {
                    if let Ok(mut queue) = app.state::<SchoolRefresh>().0.lock() {
                        queue.seen.remove(&course_id);
                    }
                }
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditions_preserve_platform_defaults_and_external_facts() {
        let refresh = SchoolRefresh::default();
        let mut queue = refresh.0.lock().unwrap();
        assert_eq!(
            network(&queue.network),
            if cfg!(mobile) {
                PullNetwork::Cellular
            } else {
                PullNetwork::Wifi
            }
        );
        queue.network = "offline".into();
        queue.foreground = false;
        assert_eq!(
            pull_conditions(&queue, true, true),
            PullConditions {
                network: PullNetwork::Offline,
                foreground: false,
                school_visible: true,
                allow_cellular: true,
            }
        );
        queue.network = "unexpected".into();
        assert_eq!(network(&queue.network), PullNetwork::Unknown);
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
    fn ttl_and_session_detail_debounce_are_preserved() {
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
        let mut queue = refresh.0.lock().unwrap();
        assert_eq!(queue.details.len(), 1);
        assert!(drain_ready_details(&mut queue, Instant::now()).is_empty());
        let ready = drain_ready_details(&mut queue, Instant::now() + Duration::from_secs(3));
        assert_eq!(ready, [42]);
        assert!(queue.seen.contains(&42));
    }
}
