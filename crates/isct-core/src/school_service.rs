//! Coordinated school extraction independent of the native webview implementation.

use crate::{
    events::{CoreEvent, WorkspaceChangeSource},
    lifecycle::SchoolSite,
    ports::EventSink,
    pull_coordinator::{PullContext, PullCoordinator, PullError, PullOptions, PullTarget},
    school_auth::{SchoolAuthError, SchoolAuthProfile, SchoolAuthService, SchoolCredentials},
};
use serde::{Deserialize, Serialize};
use std::{future::Future, pin::Pin, sync::Arc};
#[cfg(feature = "ts")]
use ts_rs::TS;

/// One extractor result. The wire contract is shared by desktop and mobile.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestOutcome {
    pub site: SchoolSite,
    pub kind: String,
    pub state: IngestState,
    pub changed: usize,
    pub message: String,
}

/// How one extractor result ended.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum IngestState {
    Ok,
    Unavailable,
    Maintenance,
    ReconnectRequired,
    Error,
}

impl IngestState {
    /// Unavailable is a complete capability observation, not a failed pull.
    pub fn is_problem(self) -> bool {
        matches!(
            self,
            Self::Maintenance | Self::ReconnectRequired | Self::Error
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "kebab-case")]
pub enum SchoolPageAvailabilityState {
    Available,
    Unavailable,
}

/// Latest authoritative capability observation for one school page.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SchoolPageAvailability {
    pub state: SchoolPageAvailabilityState,
    pub checked_at: String,
    pub message: String,
}

#[cfg(feature = "ts")]
pub fn declarations() -> Vec<String> {
    vec![
        SchoolPageAvailabilityState::decl(),
        SchoolPageAvailability::decl(),
    ]
}

/// One coordinated request consumed by the Core school browser engine.
pub struct SchoolExecution {
    pub site: SchoolSite,
    pub course_id: Option<u64>,
    /// Saved credentials when the job asked to sign in before it started.
    /// A join after the start is visible through `PullContext::login_first`.
    pub credentials: Option<(SchoolAuthProfile, SchoolCredentials)>,
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum SchoolExecutorError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Authentication(String),
    #[error("{0}")]
    Maintenance(String),
    #[error("{0}")]
    Storage(String),
    #[error("{0}")]
    Timeout(String),
    #[error("{0}")]
    Stopped(String),
    #[error("{0}")]
    SiteChanged(String),
    #[error("{0}")]
    Execution(String),
    #[error("Pull cancelled.")]
    Cancelled,
}

pub type SchoolExecutorFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<IngestOutcome>, SchoolExecutorError>> + Send + 'a>>;

pub trait SchoolExecutor: Send + Sync {
    fn execute(&self, request: SchoolExecution, context: PullContext) -> SchoolExecutorFuture<'_>;
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum SchoolError {
    #[error("{0}")]
    InvalidInput(String),
    #[error(transparent)]
    Auth(#[from] SchoolAuthError),
    #[error("{0}")]
    Executor(String),
    #[error("{0}")]
    Authentication(String),
    #[error("{0}")]
    Maintenance(String),
    #[error("{0}")]
    Timeout(String),
    #[error("{0}")]
    Stopped(String),
    #[error("{0}")]
    SiteChanged(String),
    #[error("Pull cancelled.")]
    Cancelled,
    #[error("{0}")]
    Coordination(#[source] PullError),
    #[error("School pull returned an invalid result.")]
    Compatibility,
    #[error("refresh intent unavailable")]
    RefreshUnavailable,
    #[error("{0}")]
    Storage(String),
}

impl From<PullError> for SchoolError {
    fn from(error: PullError) -> Self {
        match error {
            PullError::Cancelled => Self::Cancelled,
            PullError::InvalidTarget(message) | PullError::InvalidInput(message) => {
                Self::InvalidInput(message)
            }
            PullError::Authentication(message) => Self::Authentication(message),
            PullError::Maintenance(message) => Self::Maintenance(message),
            PullError::Storage(message) => Self::Storage(message),
            PullError::Timeout(message) => Self::Timeout(message),
            PullError::ServiceStopped(message) => Self::Stopped(message),
            PullError::SiteChanged(message) => Self::SiteChanged(message),
            PullError::Execution(message) => Self::Executor(message),
            other => Self::Coordination(other),
        }
    }
}

#[derive(Clone)]
pub struct SchoolService {
    coordinator: PullCoordinator,
    auth: Arc<SchoolAuthService>,
    events: Arc<dyn EventSink>,
}

impl SchoolService {
    pub fn new(
        coordinator: PullCoordinator,
        auth: Arc<SchoolAuthService>,
        events: Arc<dyn EventSink>,
    ) -> Self {
        Self {
            coordinator,
            auth,
            events,
        }
    }

    /// Current-quarter content selection is shared by shells; each course
    /// still uses the coordinator's normal ownership and cancellation path.
    pub async fn pull_current_term(
        &self,
        store: &crate::data::DataStore,
        executor: Arc<dyn SchoolExecutor>,
    ) -> Result<Vec<IngestOutcome>, SchoolError> {
        let snapshot = store
            .workspace_snapshot_parts()
            .map_err(|e| SchoolError::Storage(e.to_string()))?;
        let term = crate::schedule::Term::parse(snapshot.0["currentTerm"].as_str().unwrap_or(""))
            .map_err(|e| SchoolError::InvalidInput(e.to_string()))?;
        let quarter = format!("{}Q", term.quarter);
        let ids: Vec<u64> = snapshot.1["lmsCourses"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| {
                c["year"].as_i64() == Some(i64::from(term.year))
                    && c["quarters"]
                        .as_array()
                        .is_some_and(|q| q.iter().any(|v| v.as_str() == Some(&quarter)))
            })
            .filter_map(|c| c["lmsId"].as_u64())
            .collect();
        if ids.is_empty() {
            return Err(SchoolError::InvalidInput(
                "No current-quarter LMS courses are cached. Pull all courses first.".into(),
            ));
        }
        let mut outcomes = Vec::new();
        for id in ids {
            outcomes.extend(
                self.pull(
                    executor.clone(),
                    SchoolSite::ScienceLms,
                    Some(id),
                    PullOptions::manual(),
                    false,
                )
                .await?,
            );
        }
        Ok(outcomes)
    }

    pub async fn pull(
        &self,
        executor: Arc<dyn SchoolExecutor>,
        site: SchoolSite,
        course_id: Option<u64>,
        mut options: PullOptions,
        login_first: bool,
    ) -> Result<Vec<IngestOutcome>, SchoolError> {
        validate_request(site, course_id)?;
        // A caller joining a live job of the same key upgrades it, so the flag
        // travels with the job rather than with this caller's closure.
        options.login_first |= login_first;
        let service = self.clone();
        let value = self
            .coordinator
            .run(
                PullTarget::School { site, course_id },
                options,
                move |context| {
                    let service = service.clone();
                    let executor = executor.clone();
                    async move {
                        if !context.phase("extracting").await {
                            return Err(PullError::Cancelled);
                        }
                        if context.is_cancelled() {
                            return Err(PullError::Cancelled);
                        }
                        // A later join can still ask for sign-in; the executor reads
                        // the job's flag again before each extraction.
                        let credentials = if context.login_first() {
                            let auth = service.auth.clone();
                            let profile = profile_for_site(site);
                            let loaded = tokio::task::spawn_blocking(move || auth.load(profile))
                                .await
                                .map_err(|error| {
                                    if error.is_panic() {
                                        PullError::ExecutorPanicked
                                    } else {
                                        PullError::ExecutorStopped
                                    }
                                })?
                                .map_err(map_auth_error)
                                .map_err(map_executor_error)?;
                            Some((
                                profile,
                                loaded.ok_or_else(|| {
                                    PullError::Authentication(
                                        "School credentials are not configured.".into(),
                                    )
                                })?,
                            ))
                        } else {
                            None
                        };
                        if context.is_cancelled() {
                            return Err(PullError::Cancelled);
                        }
                        let result = executor
                            .execute(
                                SchoolExecution {
                                    site,
                                    course_id,
                                    credentials,
                                },
                                context.clone(),
                            )
                            .await;
                        // 教務Web commits per page, so a run cancelled between pages
                        // may already have changed rows.
                        if matches!(result, Ok(_) | Err(SchoolExecutorError::Cancelled)) {
                            let _ = service.events.emit(CoreEvent::WorkspaceChanged {
                                source: WorkspaceChangeSource::SchoolPull,
                                revision: None,
                            });
                        }
                        let result = result.map_err(map_executor_error)?;
                        if context.is_cancelled() {
                            return Err(PullError::Cancelled);
                        }
                        // Partial outcomes already committed useful rows. Preserve the
                        // result while marking the coordinator snapshot as failed.
                        if let Some(problem) = result.iter().find(|row| row.state.is_problem()) {
                            context.outcome_error(problem.message.clone()).await;
                        }
                        serde_json::to_value(result)
                            .map_err(|error| PullError::Execution(error.to_string()))
                    }
                },
            )
            .await
            .map_err(SchoolError::from)?;
        serde_json::from_value(value).map_err(|_| SchoolError::Compatibility)
    }
}

pub(crate) fn map_executor_error(error: SchoolExecutorError) -> PullError {
    match error {
        SchoolExecutorError::InvalidInput(message) => PullError::InvalidInput(message),
        SchoolExecutorError::Authentication(message) => PullError::Authentication(message),
        SchoolExecutorError::Maintenance(message) => PullError::Maintenance(message),
        SchoolExecutorError::Storage(message) => PullError::Storage(message),
        SchoolExecutorError::Timeout(message) => PullError::Timeout(message),
        SchoolExecutorError::Stopped(message) => PullError::ServiceStopped(message),
        SchoolExecutorError::SiteChanged(message) => PullError::SiteChanged(message),
        SchoolExecutorError::Execution(message) => PullError::Execution(message),
        SchoolExecutorError::Cancelled => PullError::Cancelled,
    }
}

fn map_auth_error(error: SchoolAuthError) -> SchoolExecutorError {
    match error {
        SchoolAuthError::InvalidInput(message) => SchoolExecutorError::InvalidInput(message),
        SchoolAuthError::Storage(message) => SchoolExecutorError::Storage(message),
        other => SchoolExecutorError::Authentication(other.to_string()),
    }
}

fn validate_request(site: SchoolSite, course_id: Option<u64>) -> Result<(), SchoolError> {
    if !matches!(site, SchoolSite::ScienceLms | SchoolSite::Kyomu) {
        return Err(SchoolError::InvalidInput(
            "Only LMS and 教務Web can be pulled.".into(),
        ));
    }
    if course_id == Some(0) || (course_id.is_some() && site != SchoolSite::ScienceLms) {
        return Err(SchoolError::InvalidInput(
            "LMS course ID must be positive and use the LMS site.".into(),
        ));
    }
    Ok(())
}

fn profile_for_site(site: SchoolSite) -> SchoolAuthProfile {
    if site == SchoolSite::ScienceLms {
        SchoolAuthProfile::Science
    } else {
        SchoolAuthProfile::TokyoTech
    }
}

#[cfg(test)]
#[path = "tests/school_service.rs"]
mod tests;
