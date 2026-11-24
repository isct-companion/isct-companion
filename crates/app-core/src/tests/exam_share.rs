use super::*;
use std::collections::HashSet;

fn codes(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

/// Every code this build has words for, plus one it does not (`oral`,
/// `spam`), as the service may add codes at any time.
fn known_codes() -> form::Codes {
    form::Codes {
        upload_occasions: codes(&["final", "midterm", "quiz", "report", "other", "oral"]),
        request_occasions: codes(&["final", "midterm", "quiz", "report"]),
        numbered_occasions: codes(&["quiz", "report"]),
        roles: codes(&["question", "answer", "both", "notes", "other", "oral"]),
        provenances: codes(&["original", "reconstructed"]),
        answer_sources: codes(&["official", "student", "unknown"]),
        wanted_roles: codes(&["question", "answer"]),
        report_reasons: codes(&[
            "copyright",
            "inappropriate",
            "wrong_info",
            "wrong_answer",
            "broken_file",
            "other",
            "spam",
        ]),
    }
}

fn draft(role: ExamRole) -> ExamUploadDraft {
    ExamUploadDraft {
        subject_codes: vec!["LAS.B103".into()],
        teacher_ids: vec![],
        new_teachers: vec![],
        all_teachers: false,
        academic_year: Some(2025),
        occasion: None,
        occasion_text: String::new(),
        occasion_number: 0,
        variant: String::new(),
        role,
        role_text: String::new(),
        provenance: ExamProvenance::Original,
        answer_source: None,
        note: String::new(),
        request_id: None,
    }
}

/// The forms offer the codes the service lists that this build knows, the
/// report reasons by their snake_case API codes, and a draft keeps to what
/// its role and occasion take, as the website's form does: no occasion until
/// picked, `none` for notes, a number only when numbered, a name only for
/// `other`, an answer source only for answers (`unknown` until picked), and
/// no teachers for a material common to all.
#[test]
fn a_draft_follows_the_choices_the_service_lists() {
    let choices = form::build(known_codes(), ExamLimits::default()).unwrap();
    assert_eq!(
        choices
            .roles
            .iter()
            .map(|choice| choice.role)
            .collect::<Vec<_>>(),
        [
            ExamRole::Question,
            ExamRole::Answer,
            ExamRole::Both,
            ExamRole::Notes,
            ExamRole::Other
        ]
    );
    assert_eq!(
        choices
            .report_reasons
            .iter()
            .map(ExamReportReason::api_code)
            .collect::<Vec<_>>(),
        [
            "copyright",
            "inappropriate",
            "wrong_info",
            "wrong_answer",
            "broken_file",
            "other"
        ]
    );

    let mut upload = draft(ExamRole::Question);
    form::normalize(&mut upload, &choices);
    assert_eq!((upload.occasion, upload.answer_source), (None, None));

    upload.role = ExamRole::Answer;
    upload.occasion = Some(ExamOccasion::Quiz);
    upload.occasion_number = 3;
    form::normalize(&mut upload, &choices);
    assert_eq!(
        (
            upload.occasion,
            upload.occasion_number,
            upload.answer_source
        ),
        (Some(ExamOccasion::Quiz), 3, Some(ExamAnswerSource::Unknown))
    );

    upload.occasion = Some(ExamOccasion::Other);
    upload.occasion_text = "Mock exam".into();
    form::normalize(&mut upload, &choices);
    assert_eq!(
        (upload.occasion_number, upload.occasion_text.as_str()),
        (0, "Mock exam")
    );

    upload.role = ExamRole::Other;
    upload.role_text = "Slides".into();
    upload.answer_source = Some(ExamAnswerSource::Student);
    form::normalize(&mut upload, &choices);
    assert_eq!(
        (
            upload.occasion,
            upload.occasion_text.as_str(),
            upload.role_text.as_str(),
            upload.answer_source
        ),
        (Some(ExamOccasion::NotApplicable), "", "Slides", None)
    );

    upload.role = ExamRole::Question;
    upload.all_teachers = true;
    upload.teacher_ids = vec!["t1".into()];
    upload.new_teachers = vec![ExamNewTeacher {
        family_name: "Tanaka".into(),
        given_name: String::new(),
    }];
    form::normalize(&mut upload, &choices);
    assert_eq!(
        (
            upload.occasion,
            upload.role_text.as_str(),
            upload.teacher_ids.len(),
            upload.new_teachers.len()
        ),
        (None, "", 0, 0)
    );
}

/// A list a form needs with nothing this build knows means the service has
/// moved past this build; the forms are not offered empty pickers.
#[test]
fn choices_this_build_cannot_offer_are_refused() {
    let strips: [fn(&mut form::Codes); 5] = [
        |listed| listed.roles = codes(&["oral"]),
        |listed| listed.upload_occasions.clear(),
        |listed| listed.answer_sources.clear(),
        |listed| listed.provenances.clear(),
        |listed| listed.report_reasons = codes(&["spam"]),
    ];
    for strip in strips {
        let mut listed = known_codes();
        strip(&mut listed);
        assert!(matches!(
            form::build(listed, ExamLimits::default()),
            Err(ExamError::Unsupported)
        ));
    }
}

fn document(id: &str, role: &str, teachers: &[&str]) -> ExamDocument {
    ExamDocument {
        id: id.into(),
        subjects: vec![],
        teachers: teachers
            .iter()
            .map(|id| ExamTeacher {
                id: (*id).into(),
                name: (*id).into(),
            })
            .collect(),
        all_teachers: false,
        academic_year: Some(2025),
        occasion: Some("final".into()),
        occasion_text: String::new(),
        occasion_number: 0,
        variant: String::new(),
        role: Some(role.into()),
        role_text: String::new(),
        note: None,
        file_name: format!("{id}.pdf"),
        file_type: "application/pdf".into(),
        number: None,
    }
}

/// Documents the service pairs are one sitting and nothing else is, and
/// several of one role and role name are numbered; the page's class takes,
/// as the service decides it, a material naming one of its teachers or
/// common to all, or every material when it lists no teachers; a page with
/// no class has no sitting of its own; and a row names the teachers unless
/// they are the class's own listed ones.
#[test]
fn sittings_follow_the_service_pairs_and_the_class() {
    let mut common = document("common", "question", &[]);
    common.all_teachers = true;
    let named = |id: &str, name: &str| {
        let mut other = document(id, "other", &["t1"]);
        other.role_text = name.into();
        (Some("k4".to_owned()), other)
    };
    let exams = || {
        vec![
            (Some("k2".into()), document("other-q", "question", &["t2"])),
            (Some("k1".into()), document("q-1", "question", &["t1"])),
            (Some("k1".into()), document("a", "answer", &["t1"])),
            (Some("k1".into()), document("q-2", "question", &["t1"])),
            (None, document("unpaired", "question", &["t1"])),
            (Some("k3".into()), common.clone()),
            named("slides-1", "Slides"),
            named("drills", "Drills"),
            named("slides-2", "Slides"),
        ]
    };
    type Layout = Vec<(bool, bool, Vec<(String, Option<u32>)>)>;
    let layout = |sets: &[ExamSet]| -> Layout {
        sets.iter()
            .map(|set| {
                (
                    set.mine,
                    set.show_teachers,
                    set.documents
                        .iter()
                        .map(|document| (document.id.clone(), document.number))
                        .collect(),
                )
            })
            .collect()
    };
    let class: HashSet<String> = ["t1".to_owned()].into();
    let row = |mine, shown, documents: &[(&str, Option<u32>)]| {
        let documents = documents
            .iter()
            .map(|(id, number)| ((*id).to_owned(), *number))
            .collect();
        (mine, shown, documents)
    };
    assert_eq!(
        layout(&sets::group(exams(), Some(&class))),
        [
            row(
                true,
                false,
                &[("q-1", Some(1)), ("q-2", Some(2)), ("a", None)]
            ),
            row(true, false, &[("unpaired", None)]),
            row(true, true, &[("common", None)]),
            row(
                true,
                false,
                &[
                    ("drills", None),
                    ("slides-1", Some(1)),
                    ("slides-2", Some(2))
                ]
            ),
            row(false, true, &[("other-q", None)]),
        ]
    );
    let no_teachers = HashSet::new();
    assert!(sets::group(exams(), Some(&no_teachers))
        .iter()
        .all(|set| set.mine && set.show_teachers));
    assert!(sets::group(exams(), None)
        .iter()
        .all(|set| !set.mine && set.show_teachers));
}

/// An upload journal an earlier version wrote in a form this build cannot
/// read (1.4.0's) is dropped instead of showing an upload to recover that
/// blocks every new one.
#[test]
fn an_unreadable_upload_journal_is_dropped() {
    let folder = std::env::temp_dir().join(format!("exam-journal-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&folder).unwrap();
    let session = Session {
        saved: Some(SavedSession {
            uid: "u1".into(),
            email: "student@example.invalid".into(),
            refresh_token: String::new(),
            registered: true,
        }),
        token: None,
        status: ExamAccountStatus::Connected,
        version_check: None,
        register_attempted: false,
    };
    let journal = cache::account_folder(&folder, "u1").with_extension("upload.json");
    std::fs::write(&journal, br#"{"registration":{"occasion":"unknown"}}"#).unwrap();
    assert!(!upload::pending_upload(&folder, &session));
    assert!(!journal.exists());
    std::fs::remove_dir_all(folder).unwrap();
}

/// An occasion not picked yet is left out of what is sent, which the
/// service refuses; sent as null it would still be published as 1.4.0's
/// "unknown" occasion.
#[test]
fn an_unpicked_occasion_is_not_sent() {
    let mut upload = draft(ExamRole::Question);
    let sent = upload::service_body(&upload).unwrap();
    assert!(sent.get("occasion").is_none());
    upload.occasion = Some(ExamOccasion::Final);
    assert_eq!(upload::service_body(&upload).unwrap()["occasion"], "final");
}

/// Signed file URLs are fetched only from the storage host, over HTTPS on
/// its own port and without credentials in the URL.
#[test]
fn files_come_only_from_the_storage_host() {
    let file = |url: &str| client::is_file_url(&url::Url::parse(url).unwrap());
    assert!(file(
        "https://bucket.r2.cloudflarestorage.com/exams/u1/key?X-Amz=1"
    ));
    for refused in [
        "http://bucket.r2.cloudflarestorage.com/exams/u1/key",
        "https://bucket.r2.cloudflarestorage.com:8443/exams/u1/key",
        "https://user:pass@bucket.r2.cloudflarestorage.com/exams/u1/key",
        "https://r2.cloudflarestorage.com.example.com/exams/u1/key",
        "https://sciencetokyoexam.com/exams/u1/key",
    ] {
        assert!(!file(refused), "{refused}");
    }
}

#[test]
fn upload_readiness_checks_the_staged_bytes() {
    let folder = std::env::temp_dir().join(format!("exam-readiness-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&folder).unwrap();
    let image = folder.join("photo.png");
    image::RgbImage::new(2, 2).save(&image).unwrap();
    let pdf = folder.join("document.jpg");
    std::fs::write(&pdf, b"%PDF-1.7").unwrap();
    let file = |path: &std::path::Path, name: &str| ExamUploadFile {
        path: path.to_string_lossy().into(),
        name: name.into(),
    };
    let photo = file(&image, "misleading.pdf");
    let document = file(&pdf, "misleading.jpg");
    let ready = |files: &[ExamUploadFile], limit: Option<u64>, reason: Option<&str>| {
        assert_eq!(upload_readiness(files, limit).reason_key.as_deref(), reason);
    };
    ready(&[], None, Some("error.exam-share-upload-no-files"));
    ready(
        &[photo.clone(), document],
        None,
        Some("error.exam-share-upload-mixed"),
    );
    ready(&[photo.clone(), photo.clone()], None, None);
    ready(&[photo], Some(8), Some("error.exam-share-upload-size"));
    std::fs::remove_dir_all(folder).unwrap();
}
