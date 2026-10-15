use super::*;
use crate::{ports::NativeVault, types::*};
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};
static NEXT: AtomicU64 = AtomicU64::new(1);
#[derive(Default)]
struct Vault(Mutex<HashMap<String, String>>);
impl NativeVault for Vault {
    fn load(&self, a: String) -> Result<Option<String>, SdkError> {
        Ok(self.0.lock().unwrap().get(&a).cloned())
    }
    fn store(&self, a: String, s: String) -> Result<(), SdkError> {
        self.0.lock().unwrap().insert(a, s);
        Ok(())
    }
    fn delete(&self, a: String) -> Result<(), SdkError> {
        self.0.lock().unwrap().remove(&a);
        Ok(())
    }
}
struct Fixture {
    root: std::path::PathBuf,
    session: Arc<Session>,
}
impl Fixture {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "isct-sdk-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let session = Session::open(
            SessionConfig {
                data_directory: root.join("data").to_string_lossy().into(),
                files_directory: root.join("files").to_string_lossy().into(),
                initial_network: PullNetwork::Offline,
                cloud_providers: vec![],
                authorized_cloud_providers: vec![],
            },
            Arc::new(Vault::default()),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        Self { root, session }
    }
    async fn snapshot(&self) -> WorkspaceSnapshot {
        match self
            .session
            .snapshot()
            .unwrap()
            .await_result()
            .await
            .unwrap()
        {
            OperationResult::Workspace { snapshot } => snapshot,
            _ => panic!("wrong result"),
        }
    }
    async fn finish(self) {
        self.session.shutdown().await.unwrap();
        drop(self.session);
        std::fs::remove_dir_all(self.root).unwrap()
    }
}
#[tokio::test]
async fn revision_conflict_and_core_queries_cross_the_typed_boundary() {
    let f = Fixture::new().await;
    let first = f.snapshot().await;
    let mutation = WorkspaceMutation {
        expected_revision: first.revision.clone(),
        intent: WorkspaceIntent::SetAdmissionYear {
            admission_year: Some(2026),
        },
    };
    let result = f
        .session
        .mutate(mutation.clone())
        .unwrap()
        .await_result()
        .await
        .unwrap();
    let OperationResult::Workspace { snapshot } = result else {
        panic!("wrong result")
    };
    assert_ne!(first.revision, snapshot.revision);
    assert_eq!(
        snapshot.workspace.0["academicProfile"]["admissionYear"],
        2026
    );
    assert!(matches!(
        f.session.mutate(mutation).unwrap().await_result().await,
        Err(SdkError::Conflict { .. })
    ));
    let result = f
        .session
        .schedule_query(vec![
            WorkspaceQuery::Glance {
                now: "2026-10-15T09:00:00+09:00".into(),
            },
            WorkspaceQuery::NotificationPlan {
                now: "2026-10-15T09:00:00+09:00".into(),
                horizon: "2026-10-16T09:00:00+09:00".into(),
                locale: None,
            },
        ])
        .unwrap()
        .await_result()
        .await
        .unwrap();
    let OperationResult::Schedule { snapshot: result } = result else {
        panic!("wrong result")
    };
    assert_eq!(result.revision, snapshot.content_revision);
    assert_eq!(result.values.len(), 2);
    f.finish().await;
}
#[tokio::test]
async fn dropping_foreign_wait_does_not_abort_an_admitted_transaction() {
    let f = Fixture::new().await;
    let first = f.snapshot().await;
    let gate = Arc::new(tokio::sync::Notify::new());
    let release = gate.clone();
    let revision = first.revision;
    let op = f
        .session
        .start(false, move |i, _| async move {
            release.notified().await;
            let snapshot = i
                .core
                .workspace
                .mutate(isct_core::workspace::WorkspaceMutation {
                    expected_revision: revision,
                    intent: isct_core::workspace::WorkspaceIntent::SetAdmissionYear {
                        admission_year: Some(2025),
                    },
                })?;
            Ok(OperationResult::Workspace {
                snapshot: crate::json::project(snapshot)?,
            })
        })
        .unwrap();
    assert!(!op.cancel().unwrap());
    op.detach().unwrap();
    assert!(matches!(op.await_result().await, Err(SdkError::Cancelled)));
    gate.notify_one();
    let mut rx = op.receiver.clone();
    while rx.borrow_and_update().is_none() {
        rx.changed().await.unwrap();
    }
    assert!(matches!(
        op.status().unwrap(),
        OperationStatus::Completed { .. }
    ));
    assert_eq!(
        f.snapshot().await.workspace.0["academicProfile"]["admissionYear"],
        2025
    );
    f.finish().await;
}
#[tokio::test]
async fn stream_lag_disposal_and_shutdown_are_recoverable() {
    let f = Fixture::new().await;
    let stream = f.session.subscribe().unwrap();
    for n in 0..260 {
        f.session
            .inner
            .events
            .publish("fixture", json!({"n":n}))
            .unwrap();
    }
    assert!(matches!(
        stream.next().await.unwrap(),
        StreamUpdate::SnapshotRequired { .. }
    ));
    let first = f.snapshot().await;
    assert!(!first.content_revision.is_empty());
    stream.unsubscribe().unwrap();
    assert!(matches!(stream.next().await, Err(SdkError::Closed)));
    f.session.shutdown().await.unwrap();
    f.session.shutdown().await.unwrap();
    assert!(matches!(f.session.snapshot(), Err(SdkError::Closed)));
    assert!(matches!(f.session.subscribe(), Err(SdkError::Closed)));
    f.finish().await;
}
#[tokio::test]
async fn shutdown_waits_for_a_started_commit_and_settles_cooperative_work() {
    let f = Fixture::new().await;
    let gate = Arc::new(tokio::sync::Notify::new());
    let release = gate.clone();
    let revision = f.snapshot().await.revision;
    let write = f
        .session
        .start(false, move |i, _| async move {
            release.notified().await;
            let s = i
                .core
                .workspace
                .mutate(isct_core::workspace::WorkspaceMutation {
                    expected_revision: revision,
                    intent: isct_core::workspace::WorkspaceIntent::SetAdmissionYear {
                        admission_year: Some(2024),
                    },
                })?;
            Ok(OperationResult::Workspace {
                snapshot: crate::json::project(s)?,
            })
        })
        .unwrap();
    let cancellable = f
        .session
        .start(true, |_, cancel| async move {
            cancel.cancelled().await;
            Err(SdkError::Cancelled)
        })
        .unwrap();
    let session = f.session.clone();
    let close = tokio::spawn(async move { session.shutdown().await });
    tokio::task::yield_now().await;
    gate.notify_one();
    close.await.unwrap().unwrap();
    assert!(matches!(
        write.await_result().await.unwrap(),
        OperationResult::Workspace { .. }
    ));
    assert!(matches!(
        cancellable.await_result().await,
        Err(SdkError::Cancelled)
    ));
    f.finish().await;
}
#[tokio::test]
async fn callback_failure_and_worker_panic_settle_without_killing_the_process() {
    let f = Fixture::new().await;
    let bad = f
        .session
        .start(false, |_, _| async move {
            panic!("test-only owned worker panic")
        })
        .unwrap();
    assert!(matches!(
        bad.await_result().await,
        Err(SdkError::Internal { .. })
    ));
    let _ = f.snapshot().await;
    assert!(matches!(
        f.session.school_session().unwrap().await_result().await,
        Err(SdkError::Unavailable { .. })
    ));
    f.finish().await;
}
#[tokio::test]
async fn notes_parser_backup_and_disabled_ocr_share_the_real_core() {
    let f = Fixture::new().await;
    let context = CourseFolder {
        term: "2026-3Q".into(),
        course: "sample".into(),
        section: "lesson".into(),
    };
    let saved = f
        .session
        .save_note(context.clone(), String::new(), "SDK fixture".into())
        .unwrap()
        .await_result()
        .await
        .unwrap();
    let OperationResult::File { file } = saved else {
        panic!("wrong result")
    };
    let read = f
        .session
        .read_note(context, file.id)
        .unwrap()
        .await_result()
        .await
        .unwrap();
    let OperationResult::File { file } = read else {
        panic!("wrong result")
    };
    assert_eq!(file.text.as_deref(), Some("SDK fixture"));
    assert!(matches!(
        f.session
            .export_backup()
            .unwrap()
            .await_result()
            .await
            .unwrap(),
        OperationResult::Text { .. }
    ));
    assert!(matches!(
        f.session
            .parse_lms_resource(
                "<html></html>".into(),
                "https://lms.s.isct.ac.jp/2025/mod/resource/view.php?id=1".into(),
                "resource".into()
            )
            .unwrap()
            .await_result()
            .await
            .unwrap(),
        OperationResult::Document { .. }
    ));
    #[cfg(not(feature = "ocr"))]
    assert!(matches!(
        f.session
            .scan_matrix(vec![1, 2, 3])
            .unwrap()
            .await_result()
            .await,
        Err(SdkError::Unavailable { .. })
    ));
    f.finish().await;
}
#[test]
fn generated_contract_preserves_intents_fixed_arrays_and_unknown_payloads() {
    let value = json!({"kind":"savePlan","draft":{"title":"future field","unknown":{"keep":true}},"existingId":null});
    let ffi: WorkspaceIntent = serde_json::from_value(value.clone()).unwrap();
    let core: isct_core::workspace::WorkspaceIntent = crate::json::project(ffi).unwrap();
    assert_eq!(serde_json::to_value(core).unwrap(), value);
    let bad = WorkspaceQuery::IntensiveDays {
        from: "2026-10-15".into(),
        to: "2026-10-16".into(),
        periods: vec![1, 2, 3],
    };
    assert!(crate::json::project::<_, isct_core::schedule::WorkspaceQuery>(bad).is_err());
}
struct BrokenVault;
impl NativeVault for BrokenVault {
    fn load(&self, _: String) -> Result<Option<String>, SdkError> {
        Err(SdkError::Callback {
            detail: "test native callback".into(),
        })
    }
    fn store(&self, _: String, _: String) -> Result<(), SdkError> {
        Err(SdkError::Callback {
            detail: "test native callback".into(),
        })
    }
    fn delete(&self, _: String) -> Result<(), SdkError> {
        Err(SdkError::Callback {
            detail: "test native callback".into(),
        })
    }
}
#[tokio::test]
async fn callback_errors_keep_their_category_and_last_session_drop_releases_ports() {
    let root = std::env::temp_dir().join(format!(
        "isct-vault-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let vault = Arc::new(BrokenVault);
    let weak = Arc::downgrade(&vault);
    let session = Session::open(
        SessionConfig {
            data_directory: root.join("data").to_string_lossy().into(),
            files_directory: root.join("files").to_string_lossy().into(),
            initial_network: PullNetwork::Offline,
            cloud_providers: vec![],
            authorized_cloud_providers: vec![],
        },
        vault.clone(),
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    drop(vault);
    let result = session
        .save_credentials(
            SchoolAuthProfile::TokyoTech,
            SchoolCredentials {
                username: "fixture".into(),
                password: "not-a-real-password".into(),
                otp_secret: String::new(),
                matrix: "A".repeat(70),
            },
        )
        .unwrap()
        .await_result()
        .await;
    assert!(
        matches!(result, Err(SdkError::Callback { .. })),
        "{result:?}"
    );
    drop(session);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while weak.strong_count() > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await
        }
    })
    .await
    .unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn shutdown_timeout_keeps_ownership_of_the_pending_commit() {
    let f = Fixture::new().await;
    let gate = Arc::new(tokio::sync::Notify::new());
    let release = gate.clone();
    let revision = f.snapshot().await.revision;
    let write = f
        .session
        .start(false, move |i, _| async move {
            release.notified().await;
            let snapshot = i
                .core
                .workspace
                .mutate(isct_core::workspace::WorkspaceMutation {
                    expected_revision: revision,
                    intent: isct_core::workspace::WorkspaceIntent::SetAdmissionYear {
                        admission_year: Some(2023),
                    },
                })?;
            Ok(OperationResult::Workspace {
                snapshot: crate::json::project(snapshot)?,
            })
        })
        .unwrap();
    assert!(matches!(
        f.session
            .shutdown_with_timeout(std::time::Duration::from_millis(5))
            .await,
        Err(SdkError::Timeout { .. })
    ));
    gate.notify_one();
    f.session.shutdown().await.unwrap();
    assert!(matches!(
        write.await_result().await.unwrap(),
        OperationResult::Workspace { .. }
    ));
    f.finish().await;
}
#[derive(Default)]
struct BrowserFixture(Mutex<Option<Arc<crate::ports::BrowserEvents>>>);
impl crate::ports::NativeBrowser for BrowserFixture {
    fn attach(&self, e: Arc<crate::ports::BrowserEvents>) -> Result<(), SdkError> {
        *self.0.lock().unwrap() = Some(e);
        Ok(())
    }
    fn ensure(&self, _: BrowserId, _: String) -> Result<(), SdkError> {
        Ok(())
    }
    fn dispatch(&self, _: crate::ports::BrowserCommand) -> Result<(), SdkError> {
        Ok(())
    }
    fn current_url(&self, _: BrowserId) -> Result<Option<String>, SdkError> {
        Ok(None)
    }
    fn dispose(&self, _: BrowserId) -> Result<(), SdkError> {
        Ok(())
    }
    fn shutdown(&self) -> Result<(), SdkError> {
        Err(SdkError::Callback {
            detail: "test shutdown failure".into(),
        })
    }
}
#[tokio::test]
async fn shutdown_reports_native_failure_and_rejects_retained_browser_callbacks() {
    let f = Fixture::new().await;
    let browser = Arc::new(BrowserFixture::default());
    let config = SessionConfig {
        data_directory: f.root.join("browser-data").to_string_lossy().into(),
        files_directory: f.root.join("browser-files").to_string_lossy().into(),
        initial_network: PullNetwork::Offline,
        cloud_providers: vec![],
        authorized_cloud_providers: vec![],
    };
    let session = Session::open(
        config,
        Arc::new(Vault::default()),
        Some(browser.clone()),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let events = browser.0.lock().unwrap().clone().unwrap();
    assert!(matches!(
        session.shutdown().await,
        Err(SdkError::Callback { .. })
    ));
    assert!(matches!(
        session.shutdown().await,
        Err(SdkError::Callback { .. })
    ));
    let page = PageId {
        browser: BrowserId {
            site: SchoolSite::ScienceLms,
            generation: 1,
        },
        sequence: 1,
    };
    assert!(matches!(
        events.title_changed(page, "stale page".into()),
        Err(SdkError::Closed)
    ));
    drop(session);
    drop(events);
    drop(browser);
    f.finish().await;
}
