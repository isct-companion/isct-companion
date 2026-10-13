//! Coordinated school extraction independent of the native webview implementation.

use crate::{
    events::{CoreEvent, WorkspaceChangeSource},
    lifecycle::SchoolSite,
    ports::EventSink,
    pull_coordinator::{PullContext, PullCoordinator, PullError, PullOptions, PullTarget},
    school_auth::{SchoolAuthError, SchoolAuthProfile, SchoolAuthService, SchoolCredentials},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// One extractor result. The wire contract is shared by desktop and mobile.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestOutcome {
    pub site: SchoolSite,
    pub kind: String,
    /// `ok`, `reconnect-required` or `error`.
    pub state: String,
    pub changed: usize,
    pub message: String,
}

/// Everything a native school adapter needs for one blocking extraction.
pub struct SchoolExecution {
    pub site: SchoolSite,
    pub course_id: Option<u64>,
    pub credentials: Option<(SchoolAuthProfile, SchoolCredentials)>,
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum SchoolExecutorError {
    #[error("{0}")]
    Execution(String),
    #[error("Pull cancelled.")]
    Cancelled,
}

pub trait SchoolExecutor: Send + Sync {
    fn execute(
        &self,
        request: SchoolExecution,
        context: PullContext,
    ) -> Result<Vec<IngestOutcome>, SchoolExecutorError>;
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum SchoolError {
    #[error("{0}")]
    InvalidInput(String),
    #[error(transparent)]
    Auth(#[from] SchoolAuthError),
    #[error("{0}")]
    Executor(String),
    #[error("Pull cancelled.")]
    Cancelled,
    #[error("{0}")]
    Coordination(String),
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
            PullError::InvalidTarget(message) => Self::InvalidInput(message),
            PullError::Execution(message) => Self::Executor(message),
            other => Self::Coordination(other.to_string()),
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

    pub async fn pull(
        &self,
        executor: Arc<dyn SchoolExecutor>,
        site: SchoolSite,
        course_id: Option<u64>,
        options: PullOptions,
        login_first: bool,
    ) -> Result<Vec<IngestOutcome>, SchoolError> {
        validate_request(site, course_id)?;
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
                        let auth = service.auth.clone();
                        let executor_context = context.clone();
                        let result = tokio::task::spawn_blocking(move || {
                            if executor_context.is_cancelled() {
                                return Err(SchoolExecutorError::Cancelled);
                            }
                            let credentials = if login_first {
                                let profile = profile_for_site(site);
                                let credentials = auth
                                    .load(profile)
                                    .map_err(|error| {
                                        SchoolExecutorError::Execution(error.to_string())
                                    })?
                                    .ok_or_else(|| {
                                        SchoolExecutorError::Execution(
                                            "School credentials are not configured.".into(),
                                        )
                                    })?;
                                Some((profile, credentials))
                            } else {
                                None
                            };
                            if executor_context.is_cancelled() {
                                return Err(SchoolExecutorError::Cancelled);
                            }
                            executor.execute(
                                SchoolExecution {
                                    site,
                                    course_id,
                                    credentials,
                                },
                                executor_context,
                            )
                        })
                        .await
                        .map_err(|_| {
                            PullError::Execution("The school pull stopped unexpectedly.".into())
                        })?;
                        let result = match result {
                            Ok(result) => result,
                            Err(SchoolExecutorError::Cancelled) => {
                                return Err(PullError::Cancelled)
                            }
                            Err(error) => return Err(PullError::Execution(error.to_string())),
                        };
                        if context.is_cancelled() {
                            return Err(PullError::Cancelled);
                        }
                        // Partial outcomes already committed useful rows. Preserve the
                        // result while marking the coordinator snapshot as failed.
                        if let Some(problem) = result.iter().find(|row| row.state != "ok") {
                            context.outcome_error(problem.message.clone()).await;
                        }
                        let _ = service.events.emit(CoreEvent::WorkspaceChanged {
                            source: WorkspaceChangeSource::SchoolPull,
                        });
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
mod tests {
    use super::*;
    use crate::{
        ports::{EventError, SecretError, SecretVault},
        pull_coordinator::{PullPriority, PullState},
    };
    use std::sync::Mutex;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    };

    #[derive(Default)]
    struct MemoryVault(Mutex<Option<String>>);

    impl SecretVault for MemoryVault {
        fn store(&self, _account: &str, secret: &str) -> Result<(), SecretError> {
            *self.0.lock().unwrap() = Some(secret.into());
            Ok(())
        }

        fn load(&self, _account: &str) -> Result<Option<String>, SecretError> {
            Ok(self.0.lock().unwrap().clone())
        }

        fn delete(&self, _account: &str) -> Result<(), SecretError> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingEvents(Mutex<Vec<CoreEvent>>);

    impl EventSink for RecordingEvents {
        fn emit(&self, event: CoreEvent) -> Result<(), EventError> {
            self.0.lock().unwrap().push(event);
            Ok(())
        }
    }

    enum Behavior {
        Outcomes(Vec<IngestOutcome>),
        Error(String),
        Panic,
    }

    type RecordedRequest = (SchoolSite, Option<u64>, Option<SchoolAuthProfile>);

    struct FakeExecutor {
        behavior: Mutex<Behavior>,
        requests: Mutex<Vec<RecordedRequest>>,
    }

    struct BlockingVault {
        first: AtomicBool,
        started: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl SecretVault for BlockingVault {
        fn store(&self, _account: &str, _secret: &str) -> Result<(), SecretError> {
            Ok(())
        }

        fn load(&self, _account: &str) -> Result<Option<String>, SecretError> {
            if !self.first.swap(true, Ordering::AcqRel) {
                let _ = self.started.send(());
                self.release
                    .lock()
                    .unwrap()
                    .recv()
                    .map_err(|_| SecretError::Access("test release failed".into()))?;
            }
            Ok(None)
        }

        fn delete(&self, _account: &str) -> Result<(), SecretError> {
            Ok(())
        }
    }

    struct CountingExecutor(AtomicUsize);

    impl SchoolExecutor for CountingExecutor {
        fn execute(
            &self,
            _request: SchoolExecution,
            _context: PullContext,
        ) -> Result<Vec<IngestOutcome>, SchoolExecutorError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(vec![outcome("ok")])
        }
    }

    impl FakeExecutor {
        fn new(behavior: Behavior) -> Self {
            Self {
                behavior: Mutex::new(behavior),
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    impl SchoolExecutor for FakeExecutor {
        fn execute(
            &self,
            request: SchoolExecution,
            _context: PullContext,
        ) -> Result<Vec<IngestOutcome>, SchoolExecutorError> {
            self.requests.lock().unwrap().push((
                request.site,
                request.course_id,
                request.credentials.as_ref().map(|(profile, _)| *profile),
            ));
            match &*self.behavior.lock().unwrap() {
                Behavior::Outcomes(rows) => Ok(rows.clone()),
                Behavior::Error(message) => Err(SchoolExecutorError::Execution(message.clone())),
                Behavior::Panic => panic!("native executor panic"),
            }
        }
    }

    fn outcome(state: &str) -> IngestOutcome {
        IngestOutcome {
            site: SchoolSite::Kyomu,
            kind: "grades".into(),
            state: state.into(),
            changed: usize::from(state == "ok"),
            message: if state == "ok" {
                "Updated".into()
            } else {
                "Sign in".into()
            },
        }
    }

    fn service(coordinator: PullCoordinator, auth: Arc<SchoolAuthService>) -> SchoolService {
        SchoolService::new(coordinator, auth, Arc::new(RecordingEvents::default()))
    }

    #[tokio::test]
    async fn partial_outcomes_are_preserved_and_mark_the_snapshot_failed() {
        let coordinator = PullCoordinator::default();
        let service = service(
            coordinator.clone(),
            Arc::new(SchoolAuthService::new(MemoryVault::default())),
        );
        let expected = vec![outcome("ok"), outcome("reconnect-required")];
        let rows = service
            .pull(
                Arc::new(FakeExecutor::new(Behavior::Outcomes(expected.clone()))),
                SchoolSite::Kyomu,
                None,
                PullOptions::manual(),
                false,
            )
            .await
            .unwrap();
        assert_eq!(rows, expected);
        assert_eq!(coordinator.snapshots().await[0].state, PullState::Failed);
    }

    #[tokio::test]
    async fn login_first_loads_the_correct_profile_once() {
        let auth = Arc::new(SchoolAuthService::new(MemoryVault::default()));
        auth.save(
            SchoolAuthProfile::Science,
            SchoolCredentials {
                username: "student".into(),
                password: "secret-password".into(),
                otp_secret: "JBSWY3DPEHPK3PXP".into(),
                matrix: String::new(),
            },
        )
        .unwrap();
        let executor = Arc::new(FakeExecutor::new(Behavior::Outcomes(vec![outcome("ok")])));
        service(PullCoordinator::default(), auth)
            .pull(
                executor.clone(),
                SchoolSite::ScienceLms,
                None,
                PullOptions::manual(),
                true,
            )
            .await
            .unwrap();
        assert_eq!(
            executor.requests.lock().unwrap().as_slice(),
            &[(
                SchoolSite::ScienceLms,
                None,
                Some(SchoolAuthProfile::Science)
            )]
        );
    }

    #[tokio::test]
    async fn executor_errors_and_panics_release_the_school_lane() {
        for behavior in [Behavior::Error("adapter failed".into()), Behavior::Panic] {
            let coordinator = PullCoordinator::default();
            let service = service(
                coordinator,
                Arc::new(SchoolAuthService::new(MemoryVault::default())),
            );
            assert!(service
                .pull(
                    Arc::new(FakeExecutor::new(behavior)),
                    SchoolSite::Kyomu,
                    None,
                    PullOptions::manual(),
                    false,
                )
                .await
                .is_err());
            let rows = service
                .pull(
                    Arc::new(FakeExecutor::new(Behavior::Outcomes(vec![outcome("ok")]))),
                    SchoolSite::Kyomu,
                    None,
                    PullOptions::manual(),
                    false,
                )
                .await
                .unwrap();
            assert_eq!(rows, [outcome("ok")]);
        }
    }

    #[tokio::test]
    async fn executor_failures_do_not_put_credentials_in_snapshots() {
        let coordinator = PullCoordinator::default();
        let auth = Arc::new(SchoolAuthService::new(MemoryVault::default()));
        auth.save(
            SchoolAuthProfile::Science,
            SchoolCredentials {
                username: "student".into(),
                password: "never-print-this".into(),
                otp_secret: "JBSWY3DPEHPK3PXP".into(),
                matrix: String::new(),
            },
        )
        .unwrap();
        let service = service(coordinator.clone(), auth);
        let error = service
            .pull(
                Arc::new(FakeExecutor::new(Behavior::Error("adapter failed".into()))),
                SchoolSite::ScienceLms,
                None,
                PullOptions {
                    priority: PullPriority::Manual,
                    ..PullOptions::manual()
                },
                true,
            )
            .await
            .unwrap_err();
        let diagnostic = format!("{error:?} {:?}", coordinator.snapshots().await);
        assert!(!diagnostic.contains("never-print-this"));
        assert!(!diagnostic.contains("JBSWY3DPEHPK3PXP"));
    }

    #[tokio::test]
    async fn cancellation_while_the_vault_is_blocked_never_starts_the_executor() {
        let coordinator = PullCoordinator::default();
        let (started_send, started_receive) = mpsc::channel();
        let (release_send, release_receive) = mpsc::channel();
        let auth = Arc::new(SchoolAuthService::new(BlockingVault {
            first: AtomicBool::new(false),
            started: started_send,
            release: Mutex::new(release_receive),
        }));
        let executor = Arc::new(CountingExecutor(AtomicUsize::new(0)));
        let service = service(coordinator.clone(), auth);
        let task = tokio::spawn({
            let executor = executor.clone();
            async move {
                service
                    .pull(
                        executor,
                        SchoolSite::ScienceLms,
                        None,
                        PullOptions::manual(),
                        true,
                    )
                    .await
            }
        });
        tokio::task::spawn_blocking(move || started_receive.recv().unwrap())
            .await
            .unwrap();
        let id = coordinator.snapshots().await[0].id.clone();
        assert!(coordinator.cancel(&id).await);
        release_send.send(()).unwrap();
        assert!(matches!(task.await.unwrap(), Err(SchoolError::Cancelled)));
        assert_eq!(executor.0.load(Ordering::Relaxed), 0);
    }
}
