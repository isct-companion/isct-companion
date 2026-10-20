use super::*;
use crate::{
    ports::{CloudObject, NativeCloud, NativeDownloadSession, NativeVault},
    types::*,
};
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};
static NEXT: AtomicU64 = AtomicU64::new(1);
/// A school cookie store with nobody signed in; a browser needs one.
struct NoCookies;
#[async_trait::async_trait]
impl NativeDownloadSession for NoCookies {
    async fn cookie_header(&self, _: String) -> Result<String, SdkError> {
        Ok(String::new())
    }
    async fn store_set_cookies(&self, _: String, _: Vec<String>) -> Result<(), SdkError> {
        Ok(())
    }
}
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
        Self::with_cloud(None).await
    }
    async fn with_cloud(cloud: Option<Arc<dyn crate::ports::NativeCloud>>) -> Self {
        let providers = if cloud.is_some() {
            vec![CloudProvider::Icloud]
        } else {
            vec![]
        };
        let root = std::env::temp_dir().join(format!(
            "isct-sdk-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let session = Session::open(
            SessionConfig {
                data_directory: root.join("data").to_string_lossy().into(),
                files_directory: root.join("files").to_string_lossy().into(),
                initial_network: SyncNetwork::Offline,
                cloud_providers: providers.clone(),
                authorized_cloud_providers: providers,
            },
            Arc::new(Vault::default()),
            None,
            cloud,
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
    let events = f.session.subscribe().unwrap();
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
    let StreamUpdate::Event { event } = events.next().await.unwrap() else {
        panic!("missing mutation event")
    };
    assert_eq!(event.name, "workspace-changed");
    assert_eq!(event.payload.0["source"], "user-intent");
    assert_eq!(event.payload.0["revision"], json!(snapshot.revision));
    let changes: Vec<WorkspaceChange> =
        serde_json::from_value(event.payload.0["changes"].clone()).unwrap();
    assert!(
        event.payload.0["changes"]
            .as_array()
            .unwrap()
            .contains(&json!("settings")),
        "{}",
        event.payload.0
    );
    // The event's names read back only what they feed, from the same state.
    let OperationResult::WorkspaceRecords { patch } = f
        .session
        .workspace_records(changes)
        .unwrap()
        .await_result()
        .await
        .unwrap()
    else {
        panic!("wrong result")
    };
    assert_eq!(patch.content_revision, snapshot.content_revision);
    assert_eq!(patch.workspace.0["academicProfile"]["admissionYear"], 2026);
    assert!(matches!(
        f.session.mutate(mutation).unwrap().await_result().await,
        Err(SdkError::Conflict { .. })
    ));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), events.next())
            .await
            .is_err()
    );
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
async fn typed_study_profile_round_trips_and_rejects_invalid_terms_atomically() {
    let f = Fixture::new().await;
    let first = f.snapshot().await;
    let saved = f
        .session
        .mutate(WorkspaceMutation {
            expected_revision: first.revision,
            intent: WorkspaceIntent::SetStudyProfile {
                admission_year: Some(2026),
                study_enrollments: vec![StudyEnrollment {
                    stage: StudyStage::Master,
                    entry_kind: StudyEntryKind::Regular,
                    entry_grade: Some(1),
                    admitted_term: "2026-3Q".into(),
                }],
            },
        })
        .unwrap()
        .await_result()
        .await
        .unwrap();
    let OperationResult::Workspace { snapshot: saved } = saved else {
        panic!("wrong result")
    };
    assert_eq!(saved.workspace.0["academicProfile"]["admissionYear"], 2026);
    assert_eq!(
        saved.workspace.0["academicProfile"]["studyEnrollments"],
        json!([{
            "stage":"master",
            "entryKind":"regular",
            "entryGrade":1,
            "admittedTerm":"2026-3Q"
        }])
    );
    let committed_workspace = saved.workspace.0.clone();
    let committed_revision = saved.revision.clone();

    let invalid = f
        .session
        .mutate(WorkspaceMutation {
            expected_revision: saved.revision,
            intent: WorkspaceIntent::SetStudyProfile {
                admission_year: Some(2027),
                study_enrollments: vec![StudyEnrollment {
                    stage: StudyStage::Doctoral,
                    entry_kind: StudyEntryKind::Regular,
                    entry_grade: Some(1),
                    admitted_term: "2027-spring".into(),
                }],
            },
        })
        .unwrap()
        .await_result()
        .await;
    assert!(matches!(invalid, Err(SdkError::InvalidInput { .. })));
    let after = f.snapshot().await;
    assert_eq!(after.revision, committed_revision);
    assert_eq!(after.workspace.0, committed_workspace);
    f.finish().await;
}

#[tokio::test]
async fn graduate_teaching_calendar_crosses_ffi_and_invalid_tables_roll_back() {
    let f = Fixture::new().await;
    let first = f.snapshot().await;
    let OperationResult::Workspace { snapshot: saved } = f
        .session
        .mutate(WorkspaceMutation {
            expected_revision: first.revision,
            intent: WorkspaceIntent::SetStudyProfile {
                admission_year: Some(2026),
                study_enrollments: vec![StudyEnrollment {
                    stage: StudyStage::Professional,
                    entry_kind: StudyEntryKind::Regular,
                    entry_grade: Some(1),
                    admitted_term: "2026-3Q".into(),
                }],
            },
        })
        .unwrap()
        .await_result()
        .await
        .unwrap()
    else {
        panic!("wrong result")
    };
    let mut periods: Vec<TeachingPeriod> =
        crate::json::project(isct_core::schedule::default_teaching_periods()).unwrap();
    assert_eq!(periods[5].periods, vec![11, 12]);
    periods[5].start_time = "20:00".into();
    periods[5].end_time = "21:40".into();
    let mut profile = saved.workspace.0["academicProfile"].clone();
    profile["teachingPeriods"] = serde_json::to_value(&periods).unwrap();
    let OperationResult::Workspace { snapshot: saved } = f
        .session
        .mutate(WorkspaceMutation {
            expected_revision: saved.revision,
            intent: WorkspaceIntent::EditAcademicCalendar {
                academic_profile: JsonValue(profile.clone()),
                exceptions: vec![],
            },
        })
        .unwrap()
        .await_result()
        .await
        .unwrap()
    else {
        panic!("wrong result")
    };
    assert_eq!(saved.workspace.0["academicProfile"], profile);
    assert_eq!(profile["studyEnrollments"][0]["stage"], "professional");

    // Foreign vectors can have a shape Core's fixed pair cannot represent.
    profile["teachingPeriods"][5]["periods"] = json!([11, 12, 13]);
    let invalid = f
        .session
        .mutate(WorkspaceMutation {
            expected_revision: saved.revision.clone(),
            intent: WorkspaceIntent::EditAcademicCalendar {
                academic_profile: JsonValue(profile),
                exceptions: vec![],
            },
        })
        .unwrap()
        .await_result()
        .await;
    assert!(matches!(invalid, Err(SdkError::InvalidInput { .. })));
    let after = f.snapshot().await;
    assert_eq!(after.revision, saved.revision);
    assert_eq!(after.workspace.0, saved.workspace.0);
    let OperationResult::Schedule { snapshot: queried } = f
        .session
        .schedule_query(vec![WorkspaceQuery::TeachingPeriods])
        .unwrap()
        .await_result()
        .await
        .unwrap()
    else {
        panic!("wrong result")
    };
    let WorkspaceQueryResult::TeachingPeriods(resolved) = &queried.values[0] else {
        panic!("wrong query result")
    };
    assert_eq!(resolved[5].periods, vec![11, 12]);
    assert_eq!(resolved[5].start_time, "20:00");
    assert_eq!(resolved[5].end_time, "21:40");
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
    let core: isct_core::workspace::WorkspaceIntent = crate::json::project_input(ffi).unwrap();
    assert_eq!(serde_json::to_value(core).unwrap(), value);
    let bad = WorkspaceQuery::IntensiveDays {
        from: "2026-10-15".into(),
        to: "2026-10-16".into(),
        periods: vec![1, 2, 3],
    };
    assert!(matches!(
        crate::json::project_input::<_, isct_core::schedule::WorkspaceQuery>(bad),
        Err(SdkError::InvalidInput { .. })
    ));
    // The same mismatch on the way out is Core's fault, not the caller's.
    assert!(matches!(
        crate::json::project::<_, [u8; 2]>(vec![1]),
        Err(SdkError::Internal { .. })
    ));
}

/// The sync types are renamed for the shells (`SyncView`, `SyncNetwork`, …)
/// and carry timestamps as text; the JSON both sides agree on must not change.
#[test]
fn sync_snapshots_and_hints_cross_the_renamed_ffi_types() {
    use isct_core::sync_engine as core;
    let retry = chrono::DateTime::parse_from_rfc3339("2026-10-20T03:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let snapshot = core::SyncSnapshot {
        generation: 3,
        resetting: false,
        runs: vec![core::RunView {
            run: "run-1".into(),
            dataset: "lms.course".into(),
            label: core::Label {
                key: "sync.dataset.lms.course".into(),
                args: json!({"term": "2026-3Q"}),
            },
            priority: core::Priority::Interactive,
            done: 4,
            total: Some(9),
            current: None,
            scope: Some("123".into()),
            held: Some(core::Held::Backoff { until: retry }),
            failure: Some(core::SyncFailure {
                kind: "http-503".into(),
                keys_failed: 1,
                retry_at: Some(retry),
            }),
        }],
        sources: vec![],
    };
    let payload = serde_json::to_value(&snapshot).unwrap();
    let decoded = crate::decode_sync_snapshot(crate::JsonValue(payload.clone())).unwrap();
    assert!(matches!(decoded.runs[0].priority, SyncPriority::Interactive));
    assert!(matches!(&decoded.runs[0].held, Some(Held::Backoff { until }) if until.starts_with("2026-10-20T03:00:00")));
    assert_eq!(decoded.runs[0].label.args.0["term"], "2026-3Q");
    assert_eq!(serde_json::to_value(&decoded).unwrap(), payload);

    let hint = Hint::Open {
        view: SyncView::Course { lms_course_id: 123 },
    };
    let core_hint: core::Hint = crate::json::project_input(hint).unwrap();
    assert_eq!(
        core_hint,
        core::Hint::Open {
            view: core::View::Course { lms_course_id: 123 }
        }
    );
    let network: core::Hint = crate::json::project_input(Hint::Network {
        network: SyncNetwork::Cellular,
    })
    .unwrap();
    assert_eq!(
        network,
        core::Hint::Network {
            network: core::Network::Cellular
        }
    );
}

#[test]
fn school_outcomes_project_into_the_generated_ffi_types() {
    let outcome = isct_core::school_service::IngestOutcome {
        site: isct_core::lifecycle::SchoolSite::ScienceLms,
        kind: "courses".into(),
        state: isct_core::school_service::IngestState::ReconnectRequired,
        changed: 2,
        message: "partial result".into(),
    };
    let outcomes: Vec<IngestOutcome> = crate::json::project(vec![outcome]).unwrap();
    assert!(matches!(outcomes[0].state, IngestState::ReconnectRequired));
    assert_eq!(outcomes[0].changed, 2);
    let unavailable: IngestOutcome =
        crate::json::project(isct_core::school_service::IngestOutcome {
            site: isct_core::lifecycle::SchoolSite::Kyomu,
            kind: "gpa".into(),
            state: isct_core::school_service::IngestState::Unavailable,
            changed: 0,
            message: "This page is not offered to this account.".into(),
        })
        .unwrap();
    assert!(matches!(unavailable.state, IngestState::Unavailable));
    assert_eq!(unavailable.changed, 0);
}

#[tokio::test]
async fn backup_import_restores_through_the_journal_without_a_cloud_store() {
    let f = Fixture::new().await;
    let OperationResult::Text { value } = f
        .session
        .export_backup()
        .unwrap()
        .await_result()
        .await
        .unwrap()
    else {
        panic!("wrong backup result")
    };
    // Core publishes only writes that change rows, so the backup carries a
    // plan this device does not have yet.
    let mut document: serde_json::Value = serde_json::from_str(&value).unwrap();
    document["tables"]["plans"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "restored-plan",
            "rev": 1,
            "updatedAt": "2026-10-13T01:00:00Z",
            "updatedBy": "other-device",
            "source": {"provider": "manual"},
            "title": "Restored",
            "start": "2026-10-13T10:00:00+09:00",
            "duration": 60,
            "exdates": []
        }));
    let events = f.session.subscribe().unwrap();
    f.session
        .apply_backup(JsonValue(document))
        .unwrap()
        .await_result()
        .await
        .unwrap();
    // The journal publishes its own state around the restore; the import
    // itself is the workspace change.
    let event = loop {
        let StreamUpdate::Event { event } =
            tokio::time::timeout(std::time::Duration::from_secs(2), events.next())
                .await
                .expect("restore event within two seconds")
                .unwrap()
        else {
            panic!("missing restore event")
        };
        if event.name == "workspace-changed" {
            break event;
        }
    };
    assert_eq!(event.payload.0["source"], "cloud-restore");
    for restored in ["plans", "events"] {
        assert!(
            event.payload.0["changes"]
                .as_array()
                .unwrap()
                .contains(&json!(restored)),
            "{}",
            event.payload.0
        );
    }
    let snapshot = f.snapshot().await;
    assert!(snapshot.workspace.0["plans"]
        .as_array()
        .unwrap()
        .iter()
        .any(|plan| plan["id"] == "restored-plan"));
    f.finish().await;
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
            initial_network: SyncNetwork::Offline,
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
        initial_network: SyncNetwork::Offline,
        cloud_providers: vec![],
        authorized_cloud_providers: vec![],
    };
    let session = Session::open(
        config,
        Arc::new(Vault::default()),
        Some(browser.clone()),
        None,
        None,
        Some(Arc::new(NoCookies)),
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
#[cfg(feature = "ocr")]
#[tokio::test]
async fn ocr_progress_correlates_to_the_operation_and_bad_images_leave_core_usable() {
    let f = Fixture::new().await;
    let events = f.session.subscribe().unwrap();
    let operation = f.session.scan_matrix(vec![1, 2, 3]).unwrap();
    assert!(matches!(
        operation.await_result().await,
        Err(SdkError::InvalidInput { .. })
    ));
    let StreamUpdate::Event { event } = events.next().await.unwrap() else {
        panic!("missing progress")
    };
    assert_eq!(event.name, "ocr-progress");
    assert_eq!(event.payload.0["operationId"], operation.id());
    let _ = f.snapshot().await;
    f.finish().await;
}

/// One CloudKit-like record shared by two sessions: each accepted write gets
/// a new tag, and a write against another tag is a conflict.
#[derive(Default)]
struct SharedRecord {
    object: Mutex<Option<CloudObject>>,
    writes: AtomicU64,
    /// Another device replaces the record just before the next write lands.
    race_next_write: std::sync::atomic::AtomicBool,
}
#[async_trait::async_trait]
impl NativeCloud for SharedRecord {
    fn device_name(&self) -> Option<String> {
        Some("Test device".into())
    }
    fn disconnect(&self, _: CloudProvider) -> Result<(), SdkError> {
        Ok(())
    }
    async fn read(&self, _: CloudProvider) -> Result<Option<CloudObject>, SdkError> {
        Ok(self.object.lock().unwrap().clone())
    }
    async fn write(
        &self,
        _: CloudProvider,
        bytes: Vec<u8>,
        expected: Option<String>,
    ) -> Result<(), SdkError> {
        let attempt = self.writes.fetch_add(1, Ordering::SeqCst) + 1;
        let mut object = self.object.lock().unwrap();
        if self.race_next_write.swap(false, Ordering::SeqCst) {
            if let Some(object) = object.as_mut() {
                object.tag = format!("raced-{attempt}");
            }
        }
        if object.as_ref().map(|o| o.tag.clone()) != expected {
            return Err(SdkError::Conflict {
                detail: "record changed".into(),
            });
        }
        *object = Some(CloudObject {
            bytes,
            tag: format!("tag-{attempt}"),
        });
        Ok(())
    }
}

async fn cloud_op(operation: Arc<Operation>) -> SyncState {
    match operation.await_result().await.unwrap() {
        OperationResult::Cloud { state } => state,
        _ => panic!("wrong cloud result"),
    }
}

/// The desktop and phone stand-ins share one record through the byte port:
/// what one uploads the other pulls, and a record replaced mid-write is
/// re-read and merged instead of overwritten.
#[tokio::test]
async fn sessions_sharing_one_cloud_record_load_each_others_data() {
    let record = Arc::new(SharedRecord::default());
    let mac = Fixture::with_cloud(Some(record.clone())).await;
    let phone = Fixture::with_cloud(Some(record.clone())).await;

    let OperationResult::Text { value } = mac
        .session
        .export_backup()
        .unwrap()
        .await_result()
        .await
        .unwrap()
    else {
        panic!("wrong backup result")
    };
    let mut document: serde_json::Value = serde_json::from_str(&value).unwrap();
    document["tables"]["plans"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "mac-plan",
            "rev": 1,
            "updatedAt": "2026-10-13T01:00:00Z",
            "updatedBy": "mac",
            "source": {"provider": "manual"},
            "title": "From the Mac",
            "start": "2026-10-13T10:00:00+09:00",
            "duration": 60,
            "exdates": []
        }));
    mac.session
        .apply_backup(JsonValue(document))
        .unwrap()
        .await_result()
        .await
        .unwrap();
    for session in [&mac.session, &phone.session] {
        cloud_op(
            session
                .set_cloud_enabled(CloudProvider::Icloud, true)
                .unwrap(),
        )
        .await;
    }
    cloud_op(mac.session.sync_cloud(Direction::Upload).unwrap()).await;
    let state = cloud_op(phone.session.sync_cloud(Direction::Pull).unwrap()).await;
    assert!(
        state.providers.iter().all(|p| p.error.is_none()),
        "{state:?}"
    );
    assert!(phone.snapshot().await.workspace.0["plans"]
        .as_array()
        .unwrap()
        .iter()
        .any(|plan| plan["id"] == "mac-plan"));

    let before = record.writes.load(Ordering::SeqCst);
    record.race_next_write.store(true, Ordering::SeqCst);
    let mut document: serde_json::Value = {
        let OperationResult::Text { value } = phone
            .session
            .export_backup()
            .unwrap()
            .await_result()
            .await
            .unwrap()
        else {
            panic!("wrong backup result")
        };
        serde_json::from_str(&value).unwrap()
    };
    document["tables"]["plans"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "phone-plan",
            "rev": 1,
            "updatedAt": "2026-10-13T02:00:00Z",
            "updatedBy": "phone",
            "source": {"provider": "manual"},
            "title": "From the phone",
            "start": "2026-10-14T10:00:00+09:00",
            "duration": 60,
            "exdates": []
        }));
    phone
        .session
        .apply_backup(JsonValue(document))
        .unwrap()
        .await_result()
        .await
        .unwrap();
    let state = cloud_op(phone.session.sync_cloud(Direction::Upload).unwrap()).await;
    assert!(
        state.providers.iter().all(|p| p.error.is_none()),
        "{state:?}"
    );
    // The raced write was refused as a conflict and retried after a re-read.
    assert!(!record.race_next_write.load(Ordering::SeqCst));
    assert!(record.writes.load(Ordering::SeqCst) >= before + 2);
    cloud_op(mac.session.sync_cloud(Direction::Pull).unwrap()).await;
    let plans = mac.snapshot().await.workspace.0["plans"].clone();
    assert!(plans
        .as_array()
        .unwrap()
        .iter()
        .any(|plan| plan["id"] == "phone-plan"));

    mac.finish().await;
    phone.finish().await;
}
