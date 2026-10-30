use super::*;

/// What a test reads back around a write: the revision the next intent is
/// made against, every workspace field (`DataStore::load_reads`) and the
/// commit sequence, read one after another.
#[derive(Clone)]
pub(crate) struct ReadBack {
    pub revision: String,
    pub workspace: Arc<Value>,
    pub school: Arc<crate::schedule::SchoolReads>,
    pub seq: u64,
}

// Only for `unwrap_err`: the fields that tell two reads apart.
impl std::fmt::Debug for ReadBack {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReadBack")
            .field("revision", &self.revision)
            .field("seq", &self.seq)
            .finish_non_exhaustive()
    }
}

impl WorkspaceService {
    pub(crate) fn read_back(&self) -> Result<ReadBack, WorkspaceError> {
        let revision = self.revision()?;
        let (workspace, school) = self.store.load_reads()?;
        Ok(ReadBack {
            revision,
            workspace,
            school,
            seq: self.store.commit_seq(),
        })
    }

    /// Commits `mutation`, then reads back what it left.
    pub(crate) fn apply(&self, mutation: WorkspaceMutation) -> Result<ReadBack, WorkspaceError> {
        self.mutate_commit(mutation)?;
        self.read_back()
    }
}

fn service(name: &str) -> WorkspaceService {
    let path = std::env::temp_dir().join(format!("isct-workspace-{name}-{}", Uuid::new_v4()));
    WorkspaceService::new(DataStore::initialize_dir(path).unwrap().0)
}

fn settings_with(snapshot: &ReadBack, edit: impl FnOnce(&mut Value)) -> Value {
    let mut settings = snapshot.workspace["settings"].clone();
    edit(&mut settings);
    settings
}

#[test]
fn stale_revision_cannot_overwrite_a_newer_settings_change() {
    let service = service("conflict");
    let original = service.read_back().unwrap();
    let changed = service
        .apply(WorkspaceMutation {
            expected_revision: original.revision.clone(),
            intent: WorkspaceIntent::ReplaceSettings {
                settings: settings_with(&original, |s| {
                    s["appearance"] = json!("dark");
                    s["theme"] = json!("paper");
                }),
            },
        })
        .unwrap();
    assert_ne!(changed.revision, original.revision);
    assert_eq!(changed.workspace["settings"]["appearance"], "dark");
    // The background theme is a user preference and reads back as stored.
    assert_eq!(changed.workspace["settings"]["theme"], "paper");

    let error = service
        .apply(WorkspaceMutation {
            expected_revision: original.revision.clone(),
            intent: WorkspaceIntent::ReplaceSettings {
                settings: settings_with(&original, |s| s["density"] = json!("compact")),
            },
        })
        .unwrap_err();
    assert!(matches!(error, WorkspaceError::Conflict(_)));
    assert_eq!(
        service.read_back().unwrap().workspace["settings"]["density"],
        changed.workspace["settings"]["density"]
    );
}

#[test]
fn cached_settings_follow_external_commits_and_core_validates_every_value() {
    let service = service("settings");
    let original = service.read_back().unwrap();
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
    let warm = service.read_back().unwrap();
    let external = DataStore::initialize_dir(service.store.directory().to_path_buf())
        .unwrap()
        .0;
    external.record_reference_poll().unwrap();
    let polled = service.read_back().unwrap();
    let checked = polled.workspace["settings"]["subscriptionRefresh"]["lastCheckedAt"].clone();
    assert!(checked.is_string());
    assert_eq!(polled.revision, warm.revision);
    assert_ne!(
        warm.workspace["settings"]["subscriptionRefresh"]["lastCheckedAt"],
        checked
    );

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

    let before = service.read_back().unwrap();
    for (pointer, value) in [
        ("/locale", json!("xx")),
        ("/appearance", json!("blue")),
        ("/theme", json!("neon")),
        ("/textSize", json!("huge")),
        ("/contentLanguage", json!("fr")),
        ("/density", json!(3)),
        ("/closeWindow", json!("minimize")),
        ("/cloud/provider", json!("dropbox")),
    ] {
        let error = service
            .apply(WorkspaceMutation {
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
            .apply(WorkspaceMutation {
                expected_revision: before.revision.clone(),
                intent,
            })
            .unwrap_err();
        assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
    }
    assert!(matches!(
        service
            .store
            .put_workspace_setting("devicePreferences", json!({"locale":"xx"})),
        Err(DataError::InvalidInput(_))
    ));
    assert_eq!(service.read_back().unwrap().revision, before.revision);
    let mut invalid_periods = crate::schedule::default_teaching_periods();
    invalid_periods[1].start_time = "08:00".into();
    let mut profile = before.workspace["academicProfile"].clone();
    profile["teachingPeriods"] = serde_json::to_value(invalid_periods).unwrap();
    let error = service
        .apply(WorkspaceMutation {
            expected_revision: before.revision.clone(),
            intent: WorkspaceIntent::EditAcademicCalendar {
                academic_profile: profile,
                exceptions: vec![],
            },
        })
        .unwrap_err();
    assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
    assert_eq!(service.read_back().unwrap().revision, before.revision);
    let year = mutate(
        &service,
        WorkspaceIntent::SetAdmissionYear {
            admission_year: Some(2024),
        },
    );
    assert_eq!(year.workspace["academicProfile"]["admissionYear"], 2024);

    // The close prompt remembers a choice without a settings form; the shell
    // reads it back as a device preference that cloud sync never carries.
    let remembered = mutate(
        &service,
        WorkspaceIntent::SetCloseWindow {
            action: "background".into(),
        },
    );
    assert_eq!(
        remembered.workspace["settings"]["closeWindow"],
        "background"
    );
    assert_eq!(remembered.workspace["settings"]["textSize"], "large");
    assert_eq!(
        service.store.device_preference("/closeWindow").unwrap(),
        Some(json!("background"))
    );
    let mut synced = json!({"key":"userPreferences","value":{"closeWindow":"quit"}});
    crate::data::cloud_merge::sanitize_setting(&mut synced);
    assert_eq!(synced["value"], json!({}));
    let error = service
        .apply(WorkspaceMutation {
            expected_revision: remembered.revision.clone(),
            intent: WorkspaceIntent::SetCloseWindow {
                action: "minimize".into(),
            },
        })
        .unwrap_err();
    assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
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
        .put_workspace_setting("academicProfile", profile.clone())
        .unwrap();
    let old = service.read_back().unwrap();
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
    let before = service.read_back().unwrap();
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
            .apply(WorkspaceMutation {
                expected_revision: before.revision.clone(),
                intent: WorkspaceIntent::SetStudyProfile {
                    admission_year: Some(2026),
                    study_enrollments,
                },
            })
            .unwrap_err();
        assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
        let after = service.read_back().unwrap();
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
    let original = service.read_back().unwrap();
    let error = service
        .apply(WorkspaceMutation {
            expected_revision: original.revision.clone(),
            intent: WorkspaceIntent::SaveEvent {
                draft: json!({"title":"Missing required event fields"}),
                term_id: "2026-3Q".into(),
                existing_id: None,
            },
        })
        .unwrap_err();
    assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
    let after = service.read_back().unwrap();
    assert_eq!(after.revision, original.revision);
    assert!(after.workspace["events"].as_array().unwrap().is_empty());
}

#[test]
fn weekly_events_are_normalized_validated_atomically_and_copied() {
    let service = service("event-validation");
    let original = service.read_back().unwrap();
    // Fields of the old one-date and conflict-toggle editor are dropped.
    let saved = service
        .apply(WorkspaceMutation {
            expected_revision: original.revision,
            intent: WorkspaceIntent::SaveEvent {
                draft: json!({
                    "title":" Lab meeting ", "days":[3,1,3], "startTime":"13:30",
                    "endTime":"15:10", "date":"2026-10-14", "blocking":true
                }),
                term_id: "2026-3Q".into(),
                existing_id: None,
            },
        })
        .unwrap();
    let event = &saved.workspace["events"][0];
    assert_eq!(event["title"], "Lab meeting");
    assert_eq!(event["room"], "");
    assert_eq!(event["days"], json!([1, 3]));
    assert_eq!(event["termId"], "2026-3Q");
    assert!(event.get("date").is_none() && event.get("blocking").is_none());
    // Without a choice Core picks a palette colour by title and stores it.
    let color = event["color"].as_str().unwrap();
    assert!(crate::schedule::COURSE_PALETTE.contains(&color));
    assert_eq!(
        color,
        crate::schedule::event_color(&json!({"title":"Lab meeting"}))
    );

    for draft in [
        json!({"title":"No days", "days":[], "startTime":"13:30", "endTime":"15:10"}),
        json!({"title":"Day eight", "days":[8], "startTime":"13:30", "endTime":"15:10"}),
        json!({"title":"Backwards", "days":[1], "startTime":"15:10", "endTime":"13:30"}),
        json!({"title":"  ", "days":[1], "startTime":"13:30", "endTime":"15:10"}),
        json!({"title":"Red", "days":[1], "startTime":"13:30", "endTime":"15:10", "color":"red"}),
    ] {
        let before = service.read_back().unwrap();
        let error = service
            .apply(WorkspaceMutation {
                expected_revision: before.revision.clone(),
                intent: WorkspaceIntent::SaveEvent {
                    draft,
                    term_id: "2026-3Q".into(),
                    existing_id: None,
                },
            })
            .unwrap_err();
        assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
        assert_eq!(service.read_back().unwrap().revision, before.revision);
    }

    let before = service.read_back().unwrap();
    let copied = service
        .apply(WorkspaceMutation {
            expected_revision: before.revision,
            intent: WorkspaceIntent::CopyEvents {
                from_term_id: "2026-3Q".into(),
                term_id: "2026-4Q".into(),
            },
        })
        .unwrap();
    let events = copied.workspace["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    let copy = events
        .iter()
        .find(|row| row["termId"] == "2026-4Q")
        .unwrap();
    assert_ne!(copy["id"], event["id"]);
    assert_eq!(
        (&copy["title"], &copy["days"], &copy["color"]),
        (&event["title"], &event["days"], &event["color"])
    );

    // A quarter without weekly events has nothing to copy.
    let error = service
        .apply(WorkspaceMutation {
            expected_revision: copied.revision.clone(),
            intent: WorkspaceIntent::CopyEvents {
                from_term_id: "2026-2Q".into(),
                term_id: "2026-4Q".into(),
            },
        })
        .unwrap_err();
    assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
}

#[test]
fn schedule_override_periods_must_name_a_class_slot() {
    let service = service("override-periods");
    let original = service.read_back().unwrap();
    for draft in [
        json!({"targetKey":"course-weekly-0-2026-10-14","targetKind":"course","periods":[5,3],"hidden":false}),
        // Exams have their own intents so a link cannot freeze published facts.
        json!({"targetKey":"exam:any","targetKind":"exam","courseId":"course","hidden":false}),
    ] {
        let error = service
            .apply(WorkspaceMutation {
                expected_revision: original.revision.clone(),
                intent: WorkspaceIntent::SaveScheduleOverride {
                    draft,
                    existing_id: None,
                },
            })
            .unwrap_err();
        assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
        assert_eq!(service.read_back().unwrap().revision, original.revision);
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
    service
        .store
        .write(None, |tx| apply_exam_feed(tx, &published))
        .unwrap();
    let first = "reference:exams:2026:undergraduate:2q:first";
    let second = "reference:exams:2026:undergraduate:2q:second";
    let view = |snapshot: &ReadBack, key: &str| {
        crate::schedule::exam_views(&snapshot.workspace, &snapshot.school, "2026-2Q")
            .into_iter()
            .find(|exam| exam.key == key)
            .unwrap()
    };
    let stored = |snapshot: &ReadBack| {
        snapshot.workspace["scheduleOverrides"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["deletedAt"].is_null())
            .cloned()
            .collect::<Vec<_>>()
    };
    let before = service.read_back().unwrap();
    assert!(view(&before, first).auto_linked);
    // Linking reports the course's other exam but no longer blocks.
    let preview =
        crate::schedule::exam_association(&before.workspace, &before.school, second, &course_id)
            .unwrap();
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
        crate::schedule::course_detail(&linked.workspace, &linked.school, &course_id)
            .exam_ids
            .len(),
        2
    );

    published.feed.entries[1].room =
        Some(serde_json::from_value(json!({"ja":"新教室","en":"Updated source room"})).unwrap());
    service
        .store
        .write(None, |tx| apply_exam_feed(tx, &published))
        .unwrap();
    assert_eq!(
        view(&service.read_back().unwrap(), second).room,
        "Updated source room"
    );

    // The form submits every field; only the room differs from the source.
    let exam = view(&service.read_back().unwrap(), second);
    let edited = mutate(
        &service,
        WorkspaceIntent::EditExam {
            exam_key: second.into(),
            date: Some(exam.date.clone()),
            start_time: exam.start_time.clone(),
            end_time: exam.end_time.clone(),
            room: Some("Personal room".into()),
        },
    );
    let row = &stored(&edited)[0];
    assert_eq!(row["room"], "Personal room");
    assert_eq!(row["courseId"], course_id.as_str());
    assert!(row.get("date").is_none() && row.get("startTime").is_none());
    assert_eq!(view(&edited, second).periods, Some([3, 4]));

    published.feed.entries[1].date = "2026-08-05".into();
    service
        .store
        .write(None, |tx| apply_exam_feed(tx, &published))
        .unwrap();
    let refreshed = view(&service.read_back().unwrap(), second);
    assert_eq!(
        (
            refreshed.date.as_str(),
            refreshed.room.as_str(),
            refreshed.edited
        ),
        ("2026-08-05", "Personal room", true)
    );
    assert_eq!(refreshed.source.room, "Updated source room");

    let original = service.read_back().unwrap();
    for (date, start, end) in [
        ("2026-02-30", None, None),
        ("2026-08-05", Some("13:30"), None),
        ("2026-08-05", Some("25:00"), Some("26:00")),
        ("2026-08-05", Some("13:30"), Some("10:30")),
    ] {
        assert!(service
            .apply(WorkspaceMutation {
                expected_revision: original.revision.clone(),
                intent: WorkspaceIntent::EditExam {
                    exam_key: second.into(),
                    date: Some(date.into()),
                    start_time: start.map(Into::into),
                    end_time: end.map(Into::into),
                    room: None,
                },
            })
            .is_err());
        assert_eq!(service.read_back().unwrap().revision, original.revision);
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
    // Deleting a published exam from the schedule unlinks it from the course.
    let unlinked = mutate(
        &service,
        WorkspaceIntent::RemoveExam {
            exam_key: first.into(),
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
    let commit = service
        .mutate_commit(WorkspaceMutation {
            expected_revision: service.read_back().unwrap().revision,
            intent: WorkspaceIntent::MarkSession {
                key: second.into(),
                mark: "cancelled".into(),
            },
        })
        .unwrap();
    let marked = service.read_back().unwrap();
    assert_eq!(view(&marked, second).status, "cancelled");
    // The desktop window hears only what the mark changed, and the revision
    // its next intent is made against.
    assert_eq!(commit.changes, [WorkspaceChange::Calendar]);
    assert_eq!(commit.revision, marked.revision);
    assert_eq!(service.revision().unwrap(), marked.revision);
    let edited = mutate(
        &service,
        WorkspaceIntent::EditExam {
            exam_key: second.into(),
            date: None,
            start_time: None,
            end_time: None,
            room: Some("Mark room".into()),
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
    let original = service.read_back().unwrap();
    let error = service
        .apply(WorkspaceMutation {
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
    assert_eq!(service.read_back().unwrap().revision, original.revision);
}

fn mutate(service: &WorkspaceService, intent: WorkspaceIntent) -> ReadBack {
    if let WorkspaceIntent::AddCatalogCourses { offerings, .. } = &intent {
        service.store.catalog_upsert_offerings(offerings).unwrap();
    }
    let expected_revision = service.read_back().unwrap().revision;
    service
        .apply(WorkspaceMutation {
            expected_revision,
            intent,
        })
        .unwrap()
}

fn course<'a>(snapshot: &'a ReadBack, id: &str) -> &'a Value {
    snapshot.workspace["courses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|course| course["id"] == id)
        .unwrap()
}

/// A phone and a Mac each made an override row for one course, and sync
/// brought the phone's into the Mac without its key columns. Marks made on
/// the Mac then went to the Mac's own row while the course read the phone's
/// (the first by id): the toast said the lesson was recorded and the rail
/// never moved.
#[test]
fn a_lesson_mark_shows_after_sync_brings_another_devices_override_row() {
    let offering = json!({
        "id":"202600002", "year":2026, "quarters":["3Q"],
        "code":"CSC.T364", "title":{"en":"Systems"},
        "instructors":[{"en":"Teacher One"}],
        "syllabusUrl":"https://syllabus.s.isct.ac.jp/courses/202600002",
        "credits":{"lecture":2,"exercise":0,"lab":0},
        "instructionLanguage":"ja", "format":{"kinds":["lecture"]}, "media":{},
        "academicUnit":{}, "intensive":false, "fetchedAt":{},
        "schedule":[{"kind":"weekly","day":1,"periods":[1,2]}]
    });
    let add = |service: &WorkspaceService| {
        let added = mutate(
            service,
            WorkspaceIntent::AddCatalogCourses {
                term_id: "2026-3Q".into(),
                offerings: vec![offering.clone()],
            },
        );
        added.workspace["courses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|course| course["offeringId"] == "202600002")
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let mark = |service: &WorkspaceService, course_id: &str, occurrence: &str, section_id| {
        mutate(
            service,
            WorkspaceIntent::RecordLesson {
                course_id: course_id.into(),
                occurrence: occurrence.into(),
                section_id,
            },
        )
    };
    let last_mark = |snapshot: &ReadBack, course_id: &str| {
        course(snapshot, course_id)["overrides"]["lessonLog"]
            .as_array()
            .and_then(|log| log.last())
            .cloned()
            .unwrap()
    };
    let phone = service("lesson-mark-phone");
    let mac = service("lesson-mark-mac");
    // Each device has its own id for the course; sync maps rows by its key.
    mark(&phone, &add(&phone), "2026-10-05T08:50:00+09:00", 101);
    let course_id = add(&mac);
    mark(&mac, &course_id, "2026-10-12T08:50:00+09:00", 102);
    mac.store
        .merge_cloud_data(
            &phone.store.export_document(json!([])).unwrap(),
            crate::data::WorkspaceChangeSource::CloudSync,
        )
        .unwrap();
    // Two live rows now name the course; the Mac's later mark wins.
    assert_eq!(
        mac.store
            .user_rows("course_overrides")
            .unwrap()
            .iter()
            .filter(|row| row.get("deletedAt").is_none())
            .count(),
        2
    );
    assert_eq!(
        last_mark(&mac.read_back().unwrap(), &course_id)["sectionId"],
        102
    );

    let marked = mark(&mac, &course_id, "2026-10-19T08:50:00+09:00", 103);
    let latest = last_mark(&marked, &course_id);
    assert_eq!(latest["occurrence"], "2026-10-19T08:50:00+09:00");
    assert_eq!(latest["sectionId"], 103);
    for service in [phone, mac] {
        let directory = service.store.directory().to_path_buf();
        drop(service);
        let _ = std::fs::remove_dir_all(directory);
    }
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
    let original = service.read_back().unwrap();
    let next = service
            .apply(WorkspaceMutation {
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
    let before_language = service.read_back().unwrap();
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
    let original = service.read_back().unwrap();
    let created = service
        .apply(WorkspaceMutation {
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
        .apply(WorkspaceMutation {
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
    let original = service.read_back().unwrap();
    let task_id = original.workspace["scheduledTasks"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let next = service
        .apply(WorkspaceMutation {
            expected_revision: original.revision.clone(),
            intent: WorkspaceIntent::RecordScheduledTaskRun {
                task_id: task_id.clone(),
                occurrence_at: "2026-09-29T12:00:00.000Z".into(),
                run_at: "2026-09-29T12:00:01.000Z".into(),
                result: "Export completed".into(),
            },
        })
        .unwrap();
    // Run bookkeeping is Core-owned: it commits, but does not move the
    // baseline user edits are checked against.
    assert!(next.seq > original.seq);
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
    assert_eq!(service.read_back().unwrap().workspace, next.workspace);

    // A run claim with a malformed clock, or for a task that is gone, is
    // rejected before any row changes.
    let before = service.read_back().unwrap();
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
            .apply(WorkspaceMutation {
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
        let after = service.read_back().unwrap();
        assert_eq!(after.revision, before.revision);
        assert_eq!(after.seq, before.seq);
        assert_eq!(after.workspace["scheduledTasks"], tasks_before);
    }
}

#[test]
fn background_ingest_and_bookkeeping_do_not_conflict_with_user_edits() {
    let service = service("ingest-revision");
    let before = service.read_back().unwrap();
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
        .put_user_record_checked(
            "scheduled_tasks",
            task,
            crate::data::WorkspaceChangeSource::UserIntent,
            rev,
        )
        .unwrap();
    let ingested = service.read_back().unwrap();
    assert!(ingested.seq > before.seq);
    assert_eq!(ingested.revision, before.revision);

    let edited = service
        .apply(WorkspaceMutation {
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
        .apply(WorkspaceMutation {
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
    let original = service.read_back().unwrap();
    let created = service
            .apply(WorkspaceMutation {
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
        .apply(WorkspaceMutation {
            expected_revision: created.revision.clone(),
            intent: WorkspaceIntent::SaveCalendarException {
                draft: json!({"date":"2026-10-13","type":"holiday","label":"Duplicate"}),
                existing_id: None,
            },
        })
        .unwrap_err();
    assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
    assert_eq!(service.read_back().unwrap().revision, created.revision);
}

fn run(service: &WorkspaceService, intent: WorkspaceIntent) -> Result<Value, WorkspaceError> {
    if let WorkspaceIntent::AddCatalogCourses { offerings, .. } = &intent {
        service.store.catalog_upsert_offerings(offerings).unwrap();
    }
    let expected_revision = service.read_back()?.revision;
    service
        .apply(WorkspaceMutation {
            expected_revision,
            intent,
        })
        .map(|snapshot| Value::clone(&snapshot.workspace))
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
    let before = service.read_back().unwrap();
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
    let rejected = service.read_back().unwrap();
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
    let before = service.read_back().unwrap();

    // The requested 4Q is a master's term, but the same offering would
    // also be placed into undergraduate 3Q and must be rejected there.
    let mut submitted = canonical;
    submitted["code"] = json!("CSC.T101");
    let error = service
        .apply(WorkspaceMutation {
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
    let after = service.read_back().unwrap();
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
    // Courses are named by their internal id, minted when first added.
    let id = |offering: &str| -> String {
        rows(&service.read_back().unwrap().workspace, "courses")
            .unwrap()
            .find(|course| course["offeringId"] == offering)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    // A 3Q–4Q offering added outside its quarters stays in the quarter on screen.
    let outside = add("2026-2Q", offering("202630030", "CCC.C300", &["3Q", "4Q"]));
    assert_eq!(placed_terms(&outside, &id("202630030")), vec!["2026-2Q"]);

    add("2026-3Q", offering("202630010", "AAA.A100", &["3Q"]));
    add("2026-4Q", offering("202630020", "BBB.B200", &["4Q"]));
    let before = live_plans(&service.read_back().unwrap().workspace);
    let replaced_id = before
        .iter()
        .find(|plan| plan.1 == id("202630010"))
        .unwrap()
        .0
        .clone();
    // Added in 3Q, the 3Q–4Q offering goes into both quarters; the clashing
    // draft in 3Q leaves the timetable.
    let added = add("2026-3Q", offering("202630020", "BBB.B200", &["3Q", "4Q"]));
    assert_eq!(
        placed_terms(&added, &id("202630020")),
        vec!["2026-3Q", "2026-4Q"]
    );
    assert!(placed_terms(&added, &id("202630010")).is_empty());
    let course = rows(&added, "courses")
        .unwrap()
        .find(|row| row["id"] == id("202630020"))
        .unwrap();
    assert_eq!(
        course["termId"], "2026-4Q",
        "an existing course keeps its quarter"
    );
    // The snapshot reads the offering an added course names from the catalog.
    let snapshot = service.read_back().unwrap();
    let detail =
        crate::schedule::course_detail(&snapshot.workspace, &snapshot.school, &id("202630020"));
    assert_eq!(detail.format.unwrap().kinds, ["lecture"]);

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
            course_id: id("202630020"),
            on_timetable: false,
        },
    )
    .unwrap();
    assert!(placed_terms(&off, &id("202630020")).is_empty());

    // Deleting a catalog course removes it and its placements.
    let deleted = run(
        &service,
        WorkspaceIntent::DeleteCourseBatch {
            course_ids: vec![id("202630010")],
        },
    )
    .unwrap();
    assert!(placed_terms(&deleted, &id("202630010")).is_empty());
    assert!(rows(&deleted, "courses")
        .unwrap()
        .any(|row| row["id"] == id("202630010") && !is_live(row)));
}

#[test]
fn undo_waits_for_the_catalog_add_receipt_after_its_database_commit() {
    use crate::data::{CommitObserver, WorkspaceChangeSource};
    use std::{collections::BTreeSet, sync::mpsc, time::Duration};

    // Pause the first commit at the real event boundary, before mutate_commit
    // publishes its in-memory undo receipt. A second client may already read
    // that commit from SQLite and request its undo.
    struct PauseCommit(Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>);
    impl CommitObserver for PauseCommit {
        fn committed(&self, _: WorkspaceChangeSource, _: Option<String>, _: &BTreeSet<String>) {
            let gate = self.0.lock().unwrap().take();
            if let Some((entered, release)) = gate {
                entered.send(()).unwrap();
                release.recv_timeout(Duration::from_secs(5)).unwrap();
            }
        }
    }

    let service = service("concurrent-catalog-undo");
    let directory = service.store.directory().to_path_buf();
    let course = offering("202630010", "AAA.A100", &["3Q"]);
    service
        .store
        .catalog_upsert_offerings(std::slice::from_ref(&course))
        .unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    service.store.observe(Arc::new(PauseCommit(Mutex::new(Some((
        entered_tx, release_rx,
    ))))));
    std::thread::scope(|scope| {
        let add = scope.spawn(|| {
            run(
                &service,
                WorkspaceIntent::AddCatalogCourses {
                    term_id: "2026-3Q".into(),
                    offerings: vec![course],
                },
            )
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let revision = service.revision().unwrap();
        let (finished_tx, finished_rx) = mpsc::channel();
        let current = &service;
        let undo = scope.spawn(move || {
            let result = current.mutate_commit(WorkspaceMutation {
                expected_revision: revision,
                intent: WorkspaceIntent::UndoCatalogCourses {
                    term_id: "2026-3Q".into(),
                    offering_ids: vec!["202630010".into()],
                },
            });
            finished_tx.send(()).unwrap();
            result
        });
        // Give the competing request a chance to reach the paused commit.
        // Correct code waits for its receipt; the result, not timing, is asserted.
        let _ = finished_rx.recv_timeout(Duration::from_millis(100));
        release_tx.send(()).unwrap();
        add.join().unwrap().unwrap();
        undo.join().unwrap().unwrap();
    });
    assert!(live_plans(&service.read_back().unwrap().workspace).is_empty());
    drop(service);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn reminder_times_persist_and_invalid_settings_cannot_replace_them() {
    let service = service("reminder-times");
    service
        .store
        .put_workspace_setting(
            "userPreferences",
            json!({"notifications":{"classes":false,"assignments":true,"classLeadMinutes":30}}),
        )
        .unwrap();
    let initial = service.read_back().unwrap();
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
        .apply(WorkspaceMutation {
            expected_revision: initial.revision,
            intent: WorkspaceIntent::ReplaceSettings { settings },
        })
        .unwrap();
    assert_eq!(
        service.read_back().unwrap().workspace["settings"]["notifications"]
            ["assignmentLeadMinutes"],
        json!([180, 30])
    );
    let stored = service.read_back().unwrap().workspace;
    assert_eq!(stored["settings"]["notifications"]["examLeadMinutes"], 1440);
    assert_eq!(stored["settings"]["notifications"]["exams"], false);
    for invalid_lead in [json!(0), json!(10081), json!("one day")] {
        let invalid = settings_with(&saved, |settings| {
            settings["notifications"]["examLeadMinutes"] = invalid_lead
        });
        assert!(service
            .apply(WorkspaceMutation {
                expected_revision: saved.revision.clone(),
                intent: WorkspaceIntent::ReplaceSettings { settings: invalid },
            })
            .is_err());
        assert_eq!(
            service.read_back().unwrap().workspace["settings"]["notifications"]["examLeadMinutes"],
            1440
        );
    }
    let invalid = settings_with(&saved, |settings| {
        settings["notifications"]["assignmentLeadMinutes"] = json!([])
    });
    assert!(service
        .apply(WorkspaceMutation {
            expected_revision: saved.revision,
            intent: WorkspaceIntent::ReplaceSettings { settings: invalid }
        })
        .is_err());
    assert_eq!(
        service.read_back().unwrap().workspace["settings"]["notifications"]
            ["assignmentLeadMinutes"],
        json!([180, 30])
    );
}
