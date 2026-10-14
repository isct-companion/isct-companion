//! Refresh intent and freshness policy shared by every native shell.

use crate::{
    contracts::{KYOMU_STALE_SECONDS, LMS_STALE_SECONDS},
    data::DataStore,
    lifecycle::SchoolSite,
    pull_coordinator::{PullConditions, PullCoordinator, PullNetwork, PullOptions},
    runtime_tasks::RuntimeTasks,
    school_auth::SchoolAuthProfile,
    school_service::{SchoolError, SchoolExecutor, SchoolService},
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Deserialize)]
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
    network: PullNetwork,
    foreground: bool,
}

#[derive(Clone)]
pub struct SchoolRefresh(Arc<Mutex<RefreshIntent>>);

impl SchoolRefresh {
    pub fn new(initial_network: PullNetwork) -> Self {
        Self(Arc::new(Mutex::new(RefreshIntent {
            profiles: HashSet::new(),
            pending: HashSet::new(),
            details: VecDeque::new(),
            seen: HashSet::new(),
            network: initial_network,
            foreground: true,
        })))
    }

    pub fn event(&self, event: RefreshEvent) -> Result<(), SchoolError> {
        let mut queue = self.0.lock().map_err(|_| SchoolError::RefreshUnavailable)?;
        if let Some(network) = event.network {
            queue.network = parse_network(&network);
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
                if let Some(course_id) = event.course_id.filter(|id| *id > 0) {
                    if queue.seen.insert(course_id) {
                        queue
                            .details
                            .push_back((course_id, Instant::now() + Duration::from_secs(2)));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub fn enqueue_scheduled(&self, site: SchoolSite) -> Result<(), SchoolError> {
        self.event(RefreshEvent {
            kind: "account-active".into(),
            profile: Some(profile_for_site(site)),
            course_id: None,
            network: None,
        })
    }

    pub fn foreground(&self) -> bool {
        self.0.lock().map(|queue| queue.foreground).unwrap_or(false)
    }

    pub async fn publish_conditions(
        &self,
        store: &DataStore,
        coordinator: &PullCoordinator,
        school_visible: bool,
    ) -> Result<(), SchoolError> {
        let workspace = match store.load_workspace() {
            Ok(workspace) => workspace,
            Err(error) => {
                publish_fail_closed(coordinator).await;
                return Err(SchoolError::Storage(error.to_string()));
            }
        };
        let allow_cellular = workspace
            .pointer("/settings/subscriptionRefresh/allowCellular")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let conditions = match self.conditions(allow_cellular, school_visible) {
            Ok(conditions) => conditions,
            Err(error) => {
                publish_fail_closed(coordinator).await;
                return Err(error);
            }
        };
        coordinator.set_conditions(conditions).await;
        Ok(())
    }

    pub async fn tick(
        &self,
        store: &DataStore,
        service: &SchoolService,
        executor: Arc<dyn SchoolExecutor>,
        tasks: &RuntimeTasks,
    ) -> Result<(), SchoolError> {
        let runs = store
            .last_import_runs()
            .map_err(|error| SchoolError::Storage(error.to_string()))?;
        let (profiles, details) = self.take_ready(&runs, Utc::now(), Instant::now())?;
        for profile in profiles {
            let site = site_for_profile(profile);
            let service = service.clone();
            let executor = executor.clone();
            if !tasks.spawn(async move {
                let _ = service
                    .pull(executor, site, None, PullOptions::automatic(), false)
                    .await;
            }) {
                self.restore_profile(profile)?;
            }
        }
        for course_id in details {
            let refresh = self.clone();
            let service = service.clone();
            let executor = executor.clone();
            if !tasks.spawn(async move {
                let result = service
                    .pull(
                        executor,
                        SchoolSite::ScienceLms,
                        Some(course_id),
                        PullOptions::automatic(),
                        false,
                    )
                    .await;
                if result.as_ref().map_or(true, |rows| {
                    rows.is_empty() || rows.iter().any(|row| row.state != "ok")
                }) {
                    let _ = refresh.retry_detail(course_id);
                }
            }) {
                self.retry_detail(course_id)?;
            }
        }
        Ok(())
    }

    fn conditions(
        &self,
        allow_cellular: bool,
        school_visible: bool,
    ) -> Result<PullConditions, SchoolError> {
        let queue = self.0.lock().map_err(|_| SchoolError::RefreshUnavailable)?;
        Ok(PullConditions {
            network: queue.network,
            foreground: queue.foreground,
            school_visible,
            allow_cellular,
        })
    }

    fn take_ready(
        &self,
        runs: &Value,
        now: DateTime<Utc>,
        instant: Instant,
    ) -> Result<(Vec<SchoolAuthProfile>, Vec<u64>), SchoolError> {
        let mut queue = self.0.lock().map_err(|_| SchoolError::RefreshUnavailable)?;
        let pending = std::mem::take(&mut queue.pending);
        let mut profiles = pending
            .into_iter()
            .filter(|profile| due(*profile, runs, now))
            .collect::<Vec<_>>();
        profiles.sort_by_key(|profile| usize::from(*profile != SchoolAuthProfile::Science));
        let details = drain_ready_details(&mut queue, instant);
        Ok((profiles, details))
    }

    fn restore_profile(&self, profile: SchoolAuthProfile) -> Result<(), SchoolError> {
        self.0
            .lock()
            .map_err(|_| SchoolError::RefreshUnavailable)?
            .pending
            .insert(profile);
        Ok(())
    }

    fn retry_detail(&self, course_id: u64) -> Result<(), SchoolError> {
        self.0
            .lock()
            .map_err(|_| SchoolError::RefreshUnavailable)?
            .seen
            .remove(&course_id);
        Ok(())
    }
}

async fn publish_fail_closed(coordinator: &PullCoordinator) {
    coordinator
        .set_conditions(PullConditions {
            network: PullNetwork::Offline,
            foreground: false,
            school_visible: true,
            allow_cellular: false,
        })
        .await;
}

fn stale(last: Option<&str>, ttl: u64, now: DateTime<Utc>) -> bool {
    last.and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .is_none_or(|last| now.signed_duration_since(last).num_seconds() >= ttl as i64)
}

fn due(profile: SchoolAuthProfile, runs: &Value, now: DateTime<Utc>) -> bool {
    match profile {
        SchoolAuthProfile::Science => stale(runs["lms:full"].as_str(), LMS_STALE_SECONDS, now),
        SchoolAuthProfile::TokyoTech => {
            ["gpa", "registrations", "declared", "grades", "requirements"]
                .iter()
                .any(|page| {
                    stale(
                        runs[format!("kyomu:{page}")].as_str(),
                        KYOMU_STALE_SECONDS,
                        now,
                    )
                })
        }
    }
}

fn parse_network(value: &str) -> PullNetwork {
    match value {
        "offline" => PullNetwork::Offline,
        "wifi" => PullNetwork::Wifi,
        "cellular" => PullNetwork::Cellular,
        _ => PullNetwork::Unknown,
    }
}

fn profile_for_site(site: SchoolSite) -> SchoolAuthProfile {
    if site == SchoolSite::ScienceLms {
        SchoolAuthProfile::Science
    } else {
        SchoolAuthProfile::TokyoTech
    }
}

fn site_for_profile(profile: SchoolAuthProfile) -> SchoolSite {
    match profile {
        SchoolAuthProfile::Science => SchoolSite::ScienceLms,
        SchoolAuthProfile::TokyoTech => SchoolSite::Kyomu,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        events::CoreEvent,
        ports::{EventError, EventSink, SecretError, SecretVault},
        pull_coordinator::PullContext,
        school_auth::SchoolAuthService,
        school_service::{IngestOutcome, SchoolExecution, SchoolExecutorError},
    };
    use serde_json::json;
    use std::{
        collections::VecDeque,
        sync::atomic::{AtomicUsize, Ordering},
    };
    use uuid::Uuid;

    #[derive(Default)]
    struct EmptyVault;

    impl SecretVault for EmptyVault {
        fn store(&self, _account: &str, _secret: &str) -> Result<(), SecretError> {
            Ok(())
        }

        fn load(&self, _account: &str) -> Result<Option<String>, SecretError> {
            Ok(None)
        }

        fn delete(&self, _account: &str) -> Result<(), SecretError> {
            Ok(())
        }
    }

    struct NoopEvents;

    impl EventSink for NoopEvents {
        fn emit(&self, _event: CoreEvent) -> Result<(), EventError> {
            Ok(())
        }
    }

    struct SequencedExecutor {
        calls: AtomicUsize,
        results: Mutex<VecDeque<Result<Vec<IngestOutcome>, SchoolExecutorError>>>,
    }

    impl SchoolExecutor for SequencedExecutor {
        fn execute(
            &self,
            _request: SchoolExecution,
            _context: PullContext,
        ) -> crate::school_service::SchoolExecutorFuture<'_> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::Relaxed);
                self.results.lock().unwrap().pop_front().unwrap()
            })
        }
    }

    fn store(name: &str) -> DataStore {
        let path =
            std::env::temp_dir().join(format!("isct-school-refresh-{name}-{}.db", Uuid::new_v4()));
        DataStore::initialize(path).unwrap().0
    }

    #[test]
    fn freshness_checks_all_five_kyomu_pages_and_exact_ttl() {
        let now: DateTime<Utc> = "2026-10-12T00:00:00Z".parse().unwrap();
        let mut runs = json!({});
        for page in ["gpa", "registrations", "declared", "grades", "requirements"] {
            runs[format!("kyomu:{page}")] = json!(now.to_rfc3339());
        }
        assert!(!due(SchoolAuthProfile::TokyoTech, &runs, now));
        assert!(due(
            SchoolAuthProfile::TokyoTech,
            &runs,
            now + chrono::Duration::seconds(KYOMU_STALE_SECONDS as i64)
        ));
        runs.as_object_mut().unwrap().remove("kyomu:grades");
        assert!(due(SchoolAuthProfile::TokyoTech, &runs, now));
    }

    #[test]
    fn foreground_rechecks_each_known_profile_once() {
        let refresh = SchoolRefresh::new(PullNetwork::Wifi);
        for profile in [SchoolAuthProfile::TokyoTech, SchoolAuthProfile::Science] {
            refresh
                .event(RefreshEvent {
                    kind: "account-active".into(),
                    profile: Some(profile),
                    course_id: None,
                    network: None,
                })
                .unwrap();
        }
        let now: DateTime<Utc> = "2026-10-12T00:00:00Z".parse().unwrap();
        let (profiles, _) = refresh.take_ready(&json!({}), now, Instant::now()).unwrap();
        assert_eq!(
            profiles,
            [SchoolAuthProfile::Science, SchoolAuthProfile::TokyoTech]
        );
        assert!(refresh
            .take_ready(&json!({}), now, Instant::now())
            .unwrap()
            .0
            .is_empty());
        refresh
            .event(RefreshEvent {
                kind: "foreground".into(),
                profile: None,
                course_id: None,
                network: None,
            })
            .unwrap();
        assert_eq!(
            refresh
                .take_ready(&json!({}), now, Instant::now())
                .unwrap()
                .0,
            [SchoolAuthProfile::Science, SchoolAuthProfile::TokyoTech]
        );
    }

    #[test]
    fn detail_debounce_and_failure_retry_are_preserved() {
        let refresh = SchoolRefresh::new(PullNetwork::Wifi);
        for _ in 0..3 {
            refresh
                .event(RefreshEvent {
                    kind: "lecture-opened".into(),
                    profile: None,
                    course_id: Some(42),
                    network: None,
                })
                .unwrap();
        }
        let mut queue = refresh.0.lock().unwrap();
        assert_eq!(queue.details.len(), 1);
        assert!(drain_ready_details(&mut queue, Instant::now()).is_empty());
        assert_eq!(
            drain_ready_details(&mut queue, Instant::now() + Duration::from_secs(3)),
            [42]
        );
        drop(queue);
        refresh.retry_detail(42).unwrap();
        refresh
            .event(RefreshEvent {
                kind: "lecture-opened".into(),
                profile: None,
                course_id: Some(42),
                network: None,
            })
            .unwrap();
        assert_eq!(refresh.0.lock().unwrap().details.len(), 1);
    }

    #[test]
    fn conditions_preserve_shell_facts() {
        let refresh = SchoolRefresh::new(PullNetwork::Cellular);
        refresh
            .event(RefreshEvent {
                kind: "background".into(),
                profile: None,
                course_id: None,
                network: Some("offline".into()),
            })
            .unwrap();
        assert!(!refresh.foreground());
        assert_eq!(
            refresh.conditions(true, true).unwrap(),
            PullConditions {
                network: PullNetwork::Offline,
                foreground: false,
                school_visible: true,
                allow_cellular: true,
            }
        );
    }

    #[tokio::test]
    async fn tick_retries_a_failed_detail_but_keeps_a_successful_one_seen() {
        let store = store("detail-retry");
        let coordinator = PullCoordinator::default();
        coordinator
            .set_conditions(PullConditions {
                network: PullNetwork::Wifi,
                foreground: true,
                school_visible: false,
                allow_cellular: false,
            })
            .await;
        let service = SchoolService::new(
            coordinator,
            Arc::new(SchoolAuthService::new(EmptyVault)),
            Arc::new(NoopEvents),
        );
        let executor = Arc::new(SequencedExecutor {
            calls: AtomicUsize::new(0),
            results: Mutex::new(VecDeque::from([
                Err(SchoolExecutorError::Execution("detail failed".into())),
                Ok(vec![IngestOutcome {
                    site: SchoolSite::ScienceLms,
                    kind: "course-detail".into(),
                    state: "ok".into(),
                    changed: 1,
                    message: "Updated".into(),
                }]),
            ])),
        });
        let refresh = SchoolRefresh::new(PullNetwork::Wifi);

        refresh
            .event(RefreshEvent {
                kind: "lecture-opened".into(),
                profile: None,
                course_id: Some(42),
                network: None,
            })
            .unwrap();
        refresh.0.lock().unwrap().details[0].1 = Instant::now();
        let tasks = RuntimeTasks::default();
        refresh
            .tick(&store, &service, executor.clone(), &tasks)
            .await
            .unwrap();
        tasks.close();
        tasks.wait().await;
        assert_eq!(executor.calls.load(Ordering::Relaxed), 1);

        refresh
            .event(RefreshEvent {
                kind: "lecture-opened".into(),
                profile: None,
                course_id: Some(42),
                network: None,
            })
            .unwrap();
        refresh.0.lock().unwrap().details[0].1 = Instant::now();
        let tasks = RuntimeTasks::default();
        refresh
            .tick(&store, &service, executor.clone(), &tasks)
            .await
            .unwrap();
        tasks.close();
        tasks.wait().await;
        assert_eq!(executor.calls.load(Ordering::Relaxed), 2);

        refresh
            .event(RefreshEvent {
                kind: "lecture-opened".into(),
                profile: None,
                course_id: Some(42),
                network: None,
            })
            .unwrap();
        assert!(refresh.0.lock().unwrap().details.is_empty());
        let tasks = RuntimeTasks::default();
        refresh
            .tick(&store, &service, executor.clone(), &tasks)
            .await
            .unwrap();
        tasks.close();
        tasks.wait().await;
        assert_eq!(executor.calls.load(Ordering::Relaxed), 2);
    }
}
