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
    Typed(SchoolExecutorError),
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
    ) -> SchoolExecutorFuture<'_> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(vec![outcome(IngestState::Ok)])
        })
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
    fn execute(&self, request: SchoolExecution, _context: PullContext) -> SchoolExecutorFuture<'_> {
        Box::pin(async move {
            self.requests.lock().unwrap().push((
                request.site,
                request.course_id,
                request.credentials.as_ref().map(|(profile, _)| *profile),
            ));
            match &*self.behavior.lock().unwrap() {
                Behavior::Outcomes(rows) => Ok(rows.clone()),
                Behavior::Error(message) => Err(SchoolExecutorError::Execution(message.clone())),
                Behavior::Typed(error) => Err(error.clone()),
                Behavior::Panic => panic!("native executor panic"),
            }
        })
    }
}

fn outcome(state: IngestState) -> IngestOutcome {
    IngestOutcome {
        site: SchoolSite::Kyomu,
        kind: "grades".into(),
        state,
        changed: usize::from(state == IngestState::Ok),
        message: if state == IngestState::Ok {
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
    let expected = vec![
        outcome(IngestState::Ok),
        outcome(IngestState::ReconnectRequired),
    ];
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
    let executor = Arc::new(FakeExecutor::new(Behavior::Outcomes(vec![outcome(
        IngestState::Ok,
    )])));
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
                Arc::new(FakeExecutor::new(Behavior::Outcomes(vec![outcome(
                    IngestState::Ok,
                )]))),
                SchoolSite::Kyomu,
                None,
                PullOptions::manual(),
                false,
            )
            .await
            .unwrap();
        assert_eq!(rows, [outcome(IngestState::Ok)]);
    }
}

#[tokio::test]
async fn typed_executor_errors_keep_their_category_and_release_the_lane() {
    let cases = [
        (
            SchoolExecutorError::Maintenance("Official outage notice".into()),
            SchoolError::Maintenance("Official outage notice".into()),
        ),
        (
            SchoolExecutorError::Authentication("Sign-in expired.".into()),
            SchoolError::Authentication("Sign-in expired.".into()),
        ),
        (
            SchoolExecutorError::Storage("School database unavailable.".into()),
            SchoolError::Storage("School database unavailable.".into()),
        ),
        (
            SchoolExecutorError::Timeout("School page timed out.".into()),
            SchoolError::Timeout("School page timed out.".into()),
        ),
        (
            SchoolExecutorError::Stopped("The school browser has stopped.".into()),
            SchoolError::Stopped("The school browser has stopped.".into()),
        ),
        (
            SchoolExecutorError::SiteChanged("School page changed.".into()),
            SchoolError::SiteChanged("School page changed.".into()),
        ),
    ];
    for (executor_error, expected) in cases {
        let coordinator = PullCoordinator::default();
        let service = service(
            coordinator,
            Arc::new(SchoolAuthService::new(MemoryVault::default())),
        );
        let error = service
            .pull(
                Arc::new(FakeExecutor::new(Behavior::Typed(executor_error))),
                SchoolSite::Kyomu,
                None,
                PullOptions::manual(),
                false,
            )
            .await
            .unwrap_err();
        assert_eq!(error, expected);

        let rows = service
            .pull(
                Arc::new(FakeExecutor::new(Behavior::Outcomes(vec![outcome(
                    IngestState::Ok,
                )]))),
                SchoolSite::Kyomu,
                None,
                PullOptions::manual(),
                false,
            )
            .await
            .unwrap();
        assert_eq!(rows, vec![outcome(IngestState::Ok)]);
    }
}

#[tokio::test]
async fn executor_panic_keeps_its_typed_source_and_the_next_pull_runs() {
    let coordinator = PullCoordinator::default();
    let service = service(
        coordinator,
        Arc::new(SchoolAuthService::new(MemoryVault::default())),
    );
    let error = service
        .pull(
            Arc::new(FakeExecutor::new(Behavior::Panic)),
            SchoolSite::Kyomu,
            None,
            PullOptions::manual(),
            false,
        )
        .await
        .unwrap_err();
    assert_eq!(
        error,
        SchoolError::Coordination(PullError::ExecutorPanicked)
    );

    let rows = service
        .pull(
            Arc::new(FakeExecutor::new(Behavior::Outcomes(vec![outcome(
                IngestState::Ok,
            )]))),
            SchoolSite::Kyomu,
            None,
            PullOptions::manual(),
            false,
        )
        .await
        .unwrap();
    assert_eq!(rows, vec![outcome(IngestState::Ok)]);
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
