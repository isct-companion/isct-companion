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

/// A shell fact. Unknown kinds are rejected at deserialization instead of
/// being ignored.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum RefreshKind {
    AccountActive,
    Foreground,
    Background,
    LectureOpened,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshEvent {
    pub kind: RefreshKind,
    pub profile: Option<SchoolAuthProfile>,
    pub course_id: Option<u64>,
    pub network: Option<String>,
}

impl RefreshEvent {
    /// The window or app gained (`true`) or lost foreground.
    pub fn lifecycle(foreground: bool) -> Self {
        Self {
            kind: if foreground {
                RefreshKind::Foreground
            } else {
                RefreshKind::Background
            },
            profile: None,
            course_id: None,
            network: None,
        }
    }
}

struct RefreshIntent {
    profiles: HashSet<SchoolAuthProfile>,
    pending: HashSet<SchoolAuthProfile>,
    details: VecDeque<(u64, Instant)>,
    seen: HashSet<u64>,
    network: PullNetwork,
    foreground: bool,
    /// One automatic batch at a time, so its order is the admission order.
    running: bool,
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
            running: false,
        })))
    }

    pub fn event(&self, event: RefreshEvent) -> Result<(), SchoolError> {
        let mut queue = self.0.lock().map_err(|_| SchoolError::RefreshUnavailable)?;
        if let Some(network) = event.network {
            queue.network = parse_network(&network);
        }
        match event.kind {
            RefreshKind::AccountActive => {
                if let Some(profile) = event.profile {
                    queue.profiles.insert(profile);
                    queue.pending.insert(profile);
                }
            }
            RefreshKind::Foreground => {
                queue.foreground = true;
                queue.pending = queue.profiles.clone();
            }
            RefreshKind::Background => queue.foreground = false,
            RefreshKind::LectureOpened => {
                if let Some(course_id) = event.course_id.filter(|id| *id > 0) {
                    if queue.seen.insert(course_id) {
                        queue
                            .details
                            .push_back((course_id, Instant::now() + Duration::from_secs(2)));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn enqueue_scheduled(&self, site: SchoolSite) -> Result<(), SchoolError> {
        self.event(RefreshEvent {
            kind: RefreshKind::AccountActive,
            profile: Some(profile_for_site(site)),
            course_id: None,
            network: None,
        })
    }

    pub fn foreground(&self) -> bool {
        self.0.lock().map(|queue| queue.foreground).unwrap_or(false)
    }

    /// Read the stored facts the refresh policy needs on a blocking worker,
    /// so neither the UI thread nor an executor thread waits on SQLite.
    pub async fn load_inputs(store: &DataStore) -> Result<RefreshInputs, SchoolError> {
        let store = store.clone();
        tokio::task::spawn_blocking(move || {
            let storage = |error: crate::data::DataError| SchoolError::Storage(error.to_string());
            let allow_cellular = store
                .load_workspace()
                .map_err(storage)?
                .pointer("/settings/subscriptionRefresh/allowCellular")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let runs = store.last_import_runs().map_err(storage)?;
            Ok(RefreshInputs {
                allow_cellular,
                runs,
            })
        })
        .await
        .map_err(|error| SchoolError::Storage(error.to_string()))?
    }

    /// Republish pull conditions after a shell fact changed.
    pub async fn publish_conditions(
        &self,
        store: &DataStore,
        coordinator: &PullCoordinator,
        school_visible: bool,
    ) -> Result<(), SchoolError> {
        let inputs = Self::load_inputs(store).await;
        self.apply_conditions(inputs.as_ref(), coordinator, school_visible)
            .await
    }

    /// In-memory part of a wake: publish conditions from already loaded
    /// inputs. A failed read publishes a fail-closed state.
    pub async fn apply_conditions(
        &self,
        inputs: Result<&RefreshInputs, &SchoolError>,
        coordinator: &PullCoordinator,
        school_visible: bool,
    ) -> Result<(), SchoolError> {
        let conditions = inputs
            .map_err(Clone::clone)
            .and_then(|inputs| self.conditions(inputs.allow_cellular, school_visible));
        match conditions {
            Ok(conditions) => {
                coordinator.set_conditions(conditions).await;
                Ok(())
            }
            Err(error) => {
                publish_fail_closed(coordinator).await;
                Err(error)
            }
        }
    }

    /// Admit due work as one sequential batch: full pulls (Science first),
    /// then lecture details. Coordinator FIFO order is assigned when each pull
    /// is submitted, so submitting in sequence keeps that order real.
    pub fn tick(
        &self,
        inputs: &RefreshInputs,
        service: &SchoolService,
        executor: Arc<dyn SchoolExecutor>,
        tasks: &RuntimeTasks,
    ) -> Result<(), SchoolError> {
        let Some((profiles, details)) =
            self.take_ready(&inputs.runs, Utc::now(), Instant::now())?
        else {
            return Ok(());
        };
        let batch = RunningBatch(self.clone());
        let service = service.clone();
        let (batch_profiles, batch_details) = (profiles.clone(), details.clone());
        let admitted = tasks.spawn(async move {
            let refresh = batch.0.clone();
            let _batch = batch;
            for profile in batch_profiles {
                let site = site_for_profile(profile);
                if let Err(error) = service
                    .pull(
                        executor.clone(),
                        site,
                        None,
                        PullOptions::automatic(),
                        false,
                    )
                    .await
                {
                    crate::diagnostics::report("school-refresh", &error);
                }
            }
            for course_id in batch_details {
                let result = service
                    .pull(
                        executor.clone(),
                        SchoolSite::ScienceLms,
                        Some(course_id),
                        PullOptions::automatic(),
                        false,
                    )
                    .await;
                if let Err(error) = &result {
                    crate::diagnostics::report("school-refresh", error);
                }
                if result.as_ref().map_or(true, |rows| {
                    rows.is_empty() || rows.iter().any(|row| row.state != "ok")
                }) {
                    let _ = refresh.retry_detail(course_id);
                }
            }
        });
        if !admitted {
            for profile in profiles {
                self.restore_profile(profile)?;
            }
            for course_id in details {
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

    #[allow(clippy::type_complexity)]
    fn take_ready(
        &self,
        runs: &Value,
        now: DateTime<Utc>,
        instant: Instant,
    ) -> Result<Option<(Vec<SchoolAuthProfile>, Vec<u64>)>, SchoolError> {
        let mut queue = self.0.lock().map_err(|_| SchoolError::RefreshUnavailable)?;
        if queue.running {
            return Ok(None);
        }
        let pending = std::mem::take(&mut queue.pending);
        let mut profiles = pending
            .into_iter()
            .filter(|profile| due(*profile, runs, now))
            .collect::<Vec<_>>();
        profiles.sort_by_key(|profile| usize::from(*profile != SchoolAuthProfile::Science));
        let details = drain_ready_details(&mut queue, instant);
        if profiles.is_empty() && details.is_empty() {
            return Ok(None);
        }
        queue.running = true;
        Ok(Some((profiles, details)))
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

/// Stored facts read once per wake.
pub struct RefreshInputs {
    allow_cellular: bool,
    runs: Value,
}

/// Clears the running flag when a batch finishes, is refused or is dropped.
struct RunningBatch(SchoolRefresh);

impl Drop for RunningBatch {
    fn drop(&mut self) {
        match self.0 .0.lock() {
            Ok(mut queue) => queue.running = false,
            Err(poisoned) => poisoned.into_inner().running = false,
        }
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
                    kind: RefreshKind::AccountActive,
                    profile: Some(profile),
                    course_id: None,
                    network: None,
                })
                .unwrap();
        }
        let now: DateTime<Utc> = "2026-10-12T00:00:00Z".parse().unwrap();
        let (profiles, _) = refresh
            .take_ready(&json!({}), now, Instant::now())
            .unwrap()
            .unwrap();
        refresh.0.lock().unwrap().running = false;
        assert_eq!(
            profiles,
            [SchoolAuthProfile::Science, SchoolAuthProfile::TokyoTech]
        );
        assert!(refresh
            .take_ready(&json!({}), now, Instant::now())
            .unwrap()
            .is_none());
        refresh.event(RefreshEvent::lifecycle(true)).unwrap();
        assert_eq!(
            refresh
                .take_ready(&json!({}), now, Instant::now())
                .unwrap()
                .unwrap()
                .0,
            [SchoolAuthProfile::Science, SchoolAuthProfile::TokyoTech]
        );
        // A running batch defers newly due work instead of interleaving it.
        refresh.event(RefreshEvent::lifecycle(true)).unwrap();
        assert!(refresh
            .take_ready(&json!({}), now, Instant::now())
            .unwrap()
            .is_none());
        drop(RunningBatch(refresh.clone()));
        assert!(refresh
            .take_ready(&json!({}), now, Instant::now())
            .unwrap()
            .is_some());
    }

    #[test]
    fn detail_debounce_and_failure_retry_are_preserved() {
        let refresh = SchoolRefresh::new(PullNetwork::Wifi);
        for _ in 0..3 {
            refresh
                .event(RefreshEvent {
                    kind: RefreshKind::LectureOpened,
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
                kind: RefreshKind::LectureOpened,
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
                kind: RefreshKind::Background,
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
                kind: RefreshKind::LectureOpened,
                profile: None,
                course_id: Some(42),
                network: None,
            })
            .unwrap();
        refresh.0.lock().unwrap().details[0].1 = Instant::now();
        let tasks = RuntimeTasks::default();
        let inputs = SchoolRefresh::load_inputs(&store).await.unwrap();
        refresh
            .tick(&inputs, &service, executor.clone(), &tasks)
            .unwrap();
        tasks.close();
        tasks.wait().await;
        assert_eq!(executor.calls.load(Ordering::Relaxed), 1);

        refresh
            .event(RefreshEvent {
                kind: RefreshKind::LectureOpened,
                profile: None,
                course_id: Some(42),
                network: None,
            })
            .unwrap();
        refresh.0.lock().unwrap().details[0].1 = Instant::now();
        let tasks = RuntimeTasks::default();
        let inputs = SchoolRefresh::load_inputs(&store).await.unwrap();
        refresh
            .tick(&inputs, &service, executor.clone(), &tasks)
            .unwrap();
        tasks.close();
        tasks.wait().await;
        assert_eq!(executor.calls.load(Ordering::Relaxed), 2);

        refresh
            .event(RefreshEvent {
                kind: RefreshKind::LectureOpened,
                profile: None,
                course_id: Some(42),
                network: None,
            })
            .unwrap();
        assert!(refresh.0.lock().unwrap().details.is_empty());
        let tasks = RuntimeTasks::default();
        let inputs = SchoolRefresh::load_inputs(&store).await.unwrap();
        refresh
            .tick(&inputs, &service, executor.clone(), &tasks)
            .unwrap();
        tasks.close();
        tasks.wait().await;
        assert_eq!(executor.calls.load(Ordering::Relaxed), 2);
    }

    struct RecordingExecutor(Mutex<Vec<(SchoolSite, Option<u64>)>>);

    impl SchoolExecutor for RecordingExecutor {
        fn execute(
            &self,
            request: SchoolExecution,
            _context: PullContext,
        ) -> crate::school_service::SchoolExecutorFuture<'_> {
            Box::pin(async move {
                self.0
                    .lock()
                    .unwrap()
                    .push((request.site, request.course_id));
                Ok(vec![IngestOutcome {
                    site: request.site,
                    kind: "lms".into(),
                    state: "ok".into(),
                    changed: 0,
                    message: "Updated".into(),
                }])
            })
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn tick_runs_science_then_kyomu_then_details_in_order() {
        let store = store("batch-order");
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
        let refresh = SchoolRefresh::new(PullNetwork::Wifi);
        refresh
            .event(RefreshEvent {
                kind: RefreshKind::LectureOpened,
                profile: None,
                course_id: Some(42),
                network: None,
            })
            .unwrap();
        refresh.0.lock().unwrap().details[0].1 = Instant::now();
        for profile in [SchoolAuthProfile::TokyoTech, SchoolAuthProfile::Science] {
            refresh
                .event(RefreshEvent {
                    kind: RefreshKind::AccountActive,
                    profile: Some(profile),
                    course_id: None,
                    network: None,
                })
                .unwrap();
        }
        let executor = Arc::new(RecordingExecutor(Mutex::new(Vec::new())));
        let tasks = RuntimeTasks::default();
        let inputs = SchoolRefresh::load_inputs(&store).await.unwrap();
        refresh
            .tick(&inputs, &service, executor.clone(), &tasks)
            .unwrap();
        tasks.close();
        tasks.wait().await;
        assert_eq!(
            *executor.0.lock().unwrap(),
            [
                (SchoolSite::ScienceLms, None),
                (SchoolSite::Kyomu, None),
                (SchoolSite::ScienceLms, Some(42)),
            ]
        );
        assert!(!refresh.0.lock().unwrap().running);
    }
}
