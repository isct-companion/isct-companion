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
                    rows.is_empty() || rows.iter().any(|row| row.state.is_problem())
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
            crate::school_parser::kyomu::PAGES.iter().any(|(_, page)| {
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
#[path = "tests/school_refresh.rs"]
mod tests;
