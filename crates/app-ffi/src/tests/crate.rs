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
            None,
            None,
        )
        .await
        .unwrap();
        Self { root, session }
    }
    /// The revision the next intent is made against.
    async fn revision(&self) -> String {
        match self
            .session
            .workspace_revision()
            .unwrap()
            .await_result()
            .await
            .unwrap()
        {
            OperationResult::Text { value } => value,
            _ => panic!("wrong result"),
        }
    }
    /// The workspace fields `fields` names, as a screen's `records` query
    /// reads them.
    async fn records(&self, fields: &[&str]) -> serde_json::Value {
        let OperationResult::Schedule { snapshot } = self
            .session
            .schedule_query(vec![WorkspaceQuery::Records {
                fields: fields.iter().map(|field| (*field).into()).collect(),
            }])
            .unwrap()
            .await_result()
            .await
            .unwrap()
        else {
            panic!("wrong result")
        };
        match snapshot.values.into_iter().next() {
            Some(WorkspaceQueryResult::Records(records)) => records.0,
            _ => panic!("wrong query result"),
        }
    }
    async fn commit(&self, mutation: WorkspaceMutation) -> Result<WorkspaceCommit, SdkError> {
        match self.session.mutate_commit(mutation)?.await_result().await? {
            OperationResult::Committed { commit } => Ok(commit),
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
    let first = f.revision().await;
    let mutation = WorkspaceMutation {
        expected_revision: first.clone(),
        intent: WorkspaceIntent::SetAdmissionYear {
            admission_year: Some(2026),
        },
    };
    let committed = f.commit(mutation.clone()).await.unwrap();
    assert_ne!(first, committed.revision);
    assert_eq!(
        f.records(&[]).await["academicProfile"]["admissionYear"],
        2026
    );
    // Sync snapshots stream on the same channel; the mutation's own event is
    // the workspace change.
    let event = loop {
        let StreamUpdate::Event { event } = events.next().await.unwrap() else {
            panic!("missing mutation event")
        };
        if event.name == "workspace-changed" {
            break event;
        }
    };
    assert_eq!(event.payload.0["source"], "user-intent");
    assert_eq!(event.payload.0["revision"], json!(committed.revision));
    assert!(
        event.payload.0["changes"]
            .as_array()
            .unwrap()
            .contains(&json!("settings")),
        "{}",
        event.payload.0
    );
    // The commit answers the changes its event names.
    assert_eq!(
        serde_json::to_value(&committed.changes).unwrap(),
        event.payload.0["changes"]
    );
    assert!(matches!(
        f.commit(mutation).await,
        Err(SdkError::Conflict { .. })
    ));
    // Independent sync progress may arrive after the mutation's event; a
    // rejected write must emit no additional workspace change.
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), async {
            loop {
                let StreamUpdate::Event { event } = events.next().await.unwrap() else {
                    panic!("event stream ended")
                };
                assert_ne!(event.name, "workspace-changed");
            }
        })
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
    assert_eq!(result.values.len(), 2);
    // A shell that keeps no workspace reads the revision alone and hears a
    // commit as its changes, next revision and sequence.
    let OperationResult::Text { value: current } = f
        .session
        .workspace_revision()
        .unwrap()
        .await_result()
        .await
        .unwrap()
    else {
        panic!("wrong result")
    };
    assert_eq!(current, committed.revision);
    let OperationResult::Committed { commit } = f
        .session
        .mutate_commit(WorkspaceMutation {
            expected_revision: current,
            intent: WorkspaceIntent::SetAdmissionYear {
                admission_year: Some(2027),
            },
        })
        .unwrap()
        .await_result()
        .await
        .unwrap()
    else {
        panic!("wrong result")
    };
    assert!(commit
        .changes
        .iter()
        .any(|change| matches!(change, WorkspaceChange::Settings)));
    assert!(commit.seq >= event.payload.0["seq"].as_u64().unwrap());
    let OperationResult::Text { value: next } = f
        .session
        .workspace_revision()
        .unwrap()
        .await_result()
        .await
        .unwrap()
    else {
        panic!("wrong result")
    };
    assert_eq!(commit.revision, next);
    f.finish().await;
}

#[tokio::test]
async fn typed_study_profile_round_trips_and_rejects_invalid_terms_atomically() {
    let f = Fixture::new().await;
    let first = f.revision().await;
    let saved = f
        .commit(WorkspaceMutation {
            expected_revision: first,
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
        .await
        .unwrap();
    let committed_records = f.records(&[]).await;
    assert_eq!(committed_records["academicProfile"]["admissionYear"], 2026);
    assert_eq!(
        committed_records["academicProfile"]["studyEnrollments"],
        json!([{
            "stage":"master",
            "entryKind":"regular",
            "entryGrade":1,
            "admittedTerm":"2026-3Q"
        }])
    );
    let invalid = f
        .commit(WorkspaceMutation {
            expected_revision: saved.revision.clone(),
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
        .await;
    assert!(matches!(invalid, Err(SdkError::InvalidInput { .. })));
    assert_eq!(f.revision().await, saved.revision);
    assert_eq!(f.records(&[]).await, committed_records);
    f.finish().await;
}

#[tokio::test]
async fn graduate_teaching_calendar_crosses_ffi_and_invalid_tables_roll_back() {
    let f = Fixture::new().await;
    let first = f.revision().await;
    let saved = f
        .commit(WorkspaceMutation {
            expected_revision: first,
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
        .await
        .unwrap();
    let mut periods: Vec<TeachingPeriod> =
        crate::json::project(app_core::schedule::default_teaching_periods()).unwrap();
    assert_eq!(periods[5].periods, vec![11, 12]);
    periods[5].start_time = "20:00".into();
    periods[5].end_time = "21:40".into();
    let mut profile = f.records(&[]).await["academicProfile"].clone();
    profile["teachingPeriods"] = serde_json::to_value(&periods).unwrap();
    let saved = f
        .commit(WorkspaceMutation {
            expected_revision: saved.revision,
            intent: WorkspaceIntent::EditAcademicCalendar {
                academic_profile: JsonValue(profile.clone()),
                exceptions: vec![],
            },
        })
        .await
        .unwrap();
    let saved_records = f.records(&["exceptions"]).await;
    assert_eq!(saved_records["academicProfile"], profile);
    assert_eq!(profile["studyEnrollments"][0]["stage"], "professional");

    // Foreign vectors can have a shape Core's fixed pair cannot represent.
    profile["teachingPeriods"][5]["periods"] = json!([11, 12, 13]);
    let invalid = f
        .commit(WorkspaceMutation {
            expected_revision: saved.revision.clone(),
            intent: WorkspaceIntent::EditAcademicCalendar {
                academic_profile: JsonValue(profile),
                exceptions: vec![],
            },
        })
        .await;
    assert!(matches!(invalid, Err(SdkError::InvalidInput { .. })));
    assert_eq!(f.revision().await, saved.revision);
    assert_eq!(f.records(&["exceptions"]).await, saved_records);
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
    let first = f.revision().await;
    let gate = Arc::new(tokio::sync::Notify::new());
    let release = gate.clone();
    let revision = first;
    let op = f
        .session
        .start(false, move |i, _| async move {
            release.notified().await;
            let commit =
                i.core
                    .workspace
                    .mutate_commit(app_core::workspace::WorkspaceMutation {
                        expected_revision: revision,
                        intent: app_core::workspace::WorkspaceIntent::SetAdmissionYear {
                            admission_year: Some(2025),
                        },
                    })?;
            Ok(OperationResult::Committed {
                commit: crate::json::project(commit)?,
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
        f.records(&[]).await["academicProfile"]["admissionYear"],
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
    assert!(!f.revision().await.is_empty());
    stream.unsubscribe().unwrap();
    assert!(matches!(stream.next().await, Err(SdkError::Closed)));
    f.session.shutdown().await.unwrap();
    f.session.shutdown().await.unwrap();
    assert!(matches!(
        f.session.workspace_revision(),
        Err(SdkError::Closed)
    ));
    assert!(matches!(f.session.subscribe(), Err(SdkError::Closed)));
    f.finish().await;
}
#[tokio::test]
async fn shutdown_waits_for_a_started_commit_and_settles_cooperative_work() {
    let f = Fixture::new().await;
    let gate = Arc::new(tokio::sync::Notify::new());
    let release = gate.clone();
    let revision = f.revision().await;
    let write = f
        .session
        .start(false, move |i, _| async move {
            release.notified().await;
            let commit =
                i.core
                    .workspace
                    .mutate_commit(app_core::workspace::WorkspaceMutation {
                        expected_revision: revision,
                        intent: app_core::workspace::WorkspaceIntent::SetAdmissionYear {
                            admission_year: Some(2024),
                        },
                    })?;
            Ok(OperationResult::Committed {
                commit: crate::json::project(commit)?,
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
        OperationResult::Committed { .. }
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
    let _ = f.revision().await;
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
    let core: app_core::workspace::WorkspaceIntent = crate::json::project_input(ffi).unwrap();
    assert_eq!(serde_json::to_value(core).unwrap(), value);
    let bad = WorkspaceQuery::IntensiveDays {
        from: "2026-10-15".into(),
        to: "2026-10-16".into(),
        periods: vec![1, 2, 3],
    };
    assert!(matches!(
        crate::json::project_input::<_, app_core::schedule::WorkspaceQuery>(bad),
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
    use app_core::sync_engine as core;
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
    assert!(matches!(
        decoded.runs[0].priority,
        SyncPriority::Interactive
    ));
    assert!(
        matches!(&decoded.runs[0].held, Some(Held::Backoff { until }) if until.starts_with("2026-10-20T03:00:00"))
    );
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
    let outcome = app_core::school_service::IngestOutcome {
        site: app_core::lifecycle::SchoolSite::ScienceLms,
        kind: "courses".into(),
        state: app_core::school_service::IngestState::ReconnectRequired,
        changed: 2,
        message: "partial result".into(),
    };
    let outcomes: Vec<IngestOutcome> = crate::json::project(vec![outcome]).unwrap();
    assert!(matches!(outcomes[0].state, IngestState::ReconnectRequired));
    assert_eq!(outcomes[0].changed, 2);
    let unavailable: IngestOutcome =
        crate::json::project(app_core::school_service::IngestOutcome {
            site: app_core::lifecycle::SchoolSite::Kyomu,
            kind: "gpa".into(),
            state: app_core::school_service::IngestState::Unavailable,
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
    // weekly event this device does not have yet.
    let mut document: serde_json::Value = serde_json::from_str(&value).unwrap();
    document["tables"]["personal_events"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "restored-event",
            "rev": 1,
            "updatedAt": "2026-10-13T01:00:00Z",
            "updatedBy": "other-device",
            "source": {"provider": "manual"},
            "title": "Restored",
            "kind": "event",
            "termId": "2026-3Q",
            "room": "",
            "days": [1],
            "startTime": "10:00",
            "endTime": "11:00"
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
    assert!(
        event.payload.0["changes"]
            .as_array()
            .unwrap()
            .contains(&json!("events")),
        "{}",
        event.payload.0
    );
    assert!(f.records(&["events"]).await["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["id"] == "restored-event"));
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
        None,
        None,
    )
    .await
    .unwrap();
    drop(vault);
    let result = session
        .connect_school(
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
    let revision = f.revision().await;
    let write = f
        .session
        .start(false, move |i, _| async move {
            release.notified().await;
            let commit =
                i.core
                    .workspace
                    .mutate_commit(app_core::workspace::WorkspaceMutation {
                        expected_revision: revision,
                        intent: app_core::workspace::WorkspaceIntent::SetAdmissionYear {
                            admission_year: Some(2023),
                        },
                    })?;
            Ok(OperationResult::Committed {
                commit: crate::json::project(commit)?,
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
        OperationResult::Committed { .. }
    ));
    f.finish().await;
}
#[derive(Default)]
struct BrowserFixture(Mutex<Option<Arc<crate::ports::BrowserEvents>>>);
impl crate::ports::NativeBrowser for BrowserFixture {
    fn capabilities(&self) -> crate::ports::HostCapabilities {
        crate::ports::HostCapabilities {
            form_resubmission: true,
            inline_popups: true,
        }
    }
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
    let _ = f.revision().await;
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
    document["tables"]["personal_events"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "mac-event",
            "rev": 1,
            "updatedAt": "2026-10-13T01:00:00Z",
            "updatedBy": "mac",
            "source": {"provider": "manual"},
            "title": "From the Mac",
            "kind": "event",
            "termId": "2026-3Q",
            "room": "",
            "days": [1],
            "startTime": "10:00",
            "endTime": "11:00"
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
    assert!(phone.records(&["events"]).await["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["id"] == "mac-event"));

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
    document["tables"]["personal_events"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "phone-event",
            "rev": 1,
            "updatedAt": "2026-10-13T02:00:00Z",
            "updatedBy": "phone",
            "source": {"provider": "manual"},
            "title": "From the phone",
            "kind": "event",
            "termId": "2026-3Q",
            "room": "",
            "days": [1],
            "startTime": "10:00",
            "endTime": "11:00"
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
    let events = mac.records(&["events"]).await["events"].clone();
    assert!(events
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["id"] == "phone-event"));

    mac.finish().await;
    phone.finish().await;
}

#[derive(Default)]
struct AccountTokens {
    decline: std::sync::atomic::AtomicBool,
    prompts: AtomicU64,
    signed_out: std::sync::atomic::AtomicBool,
}
#[async_trait::async_trait]
impl crate::ports::NativeDriveTokens for AccountTokens {
    async fn access_token(&self, scope: String, interactive: bool) -> Result<String, SdkError> {
        assert_eq!(scope, "https://www.googleapis.com/auth/drive.appdata");
        if interactive {
            self.prompts.fetch_add(1, Ordering::SeqCst);
        }
        if self.decline.load(Ordering::SeqCst) {
            return Err(SdkError::Authentication {
                detail: "consent declined".into(),
            });
        }
        Ok("token".into())
    }
    fn sign_out(&self) -> Result<(), SdkError> {
        self.signed_out.store(true, Ordering::SeqCst);
        Ok(())
    }
}

/// Android has no iCloud store, only Drive through the OS account system.
/// Its session still has cloud sync; a declined sign-in leaves Drive off with
/// the reason, an accepted one turns it on and is remembered across a
/// restart without asking the OS again, and turning Drive off signs out.
#[tokio::test]
async fn drive_signs_in_through_the_token_port_and_stays_authorized_after_restart() {
    let root = std::env::temp_dir().join(format!(
        "isct-sdk-drive-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let vault = Arc::new(Vault::default());
    let tokens = Arc::new(AccountTokens::default());
    let open = || {
        Session::open(
            SessionConfig {
                data_directory: root.join("data").to_string_lossy().into(),
                files_directory: root.join("files").to_string_lossy().into(),
                initial_network: SyncNetwork::Offline,
                cloud_providers: vec![CloudProvider::GoogleDrive, CloudProvider::Icloud],
                authorized_cloud_providers: vec![],
            },
            vault.clone(),
            None,
            None,
            None,
            None,
            None,
            Some(tokens.clone()),
        )
    };
    let drive = |state: &SyncState| {
        let rows = state
            .providers
            .iter()
            .filter(|row| matches!(row.provider, CloudProvider::GoogleDrive))
            .collect::<Vec<_>>();
        assert_eq!(state.providers.len(), 1, "iCloud has no store here");
        (rows[0].enabled, rows[0].error.clone())
    };
    let session = open().await.unwrap();
    assert!(session.capabilities().unwrap().cloud);
    assert_eq!(
        drive(&cloud_op(session.cloud_state().unwrap()).await),
        (false, None)
    );

    tokens.decline.store(true, Ordering::SeqCst);
    let declined = cloud_op(
        session
            .set_cloud_enabled(CloudProvider::GoogleDrive, true)
            .unwrap(),
    )
    .await;
    let (enabled, reason) = drive(&declined);
    assert!(!enabled);
    assert!(reason.unwrap().contains("consent declined"));

    tokens.decline.store(false, Ordering::SeqCst);
    let state = cloud_op(
        session
            .set_cloud_enabled(CloudProvider::GoogleDrive, true)
            .unwrap(),
    )
    .await;
    assert_eq!(drive(&state), (true, None));
    assert_eq!(tokens.prompts.load(Ordering::SeqCst), 2);
    session.shutdown().await.unwrap();
    drop(session);

    let session = open().await.unwrap();
    let state = cloud_op(
        session
            .set_cloud_enabled(CloudProvider::GoogleDrive, true)
            .unwrap(),
    )
    .await;
    assert_eq!(drive(&state), (true, None));
    assert_eq!(
        tokens.prompts.load(Ordering::SeqCst),
        2,
        "a remembered sign-in does not prompt again"
    );
    let state = cloud_op(
        session
            .set_cloud_enabled(CloudProvider::GoogleDrive, false)
            .unwrap(),
    )
    .await;
    assert_eq!(drive(&state), (false, None));
    assert!(tokens.signed_out.load(Ordering::SeqCst));
    assert_eq!(vault.load("google-drive".into()).unwrap(), None);
    session.shutdown().await.unwrap();
    drop(session);
    std::fs::remove_dir_all(root).unwrap();
}
