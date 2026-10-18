use super::*;
use crate::ports::{EventError, EventSink};
use std::sync::atomic::{AtomicUsize, Ordering};

struct MutationEvents {
    count: AtomicUsize,
    fail: bool,
}

impl EventSink for MutationEvents {
    fn emit(&self, event: crate::events::CoreEvent) -> Result<(), EventError> {
        assert!(matches!(
            event,
            crate::events::CoreEvent::WorkspaceChanged {
                source: crate::events::WorkspaceChangeSource::UserIntent,
                revision: Some(_),
            }
        ));
        self.count.fetch_add(1, Ordering::Relaxed);
        if self.fail {
            Err(EventError::Delivery("synthetic delivery failure".into()))
        } else {
            Ok(())
        }
    }
}

fn service(name: &str) -> WorkspaceService {
    let path = std::env::temp_dir().join(format!("isct-workspace-{name}-{}.db", Uuid::new_v4()));
    WorkspaceService::new(DataStore::initialize(path).unwrap().0)
}

#[test]
fn committed_mutations_emit_once_and_delivery_does_not_change_the_result() {
    let path = std::env::temp_dir().join(format!("isct-workspace-events-{}.db", Uuid::new_v4()));
    let store = DataStore::initialize(path).unwrap().0;
    let events = Arc::new(MutationEvents {
        count: AtomicUsize::new(0),
        fail: false,
    });
    let service = WorkspaceService::with_events(store, events.clone());
    let initial = service.snapshot().unwrap();
    let committed = service
        .mutate(WorkspaceMutation {
            expected_revision: initial.revision,
            intent: WorkspaceIntent::SetAdmissionYear {
                admission_year: Some(2026),
            },
        })
        .unwrap();
    assert_eq!(events.count.load(Ordering::Relaxed), 1);

    let failed = service.mutate(WorkspaceMutation {
        expected_revision: "stale".into(),
        intent: WorkspaceIntent::SetAdmissionYear {
            admission_year: Some(2025),
        },
    });
    assert!(matches!(failed, Err(WorkspaceError::Conflict(_))));
    assert_eq!(events.count.load(Ordering::Relaxed), 1);

    let failing_events = Arc::new(MutationEvents {
        count: AtomicUsize::new(0),
        fail: true,
    });
    let service = WorkspaceService::with_events(service.store.clone(), failing_events.clone());
    assert!(service
        .mutate(WorkspaceMutation {
            expected_revision: committed.revision,
            intent: WorkspaceIntent::SetAdmissionYear {
                admission_year: Some(2027),
            },
        })
        .is_ok());
    assert_eq!(failing_events.count.load(Ordering::Relaxed), 1);
}

fn settings_with(snapshot: &WorkspaceSnapshot, edit: impl FnOnce(&mut Value)) -> Value {
    let mut settings = snapshot.workspace["settings"].clone();
    edit(&mut settings);
    settings
}

#[test]
fn stale_revision_cannot_overwrite_a_newer_settings_change() {
    let service = service("conflict");
    let original = service.snapshot().unwrap();
    let changed = service
        .mutate(WorkspaceMutation {
            expected_revision: original.revision.clone(),
            intent: WorkspaceIntent::ReplaceSettings {
                settings: settings_with(&original, |s| s["appearance"] = json!("dark")),
            },
        })
        .unwrap();
    assert_ne!(changed.revision, original.revision);
    assert_eq!(changed.workspace["settings"]["appearance"], "dark");

    let error = service
        .mutate(WorkspaceMutation {
            expected_revision: original.revision.clone(),
            intent: WorkspaceIntent::ReplaceSettings {
                settings: settings_with(&original, |s| s["density"] = json!("compact")),
            },
        })
        .unwrap_err();
    assert!(matches!(error, WorkspaceError::Conflict(_)));
    assert_eq!(
        service.snapshot().unwrap().workspace["settings"]["density"],
        changed.workspace["settings"]["density"]
    );
}

#[test]
fn cached_settings_follow_external_commits_and_core_validates_every_value() {
    let service = service("settings");
    let original = service.snapshot().unwrap();
    let chosen = mutate(
        &service,
        WorkspaceIntent::ReplaceSettings {
            settings: settings_with(&original, |s| {
                s["departmentId"] = json!("cs");
                s["textSize"] = json!("large");
                s["dataNoticeAcknowledgedAt"] = json!("2026-10-18T10:00:00Z");
            }),
        },
    );
    assert_eq!(chosen.workspace["settings"]["departmentId"], "cs");
    // Storage splits settings by an allowlist; a key missing from it is lost.
    assert_eq!(
        chosen.workspace["settings"]["dataNoticeAcknowledgedAt"],
        "2026-10-18T10:00:00Z"
    );
    // It is a device preference: cloud sync never carries it.
    let mut synced = json!({"key":"userPreferences","value":{"dataNoticeAcknowledgedAt":"x"}});
    crate::data::cloud_merge::sanitize_setting(&mut synced);
    assert_eq!(synced["value"], json!({}));
    // Warm the reader, then commit through a separately initialized handle.
    // PRAGMA data_version must invalidate the cached projection, even though
    // this background update deliberately keeps the user-intent revision.
    let warm = service.snapshot().unwrap();
    let external = DataStore::initialize(service.store.path().to_path_buf())
        .unwrap()
        .0;
    external.record_reference_poll().unwrap();
    let polled = service.snapshot().unwrap();
    let checked = polled.workspace["settings"]["subscriptionRefresh"]["lastCheckedAt"].clone();
    assert!(checked.is_string());
    assert_eq!(polled.revision, warm.revision);
    assert_ne!(polled.content_revision, warm.content_revision);

    // A form built from the earlier snapshot clears the department and
    // cannot roll the Core-owned poll clock back.
    let cleared = mutate(
        &service,
        WorkspaceIntent::ReplaceSettings {
            settings: settings_with(&chosen, |s| {
                s.as_object_mut().unwrap().remove("departmentId");
            }),
        },
    );
    assert!(cleared.workspace["settings"].get("departmentId").is_none());
    assert_eq!(cleared.workspace["settings"]["textSize"], "large");
    assert_eq!(
        cleared.workspace["settings"]["subscriptionRefresh"]["lastCheckedAt"],
        checked
    );

    let before = service.snapshot().unwrap();
    for (pointer, value) in [
        ("/locale", json!("xx")),
        ("/appearance", json!("blue")),
        ("/textSize", json!("huge")),
        ("/contentLanguage", json!("fr")),
        ("/density", json!(3)),
        ("/cloud/provider", json!("dropbox")),
    ] {
        let error = service
            .mutate(WorkspaceMutation {
                expected_revision: before.revision.clone(),
                intent: WorkspaceIntent::ReplaceSettings {
                    settings: settings_with(&before, |s| {
                        let (parent, key) = pointer.rsplit_once('/').unwrap();
                        s.pointer_mut(parent).unwrap()[key] = value;
                    }),
                },
            })
            .unwrap_err();
        assert!(
            matches!(error, WorkspaceError::InvalidIntent(_)),
            "{pointer}"
        );
    }
    for intent in [
        WorkspaceIntent::SetAdmissionYear {
            admission_year: Some(1999),
        },
        WorkspaceIntent::EditAcademicCalendar {
            academic_profile: json!({"name":"Mine","admissionYear":1990,"gradePath":[],"years":[]}),
            exceptions: vec![],
        },
    ] {
        let error = service
            .mutate(WorkspaceMutation {
                expected_revision: before.revision.clone(),
                intent,
            })
            .unwrap_err();
        assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
    }
    assert!(matches!(
        service
            .store
            .put_workspace_setting("devicePreferences", json!({"locale":"xx"}), "test"),
        Err(DataError::InvalidInput(_))
    ));
    assert_eq!(service.snapshot().unwrap().revision, before.revision);
    let mut invalid_periods = crate::schedule::default_teaching_periods();
    invalid_periods[1].start_time = "08:00".into();
    let mut profile = before.workspace["academicProfile"].clone();
    profile["teachingPeriods"] = serde_json::to_value(invalid_periods).unwrap();
    let error = service
        .mutate(WorkspaceMutation {
            expected_revision: before.revision.clone(),
            intent: WorkspaceIntent::EditAcademicCalendar {
                academic_profile: profile,
                exceptions: vec![],
            },
        })
        .unwrap_err();
    assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
    assert_eq!(service.snapshot().unwrap().revision, before.revision);
    let year = mutate(
        &service,
        WorkspaceIntent::SetAdmissionYear {
            admission_year: Some(2024),
        },
    );
    assert_eq!(year.workspace["academicProfile"]["admissionYear"], 2024);
}

#[test]
fn study_profile_and_calendar_edits_preserve_the_other_profile_fields() {
    let service = service("study-profile");
    let profile = json!({
        "name":"My program",
        "admissionYear":2024,
        "studyEnrollments":[],
        "teachingPeriods":crate::schedule::default_teaching_periods(),
        "gradePath":[{"grade":"B2","academicYear":2025}],
        "years":[{"academicYear":2026,"quarterStarts":{
            "1Q":"2026-04-01","2Q":"2026-06-01","3Q":"2026-10-01","4Q":"2026-12-01"
        }}]
    });
    service
        .store
        .put_workspace_setting("academicProfile", profile.clone(), "test")
        .unwrap();
    let old = service.snapshot().unwrap();
    assert_eq!(
        old.workspace["academicProfile"]["studyEnrollments"],
        json!([])
    );

    let saved = run(
        &service,
        WorkspaceIntent::SetStudyProfile {
            admission_year: Some(2024),
            study_enrollments: vec![
                StudyEnrollment {
                    stage: crate::study_profile::StudyStage::Undergraduate,
                    entry_kind: crate::study_profile::StudyEntryKind::Regular,
                    entry_grade: Some(1),
                    admitted_term: "2024-1Q".into(),
                },
                StudyEnrollment {
                    stage: crate::study_profile::StudyStage::Master,
                    entry_kind: crate::study_profile::StudyEntryKind::Regular,
                    entry_grade: Some(1),
                    admitted_term: "2028-1Q".into(),
                },
            ],
        },
    )
    .unwrap();
    assert_eq!(saved["academicProfile"]["name"], profile["name"]);
    assert_eq!(saved["academicProfile"]["gradePath"], profile["gradePath"]);
    assert_eq!(saved["academicProfile"]["years"], profile["years"]);
    assert_eq!(
        saved["academicProfile"]["studyEnrollments"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        saved["academicProfile"]["teachingPeriods"],
        profile["teachingPeriods"]
    );
    let mut edited = saved["academicProfile"].clone();
    edited["name"] = json!("Renamed program");
    edited["teachingPeriods"] = Value::Null;
    let calendar = run(
        &service,
        WorkspaceIntent::EditAcademicCalendar {
            academic_profile: edited,
            exceptions: vec![],
        },
    )
    .unwrap();
    assert_eq!(
        calendar["academicProfile"]["studyEnrollments"],
        saved["academicProfile"]["studyEnrollments"],
        "calendar edits include the complete profile"
    );
    assert_eq!(
        crate::schedule::teaching_periods(&calendar).unwrap(),
        crate::schedule::default_teaching_periods()
    );
}

#[test]
fn invalid_study_history_is_rejected_atomically() {
    let service = service("invalid-study-profile");
    let before = service.snapshot().unwrap();
    for study_enrollments in [
        vec![StudyEnrollment {
            stage: crate::study_profile::StudyStage::Undergraduate,
            entry_kind: crate::study_profile::StudyEntryKind::Regular,
            entry_grade: Some(1),
            admitted_term: "2026-spring".into(),
        }],
        vec![
            StudyEnrollment {
                stage: crate::study_profile::StudyStage::Undergraduate,
                entry_kind: crate::study_profile::StudyEntryKind::Regular,
                entry_grade: Some(1),
                admitted_term: "2026-1Q".into(),
            },
            StudyEnrollment {
                stage: crate::study_profile::StudyStage::Master,
                entry_kind: crate::study_profile::StudyEntryKind::Regular,
                entry_grade: Some(1),
                admitted_term: "2026-1Q".into(),
            },
        ],
    ] {
        let error = service
            .mutate(WorkspaceMutation {
                expected_revision: before.revision.clone(),
                intent: WorkspaceIntent::SetStudyProfile {
                    admission_year: Some(2026),
                    study_enrollments,
                },
            })
            .unwrap_err();
        assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
        let after = service.snapshot().unwrap();
        assert_eq!(after.revision, before.revision);
        assert_eq!(
            after.workspace["academicProfile"],
            before.workspace["academicProfile"]
        );
    }
}

#[test]
fn invalid_record_rolls_back_the_whole_intent() {
    let service = service("rollback");
    let original = service.snapshot().unwrap();
    let error = service
        .mutate(WorkspaceMutation {
            expected_revision: original.revision.clone(),
            intent: WorkspaceIntent::SaveEvent {
                draft: json!({"title":"Missing required event fields"}),
                term_id: "2026-3Q".into(),
                existing_id: None,
            },
        })
        .unwrap_err();
    assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
    let after = service.snapshot().unwrap();
    assert_eq!(after.revision, original.revision);
    assert!(after.workspace["events"].as_array().unwrap().is_empty());
}

#[test]
fn save_plan_canonicalizes_civil_time_and_rejects_invalid_drafts_atomically() {
    let service = service("plan-validation");
    let original = service.snapshot().unwrap();
    let saved = service
        .mutate(WorkspaceMutation {
            expected_revision: original.revision,
            intent: WorkspaceIntent::SavePlan {
                draft: json!({
                    "title":" Study ", "start":"2026-10-14T09:00", "duration":60,
                    "rrule":"freq=weekly;count=2", "exdates":["2026-10-21T18:00:00+09:00"]
                }),
                existing_id: None,
            },
        })
        .unwrap();
    assert_eq!(saved.workspace["plans"][0]["title"], "Study");
    assert_eq!(
        saved.workspace["plans"][0]["start"],
        "2026-10-14T09:00:00+09:00"
    );
    assert_eq!(
        saved.workspace["plans"][0]["exdates"],
        json!(["2026-10-21"])
    );

    let before_invalid = service.snapshot().unwrap();
    let error = service
        .mutate(WorkspaceMutation {
            expected_revision: before_invalid.revision.clone(),
            intent: WorkspaceIntent::SavePlan {
                draft: json!({
                    "title":"Broken", "start":"not-a-date", "duration":0, "exdates":[]
                }),
                existing_id: None,
            },
        })
        .unwrap_err();
    assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
    let after_invalid = service.snapshot().unwrap();
    assert_eq!(after_invalid.revision, before_invalid.revision);
    assert_eq!(
        after_invalid.workspace["plans"].as_array().unwrap().len(),
        1
    );

    let error = service
        .mutate(WorkspaceMutation {
            expected_revision: after_invalid.revision.clone(),
            intent: WorkspaceIntent::SavePlan {
                draft: json!({
                    "title":"Broken recurrence", "start":"2026-10-14T09:00",
                    "duration":60, "rrule":"FREQ=DAILY;INTERVAL=garbage", "exdates":[]
                }),
                existing_id: None,
            },
        })
        .unwrap_err();
    assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
    let after_rule = service.snapshot().unwrap();
    assert_eq!(after_rule.revision, after_invalid.revision);
    assert_eq!(after_rule.workspace["plans"].as_array().unwrap().len(), 1);
}

#[test]
fn schedule_override_periods_must_name_a_class_slot() {
    let service = service("override-periods");
    let original = service.snapshot().unwrap();
    for draft in [
        json!({"targetKey":"course-weekly-0-2026-10-14","targetKind":"course","periods":[5,3],"hidden":false}),
        // Exams have their own intents so a link cannot freeze published facts.
        json!({"targetKey":"exam:any","targetKind":"exam","courseId":"course","hidden":false}),
    ] {
        let error = service
            .mutate(WorkspaceMutation {
                expected_revision: original.revision.clone(),
                intent: WorkspaceIntent::SaveScheduleOverride {
                    draft,
                    existing_id: None,
                },
            })
            .unwrap_err();
        assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
        assert_eq!(service.snapshot().unwrap().revision, original.revision);
    }
}

#[test]
fn exam_corrections_keep_only_changed_fields_so_source_refreshes_still_arrive() {
    use crate::subscriptions::{
        apply_exam_feed, validate_exam_feed, ExamFeedKind, PublishedExamFeed,
    };
    let service = service("exam-association");
    let added = mutate(
        &service,
        WorkspaceIntent::AddCatalogCourses {
            term_id: "2026-2Q".into(),
            offerings: vec![offering("202602001", "CSC.T241", &["2Q"])],
        },
    );
    let course_id = added.workspace["courses"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let raw = json!({"schemaVersion":1,"academicYear":2026,"timezone":"Asia/Tokyo",
    "source":[{"id":"official","title":"Exam notice","url":"https://example.com/exams.pdf"}],
    "entries":[
        {"id":"first","kind":"exam","status":"scheduled","quarter":"2Q","courseCode":"CSC.T241","syllabusCourseId":"202602001","date":"2026-07-29","periods":[1,2],"label":{"ja":"試験1","en":"First exam"},"sourceIds":["official"]},
        {"id":"second","kind":"exam","status":"scheduled","quarter":"2Q","courseCode":"CSC.T241-02","date":"2026-08-04","periods":[3,4],"label":{"ja":"試験2","en":"Second exam"},"room":{"ja":"教室","en":"Source room"},"sourceIds":["official"]}
    ]});
    let mut published = PublishedExamFeed {
        kind: ExamFeedKind::Exams,
        level: "undergraduate".into(),
        quarter: "2q".into(),
        url: "https://example.com/exams.json".into(),
        feed: validate_exam_feed(&raw.to_string(), 2026, "2q", ExamFeedKind::Exams).unwrap(),
    };
    let mut connection = rusqlite::Connection::open(service.store.path()).unwrap();
    apply_exam_feed(&mut connection, &published).unwrap();
    let first = "reference:exams:2026:undergraduate:2q:first";
    let second = "reference:exams:2026:undergraduate:2q:second";
    let view = |snapshot: &WorkspaceSnapshot, key: &str| {
        crate::schedule::exam_views(&snapshot.workspace, "2026-2Q")
            .into_iter()
            .find(|exam| exam.key == key)
            .unwrap()
    };
    let stored = |snapshot: &WorkspaceSnapshot| {
        snapshot.workspace["scheduleOverrides"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["deletedAt"].is_null())
            .cloned()
            .collect::<Vec<_>>()
    };
    let before = service.snapshot().unwrap();
    assert!(view(&before, first).auto_linked);
    // Linking reports the course's other exam but no longer blocks.
    let preview = crate::schedule::exam_association(&before.workspace, second, &course_id).unwrap();
    assert_eq!(preview.conflicts.len(), 1);
    let linked = mutate(
        &service,
        WorkspaceIntent::LinkExam {
            exam_key: second.into(),
            course_id: Some(course_id.clone()),
        },
    );
    let rows = stored(&linked);
    assert_eq!(rows.len(), 1);
    for field in ["date", "startTime", "endTime", "periods", "room"] {
        assert!(rows[0].get(field).is_none(), "a link froze {field}");
    }
    let exam = view(&linked, second);
    assert_eq!(exam.course_id.as_deref(), Some(course_id.as_str()));
    assert!(!exam.auto_linked && !exam.edited);
    assert_eq!(
        crate::schedule::course_detail(&linked.workspace, &course_id)
            .exam_ids
            .len(),
        2
    );

    published.feed.entries[1].room =
        Some(serde_json::from_value(json!({"ja":"新教室","en":"Updated source room"})).unwrap());
    apply_exam_feed(&mut connection, &published).unwrap();
    assert_eq!(
        view(&service.snapshot().unwrap(), second).room,
        "Updated source room"
    );

    // The form submits every field; only the room differs from the source.
    let exam = view(&service.snapshot().unwrap(), second);
    let edited = mutate(
        &service,
        WorkspaceIntent::EditExam {
            exam_key: second.into(),
            date: Some(exam.date.clone()),
            start_time: exam.start_time.clone(),
            end_time: exam.end_time.clone(),
            room: Some("Personal room".into()),
            hidden: false,
        },
    );
    let row = &stored(&edited)[0];
    assert_eq!(row["room"], "Personal room");
    assert_eq!(row["courseId"], course_id.as_str());
    assert!(row.get("date").is_none() && row.get("startTime").is_none());
    assert_eq!(view(&edited, second).periods, Some([3, 4]));

    published.feed.entries[1].date = "2026-08-05".into();
    apply_exam_feed(&mut connection, &published).unwrap();
    let refreshed = view(&service.snapshot().unwrap(), second);
    assert_eq!(
        (
            refreshed.date.as_str(),
            refreshed.room.as_str(),
            refreshed.edited
        ),
        ("2026-08-05", "Personal room", true)
    );
    assert_eq!(refreshed.source.room, "Updated source room");

    let original = service.snapshot().unwrap();
    for (date, start, end) in [
        ("2026-02-30", None, None),
        ("2026-08-05", Some("13:30"), None),
        ("2026-08-05", Some("25:00"), Some("26:00")),
        ("2026-08-05", Some("13:30"), Some("10:30")),
    ] {
        assert!(service
            .mutate(WorkspaceMutation {
                expected_revision: original.revision.clone(),
                intent: WorkspaceIntent::EditExam {
                    exam_key: second.into(),
                    date: Some(date.into()),
                    start_time: start.map(Into::into),
                    end_time: end.map(Into::into),
                    room: None,
                    hidden: false,
                },
            })
            .is_err());
        assert_eq!(service.snapshot().unwrap().revision, original.revision);
    }

    // Reset keeps the link; choosing the matched course stores nothing.
    let reset = mutate(
        &service,
        WorkspaceIntent::EditExam {
            exam_key: second.into(),
            date: None,
            start_time: None,
            end_time: None,
            room: None,
            hidden: false,
        },
    );
    let exam = view(&reset, second);
    assert_eq!(exam.course_id.as_deref(), Some(course_id.as_str()));
    assert!(!exam.edited);
    let rematched = mutate(
        &service,
        WorkspaceIntent::LinkExam {
            exam_key: first.into(),
            course_id: Some(course_id.clone()),
        },
    );
    assert_eq!(stored(&rematched).len(), 1);
    let unlinked = mutate(
        &service,
        WorkspaceIntent::LinkExam {
            exam_key: first.into(),
            course_id: None,
        },
    );
    assert_eq!(view(&unlinked, first).course_id, None);
    let cleared = mutate(
        &service,
        WorkspaceIntent::LinkExam {
            exam_key: second.into(),
            course_id: None,
        },
    );
    // Unlinking an exam nothing matches is its automatic state: the row goes.
    assert_eq!(stored(&cleared).len(), 1);
    assert_eq!(view(&cleared, second).course_id, None);

    // A right-click mark is one more field of the same override, so a later
    // schedule edit keeps it and marking the source kind removes it again.
    let marked = mutate(
        &service,
        WorkspaceIntent::MarkSession {
            key: second.into(),
            mark: "cancelled".into(),
        },
    );
    assert_eq!(view(&marked, second).status, "cancelled");
    let edited = mutate(
        &service,
        WorkspaceIntent::EditExam {
            exam_key: second.into(),
            date: None,
            start_time: None,
            end_time: None,
            room: Some("Mark room".into()),
            hidden: false,
        },
    );
    assert_eq!(view(&edited, second).mark.as_deref(), Some("cancelled"));
    mutate(
        &service,
        WorkspaceIntent::EditExam {
            exam_key: second.into(),
            date: None,
            start_time: None,
            end_time: None,
            room: None,
            hidden: false,
        },
    );
    let restored = mutate(
        &service,
        WorkspaceIntent::MarkSession {
            key: second.into(),
            mark: "exam".into(),
        },
    );
    assert_eq!(stored(&restored).len(), 1); // only the first exam's unlink
    assert_eq!(view(&restored, second).status, "scheduled");
}

#[test]
fn course_batch_validation_happens_before_any_row_is_changed() {
    let service = service("course-batch");
    let original = service.snapshot().unwrap();
    let error = service
        .mutate(WorkspaceMutation {
            expected_revision: original.revision.clone(),
            intent: WorkspaceIntent::EditCourseBatch {
                course_ids: vec!["missing".into()],
                changes: CourseBatchChanges {
                    credits: Some(21.0),
                    ..CourseBatchChanges::default()
                },
            },
        })
        .unwrap_err();
    assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
    assert_eq!(service.snapshot().unwrap().revision, original.revision);
}

fn mutate(service: &WorkspaceService, intent: WorkspaceIntent) -> WorkspaceSnapshot {
    if let WorkspaceIntent::AddCatalogCourses { offerings, .. } = &intent {
        service.store.catalog_upsert_offerings(offerings).unwrap();
    }
    let expected_revision = service.snapshot().unwrap().revision;
    service
        .mutate(WorkspaceMutation {
            expected_revision,
            intent,
        })
        .unwrap()
}

fn course<'a>(snapshot: &'a WorkspaceSnapshot, id: &str) -> &'a Value {
    snapshot.workspace["courses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|course| course["id"] == id)
        .unwrap()
}

#[test]
fn course_edits_merge_into_one_overlay_and_batch_credits_reach_every_course() {
    let service = service("course-overlay");
    let added = mutate(
        &service,
        WorkspaceIntent::AddCatalogCourses {
            term_id: "2026-3Q".into(),
            offerings: vec![json!({
                "id":"202600002", "year":2026, "quarters":["3Q"],
                "code":"CSC.T364", "title":{"en":"Systems"},
                "instructors":[{"en":"Teacher One"}],
                "syllabusUrl":"https://syllabus.s.isct.ac.jp/courses/202600002",
                "credits":{"lecture":2,"exercise":0,"lab":0},
                "instructionLanguage":"ja", "format":{"kinds":["lecture"]}, "media":{},
                "academicUnit":{}, "intensive":false, "fetchedAt":{},
                "schedule":[{"kind":"weekly","day":1,"periods":[1,2]}]
            })],
        },
    );
    let catalog_id = added.workspace["courses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|course| course["offeringId"] == "202600002")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let manual = mutate(
        &service,
        WorkspaceIntent::SaveCourse {
            draft: json!({"code":"X.1","title":"Own","instructor":"Me","credits":1,"meetings":[]}),
            term_id: "2026-3Q".into(),
            existing_id: None,
        },
    );
    let manual_id = manual.workspace["courses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|course| course["title"] == "Own")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    mutate(
        &service,
        WorkspaceIntent::SaveCourse {
            draft: json!({"title":"Renamed","instructor":"Teacher A","color":"#123456"}),
            term_id: "2026-3Q".into(),
            existing_id: Some(catalog_id.clone()),
        },
    );
    let marked = mutate(
        &service,
        WorkspaceIntent::RecordLesson {
            course_id: catalog_id.clone(),
            occurrence: "2026-10-05T08:50:00+09:00".into(),
            section_id: 3,
        },
    );
    let overrides = &course(&marked, &catalog_id)["overrides"];
    assert_eq!(overrides["title"], "Renamed");
    assert_eq!(overrides["color"], "#123456");
    assert_eq!(overrides["lessonLog"][0]["sectionId"], 3);

    let edited = mutate(
        &service,
        WorkspaceIntent::EditCourseBatch {
            course_ids: vec![catalog_id.clone(), manual_id.clone()],
            changes: CourseBatchChanges {
                credits: Some(4.0),
                ..CourseBatchChanges::default()
            },
        },
    );
    let catalog = course(&edited, &catalog_id);
    assert_eq!(catalog["credits"], 4.0);
    assert_eq!(catalog["overrides"]["title"], "Renamed");
    assert_eq!(catalog["overrides"]["instructor"], "Teacher A");
    assert_eq!(catalog["overrides"]["lessonLog"][0]["sectionId"], 3);
    assert_eq!(course(&edited, &manual_id)["credits"], 4.0);
    assert_eq!(course(&edited, &manual_id)["instructor"], "Me");

    let renamed = mutate(
        &service,
        WorkspaceIntent::EditCourseBatch {
            course_ids: vec![catalog_id.clone(), manual_id.clone()],
            changes: CourseBatchChanges {
                instructor: Some(" Teacher B ".into()),
                language: Some("en".into()),
                ..CourseBatchChanges::default()
            },
        },
    );
    let catalog = course(&renamed, &catalog_id);
    assert_eq!(catalog["overrides"]["instructor"], "Teacher B");
    assert_eq!(catalog["overrides"]["color"], "#123456");
    assert_eq!(catalog["instructionLanguage"], "en");
    assert_eq!(catalog["credits"], 4.0);
    assert_eq!(course(&renamed, &manual_id)["instructor"], "Teacher B");
    assert_eq!(course(&renamed, &manual_id)["instructionLanguage"], "en");
}

#[test]
fn catalog_course_preserves_localized_fields_credits_rooms_and_long_period_ranges() {
    let service = service("catalog-course");
    service
            .store
            .catalog_upsert_offerings(&[json!({
                "id":"202600001", "year":2026, "quarters":["3Q"],
                "code":"CSC.T363", "title":{"ja":"演習","en":"Exercise"},
                "instructors":[{"ja":"教員一","en":"Teacher One"},{"ja":"教員二","en":"Teacher Two"}],
                "syllabusUrl":"https://syllabus.s.isct.ac.jp/courses/202600001",
                "credits":{"lecture":0,"exercise":1.5,"lab":0.5},
                "instructionLanguage":"ja", "format":{"kinds":["exercise"]}, "media":{},
                "academicUnit":{}, "intensive":false, "fetchedAt":{},
                "content":{"summary":{"ja":"内容"}},
                "schedule":[{"kind":"weekly","day":2,"periods":[5,8],"room":{"ja":"西講義棟","en":"West Hall"}}]
            })])
            .unwrap();
    let original = service.snapshot().unwrap();
    let next = service
            .mutate(WorkspaceMutation {
                expected_revision: original.revision,
                intent: WorkspaceIntent::AddCatalogCourses {
                    term_id: "2026-3Q".into(),
                    offerings: vec![json!({
                        "id":"202600001", "year":2026, "quarters":["3Q"],
                        "code":"CSC.T363", "title":{"ja":"演習","en":"Exercise"},
                        "instructors":[{"ja":"教員一","en":"Teacher One"},{"ja":"教員二","en":"Teacher Two"}],
                        "syllabusUrl":"https://syllabus.s.isct.ac.jp/courses/202600001",
                        "credits":{"lecture":0,"exercise":1.5,"lab":0.5},
                        "instructionLanguage":"ja",
                        "content":{"summary":{"ja":"内容"}},
                        "schedule":[{"kind":"weekly","day":2,"periods":[5,8],"room":{"ja":"西講義棟","en":"West Hall"}}]
                    })],
                },
            })
            .unwrap();
    let course = next.workspace["courses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|course| course["offeringId"] == "202600001")
        .unwrap();
    assert_eq!(course["title"], "Exercise");
    assert_eq!(course["instructor"], "Teacher One / Teacher Two");
    assert_eq!(course["credits"], 2.0);
    assert_eq!(course["creditSplit"]["exercise"], 1.5);
    assert_eq!(course["meetings"][0]["startTime"], "13:30");
    assert_eq!(course["meetings"][0]["endTime"], "17:05");
    assert_eq!(course["meetings"][0]["room"], "West Hall");
    assert_eq!(course["meetings"][0]["roomNames"]["ja"], "西講義棟");

    let converted = catalog_course(
            &json!({
                "id":"202600001", "year":2026, "quarters":["3Q"],
                "code":"CSC.T363", "title":{"ja":"演習","en":"Exercise"},
                "instructors":[{"ja":"教員一","en":"Teacher One"},{"ja":"教員二","en":"Teacher Two"}],
                "syllabusUrl":"https://syllabus.s.isct.ac.jp/courses/202600001",
                "credits":{"lecture":0,"exercise":1.5,"lab":0.5},
                "instructionLanguage":"ja", "sourceUpdated":"2026-10-01",
                "schedule":[{"kind":"weekly","day":2,"periods":[5,8],"room":{"ja":"西講義棟","en":"West Hall"}}]
            }),
            "ja",
            Some(&json!({"summary":{"ja":"内容"}})),
            &crate::schedule::default_teaching_periods(),
        )
        .unwrap();
    assert_eq!(converted["title"], "演習");
    assert_eq!(converted["instructor"], "教員一 / 教員二");
    assert_eq!(converted["credits"], 2.0);
    assert_eq!(converted["meetings"][0]["endTime"], "17:05");
    assert_eq!(converted["content"]["summary"]["ja"], "内容");
    // A language change reprojects every quarter, including a non-current one.
    let mut offering = service
        .store
        .catalog_offering("202600001")
        .unwrap()
        .unwrap();
    offering["id"] = json!("202600002");
    offering["code"] = json!("CSC.T364");
    offering["quarters"] = json!(["4Q"]);
    service
        .store
        .catalog_upsert_offerings(&[offering.clone()])
        .unwrap();
    mutate(
        &service,
        WorkspaceIntent::AddCatalogCourses {
            term_id: "2026-4Q".into(),
            offerings: vec![offering],
        },
    );
    let before_language = service.snapshot().unwrap();
    let japanese = mutate(
        &service,
        WorkspaceIntent::ReplaceSettings {
            settings: settings_with(&before_language, |settings| {
                settings["contentLanguage"] = json!("ja")
            }),
        },
    );
    let rows = japanese.workspace["courses"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    for row in rows {
        assert_eq!(row["title"], "演習");
        assert_eq!(row["meetings"][0]["room"], "西講義棟");
    }
    let english = mutate(
        &service,
        WorkspaceIntent::ReplaceSettings {
            settings: settings_with(&japanese, |settings| {
                settings["contentLanguage"] = json!("en")
            }),
        },
    );
    for row in english.workspace["courses"].as_array().unwrap() {
        assert_eq!(row["title"], "Exercise");
        assert_eq!(row["instructor"], "Teacher One / Teacher Two");
    }
}

#[test]
fn subscription_toggle_and_reset_are_one_revision_checked_intent() {
    let service = service("subscription");
    let original = service.snapshot().unwrap();
    let created = service
        .mutate(WorkspaceMutation {
            expected_revision: original.revision,
            intent: WorkspaceIntent::SetSubscription {
                subscription_kind: "calendar".into(),
                enabled: Some(false),
                url: Some("https://example.test/index.json".into()),
                reset_remote: true,
            },
        })
        .unwrap();
    let row = &created.workspace["subscriptions"][0];
    assert_eq!(row["id"], "subscription:public:calendar");
    assert_eq!(row["enabled"], false);
    assert_eq!(row["url"], "https://example.test/index.json");
    assert!(row.get("remoteHash").is_none());

    let error = service
        .mutate(WorkspaceMutation {
            expected_revision: created.revision,
            intent: WorkspaceIntent::SetSubscription {
                subscription_kind: "calendar".into(),
                enabled: None,
                url: Some("http://insecure.test/index.json".into()),
                reset_remote: false,
            },
        })
        .unwrap_err();
    assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
}

#[test]
fn scheduled_task_run_is_committed_with_the_workspace_revision() {
    let service = service("scheduled-task-run");
    let original = service.snapshot().unwrap();
    let task_id = original.workspace["scheduledTasks"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let next = service
        .mutate(WorkspaceMutation {
            expected_revision: original.revision.clone(),
            intent: WorkspaceIntent::RecordScheduledTaskRun {
                task_id: task_id.clone(),
                occurrence_at: "2026-09-29T12:00:00.000Z".into(),
                run_at: "2026-09-29T12:00:01.000Z".into(),
                result: "Export completed".into(),
            },
        })
        .unwrap();
    // Run bookkeeping is Core-owned: it changes the content, not the
    // baseline user edits are checked against.
    assert_ne!(next.content_revision, original.content_revision);
    assert_eq!(next.revision, original.revision);
    let task = next.workspace["scheduledTasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["id"] == task_id)
        .unwrap();
    assert_eq!(task["lastOccurrenceAt"], "2026-09-29T12:00:00.000Z");
    assert_eq!(task["lastRunAt"], "2026-09-29T12:00:01.000Z");
    assert_eq!(task["lastResult"], "Export completed");
    assert_eq!(
        service.snapshot().unwrap().content_revision,
        next.content_revision
    );

    // A run claim with a malformed clock, or for a task that is gone, is
    // rejected before any row changes.
    let before = service.snapshot().unwrap();
    let tasks_before = before.workspace["scheduledTasks"].clone();
    for (task, occurrence_at, run_at) in [
        (
            task_id.clone(),
            "not-a-time".to_string(),
            "2026-09-30T12:00:01.000Z".to_string(),
        ),
        (
            task_id.clone(),
            "2026-09-30T12:00:00.000Z".to_string(),
            "not-a-time".to_string(),
        ),
        (
            "missing-task".to_string(),
            "2026-09-30T12:00:00.000Z".to_string(),
            "2026-09-30T12:00:01.000Z".to_string(),
        ),
    ] {
        let error = service
            .mutate(WorkspaceMutation {
                expected_revision: before.revision.clone(),
                intent: WorkspaceIntent::RecordScheduledTaskRun {
                    task_id: task.clone(),
                    occurrence_at,
                    run_at,
                    result: "Rejected".into(),
                },
            })
            .unwrap_err();
        assert!(matches!(error, WorkspaceError::InvalidIntent(_)), "{task}");
        let after = service.snapshot().unwrap();
        assert_eq!(after.revision, before.revision);
        assert_eq!(after.content_revision, before.content_revision);
        assert_eq!(after.workspace["scheduledTasks"], tasks_before);
    }
}

#[test]
fn background_ingest_and_bookkeeping_do_not_conflict_with_user_edits() {
    let service = service("ingest-revision");
    let before = service.snapshot().unwrap();
    // A school ingest, a reference poll and a scheduler run land between
    // the person's snapshot and their edit.
    service
        .store
        .catalog_upsert_offerings(&[json!({
            "id":"202600009", "year":2026, "quarters":["3Q"], "code":"X.9",
            "title":{"en":"Ingested"}, "instructors":[], "instructionLanguage":"en",
            "format":{"kinds":["lecture"]}, "media":{},
            "credits":{"lecture":1.0,"exercise":0.0,"lab":0.0}, "schedule":[],
            "academicUnit":{"en":"Unit"}, "intensive":false,
            "syllabusUrl":"https://example.invalid/202600009", "fetchedAt":{}
        })])
        .unwrap();
    service.store.record_reference_poll().unwrap();
    let mut task = service.store.user_rows("scheduled_tasks").unwrap()[0].clone();
    let rev = task["rev"].as_u64();
    task["lastRunAt"] = json!("2026-10-14T00:00:00Z");
    task["lastResult"] = json!("Done");
    service
        .store
        .put_user_record_checked("scheduled_tasks", task, "scheduler", rev)
        .unwrap();
    let ingested = service.snapshot().unwrap();
    assert_ne!(ingested.content_revision, before.content_revision);
    assert_eq!(ingested.revision, before.revision);

    let edited = service
        .mutate(WorkspaceMutation {
            expected_revision: before.revision.clone(),
            intent: WorkspaceIntent::SelectTerm {
                term_id: "2026-4Q".into(),
            },
        })
        .unwrap();
    assert_ne!(edited.revision, before.revision);
    // A toggle built from the older snapshot keeps the newer run record.
    let stale_task = before.workspace["scheduledTasks"][0].clone();
    let toggled = mutate(
        &service,
        WorkspaceIntent::SaveScheduledTask {
            existing_id: stale_task["id"].as_str().map(str::to_owned),
            draft: json!({
                "name": stale_task["name"], "action": stale_task["action"],
                "schedule": stale_task["schedule"],
                "enabled": !stale_task["enabled"].as_bool().unwrap()
            }),
        },
    );
    let saved = toggled.workspace["scheduledTasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["id"] == stale_task["id"])
        .unwrap();
    assert_eq!(saved["enabled"], !stale_task["enabled"].as_bool().unwrap());
    assert_eq!(saved["lastResult"], "Done");
    // A user edit still conflicts with another user edit.
    let error = service
        .mutate(WorkspaceMutation {
            expected_revision: edited.revision,
            intent: WorkspaceIntent::SelectTerm {
                term_id: "2026-3Q".into(),
            },
        })
        .unwrap_err();
    assert!(matches!(error, WorkspaceError::Conflict(_)));
}

#[test]
fn calendar_exception_metadata_and_unique_date_are_core_owned() {
    let service = service("calendar-exception");
    let original = service.snapshot().unwrap();
    let created = service
            .mutate(WorkspaceMutation {
                expected_revision: original.revision,
                intent: WorkspaceIntent::SaveCalendarException {
                    draft: json!({"date":"2026-10-13","type":"replacement","replacementDay":2,"label":" Tuesday classes "}),
                    existing_id: None,
                },
            })
            .unwrap();
    let row = &created.workspace["exceptions"][0];
    assert!(row["id"].as_str().is_some_and(|id| !id.is_empty()));
    assert_eq!(row["rev"], 1);
    assert_eq!(row["label"], "Tuesday classes");

    let error = service
        .mutate(WorkspaceMutation {
            expected_revision: created.revision.clone(),
            intent: WorkspaceIntent::SaveCalendarException {
                draft: json!({"date":"2026-10-13","type":"holiday","label":"Duplicate"}),
                existing_id: None,
            },
        })
        .unwrap_err();
    assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
    assert_eq!(service.snapshot().unwrap().revision, created.revision);
}

fn run(service: &WorkspaceService, intent: WorkspaceIntent) -> Result<Value, WorkspaceError> {
    if let WorkspaceIntent::AddCatalogCourses { offerings, .. } = &intent {
        service.store.catalog_upsert_offerings(offerings).unwrap();
    }
    let expected_revision = service.snapshot()?.revision;
    service
        .mutate(WorkspaceMutation {
            expected_revision,
            intent,
        })
        .map(|snapshot| snapshot.workspace)
}

fn offering(id: &str, code: &str, quarters: &[&str]) -> Value {
    json!({
        "id":id, "year":2026, "quarters":quarters, "code":code,
        "title":{"en":code}, "instructors":[], "credits":{"lecture":2,"exercise":0,"lab":0},
        "instructionLanguage":"en", "format":{"kinds":["lecture"]}, "media":{},
        "academicUnit":{}, "intensive":false, "fetchedAt":{},
        "schedule":[{"kind":"weekly","day":1,"periods":[1,2],"room":{"en":"W1"}}],
        "syllabusUrl":format!("https://syllabus.s.isct.ac.jp/courses/{id}")
    })
}

#[test]
fn catalog_add_enforces_first_year_eligibility_atomically_and_preserves_existing_data() {
    let service = service("catalog-first-year");
    run(
        &service,
        WorkspaceIntent::AddCatalogCourses {
            term_id: "2026-3Q".into(),
            offerings: vec![offering("202630300", "CSC.T301", &["3Q"])],
        },
    )
    .unwrap();
    run(
        &service,
        WorkspaceIntent::SetStudyProfile {
            admission_year: Some(2026),
            study_enrollments: vec![StudyEnrollment {
                stage: crate::study_profile::StudyStage::Undergraduate,
                entry_kind: crate::study_profile::StudyEntryKind::Regular,
                entry_grade: Some(1),
                admitted_term: "2026-1Q".into(),
            }],
        },
    )
    .unwrap();
    let before = service.snapshot().unwrap();
    let error = run(
        &service,
        WorkspaceIntent::AddCatalogCourses {
            term_id: "2026-3Q".into(),
            offerings: vec![
                offering("202630100", "CSC.T101", &["3Q"]),
                offering("202630200", "LAH.T209-01", &["3Q"]),
            ],
        },
    )
    .unwrap_err();
    assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
    let rejected = service.snapshot().unwrap();
    assert_eq!(rejected.revision, before.revision);
    assert!(rows(&rejected.workspace, "courses")
        .unwrap()
        .any(|course| is_live(course) && course["code"] == "CSC.T301"));
    assert!(!rows(&rejected.workspace, "courses")
        .unwrap()
        .any(|course| course["code"] == "CSC.T101"));

    let first_year = run(
        &service,
        WorkspaceIntent::AddCatalogCourses {
            term_id: "2026-3Q".into(),
            offerings: vec![offering("202630100", "CSC.T101", &["3Q"])],
        },
    )
    .unwrap();
    assert!(rows(&first_year, "courses")
        .unwrap()
        .any(|course| is_live(course) && course["code"] == "CSC.T101"));

    let second_year = run(
        &service,
        WorkspaceIntent::AddCatalogCourses {
            term_id: "2027-3Q".into(),
            offerings: vec![offering("202630200", "CSC.T201", &["3Q"])],
        },
    )
    .unwrap();
    assert!(rows(&second_year, "courses")
        .unwrap()
        .any(|course| is_live(course) && course["code"] == "CSC.T201"));
}

#[test]
fn catalog_add_checks_authoritative_content_in_every_placed_quarter() {
    let service = service("catalog-authority");
    run(
        &service,
        WorkspaceIntent::SetStudyProfile {
            admission_year: Some(2026),
            study_enrollments: vec![
                StudyEnrollment {
                    stage: crate::study_profile::StudyStage::Undergraduate,
                    entry_kind: crate::study_profile::StudyEntryKind::Regular,
                    entry_grade: Some(1),
                    admitted_term: "2026-1Q".into(),
                },
                StudyEnrollment {
                    stage: crate::study_profile::StudyStage::Master,
                    entry_kind: crate::study_profile::StudyEntryKind::Regular,
                    entry_grade: Some(1),
                    admitted_term: "2026-4Q".into(),
                },
            ],
        },
    )
    .unwrap();
    let canonical = offering("202630209", "LAH.T209-01", &["3Q", "4Q"]);
    service
        .store
        .catalog_upsert_offerings(std::slice::from_ref(&canonical))
        .unwrap();
    let before = service.snapshot().unwrap();

    // The requested 4Q is a master's term, but the same offering would
    // also be placed into undergraduate 3Q and must be rejected there.
    let mut submitted = canonical;
    submitted["code"] = json!("CSC.T101");
    let error = service
        .mutate(WorkspaceMutation {
            expected_revision: before.revision.clone(),
            intent: WorkspaceIntent::AddCatalogCourses {
                term_id: "2026-4Q".into(),
                offerings: vec![submitted],
            },
        })
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "LAH.T209-01 is not available to a regular first-year undergraduate student."
    );
    let after = service.snapshot().unwrap();
    assert_eq!(after.revision, before.revision);
    assert!(!rows(&after.workspace, "courses").unwrap().any(is_live));
}

/// Live plan items as `(id, courseId, termId)`.
fn live_plans(workspace: &Value) -> Vec<(String, String, String)> {
    let mut plans: Vec<_> = rows(workspace, "planItems")
        .unwrap()
        .filter(|row| is_live(row))
        .map(|row| {
            (
                row["id"].as_str().unwrap().to_string(),
                row["courseId"].as_str().unwrap().to_string(),
                row["termId"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    plans.sort_by(|a, b| (&a.1, &a.2).cmp(&(&b.1, &b.2)));
    plans
}

fn placed_terms(workspace: &Value, course_id: &str) -> Vec<String> {
    let mut terms: Vec<_> = rows(workspace, "memberships")
        .unwrap()
        .chain(rows(workspace, "planItems").unwrap())
        .filter(|row| is_live(row) && row["courseId"] == course_id)
        .map(|row| row["termId"].as_str().unwrap().to_string())
        .collect();
    terms.sort();
    terms
}

#[test]
fn a_catalog_add_plans_every_quarter_and_undo_reverses_exactly_that_add() {
    let service = service("catalog-quarters");
    let add = |term: &str, offering: Value| {
        service
            .store
            .catalog_upsert_offerings(std::slice::from_ref(&offering))
            .unwrap();
        run(
            &service,
            WorkspaceIntent::AddCatalogCourses {
                term_id: term.into(),
                offerings: vec![offering],
            },
        )
        .unwrap()
    };
    // A 3Q–4Q offering added outside its quarters stays in the quarter on screen.
    let outside = add("2026-2Q", offering("202630030", "CCC.C300", &["3Q", "4Q"]));
    assert_eq!(placed_terms(&outside, "2026-202630030"), vec!["2026-2Q"]);

    add("2026-3Q", offering("202630010", "AAA.A100", &["3Q"]));
    add("2026-4Q", offering("202630020", "BBB.B200", &["4Q"]));
    let before = live_plans(&service.snapshot().unwrap().workspace);
    let replaced_id = before
        .iter()
        .find(|plan| plan.1 == "2026-202630010")
        .unwrap()
        .0
        .clone();
    // Added in 3Q, the 3Q–4Q offering goes into both quarters; the clashing
    // draft in 3Q leaves the timetable.
    let added = add("2026-3Q", offering("202630020", "BBB.B200", &["3Q", "4Q"]));
    assert_eq!(
        placed_terms(&added, "2026-202630020"),
        vec!["2026-3Q", "2026-4Q"]
    );
    assert!(placed_terms(&added, "2026-202630010").is_empty());
    let course = rows(&added, "courses")
        .unwrap()
        .find(|row| row["id"] == "2026-202630020")
        .unwrap();
    assert_eq!(
        course["termId"], "2026-4Q",
        "an existing course keeps its quarter"
    );

    // Undo removes the 3Q plan item the add placed, keeps the earlier 4Q
    // one, and brings back the replaced draft itself.
    let undone = run(
        &service,
        WorkspaceIntent::UndoCatalogCourses {
            term_id: "2026-3Q".into(),
            offering_ids: vec!["202630020".into()],
        },
    )
    .unwrap();
    assert_eq!(live_plans(&undone), before);
    assert!(live_plans(&undone).iter().any(|plan| plan.0 == replaced_id));
    assert!(matches!(
        run(
            &service,
            WorkspaceIntent::UndoCatalogCourses {
                term_id: "2026-3Q".into(),
                offering_ids: vec!["202630020".into()],
            },
        ),
        Err(WorkspaceError::InvalidIntent(_))
    ));

    // Off the timetable means off every quarter.
    add("2026-3Q", offering("202630020", "BBB.B200", &["3Q", "4Q"]));
    let off = run(
        &service,
        WorkspaceIntent::SetCourseOnTimetable {
            course_id: "2026-202630020".into(),
            on_timetable: false,
        },
    )
    .unwrap();
    assert!(placed_terms(&off, "2026-202630020").is_empty());

    // Deleting a catalog course removes it and its placements.
    let deleted = run(
        &service,
        WorkspaceIntent::DeleteCourseBatch {
            course_ids: vec!["2026-202630010".into()],
        },
    )
    .unwrap();
    assert!(placed_terms(&deleted, "2026-202630010").is_empty());
    assert!(rows(&deleted, "courses")
        .unwrap()
        .any(|row| row["id"] == "2026-202630010" && !is_live(row)));
}

#[test]
fn reminder_times_persist_and_invalid_settings_cannot_replace_them() {
    let service = service("reminder-times");
    service
        .store
        .put_workspace_setting(
            "userPreferences",
            json!({"notifications":{"classes":false,"assignments":true,"classLeadMinutes":30}}),
            "legacy",
        )
        .unwrap();
    let initial = service.snapshot().unwrap();
    assert_eq!(
        initial.workspace["settings"]["notifications"]["exams"],
        false
    );
    assert_eq!(
        initial.workspace["settings"]["notifications"]["examLeadMinutes"],
        30
    );
    let settings = settings_with(&initial, |settings| {
        settings["notifications"]["exams"] = json!(false);
        settings["notifications"]["examLeadMinutes"] = json!(1440);
        settings["notifications"]["assignmentLeadMinutes"] = json!([180, 30]);
    });
    let saved = service
        .mutate(WorkspaceMutation {
            expected_revision: initial.revision,
            intent: WorkspaceIntent::ReplaceSettings { settings },
        })
        .unwrap();
    assert_eq!(
        service.snapshot().unwrap().workspace["settings"]["notifications"]["assignmentLeadMinutes"],
        json!([180, 30])
    );
    let stored = service.snapshot().unwrap().workspace;
    assert_eq!(stored["settings"]["notifications"]["examLeadMinutes"], 1440);
    assert_eq!(stored["settings"]["notifications"]["exams"], false);
    for invalid_lead in [json!(0), json!(10081), json!("one day")] {
        let invalid = settings_with(&saved, |settings| {
            settings["notifications"]["examLeadMinutes"] = invalid_lead
        });
        assert!(service
            .mutate(WorkspaceMutation {
                expected_revision: saved.revision.clone(),
                intent: WorkspaceIntent::ReplaceSettings { settings: invalid },
            })
            .is_err());
        assert_eq!(
            service.snapshot().unwrap().workspace["settings"]["notifications"]["examLeadMinutes"],
            1440
        );
    }
    let invalid = settings_with(&saved, |settings| {
        settings["notifications"]["assignmentLeadMinutes"] = json!([])
    });
    assert!(service
        .mutate(WorkspaceMutation {
            expected_revision: saved.revision,
            intent: WorkspaceIntent::ReplaceSettings { settings: invalid }
        })
        .is_err());
    assert_eq!(
        service.snapshot().unwrap().workspace["settings"]["notifications"]["assignmentLeadMinutes"],
        json!([180, 30])
    );
}
