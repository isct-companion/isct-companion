use super::*;

fn draft(role: ExamRole, occasion: ExamOccasion) -> ExamUploadDraft {
    ExamUploadDraft {
        subject_codes: vec!["LAS.B103".into()],
        teacher_ids: vec![],
        new_teachers: vec![],
        academic_year: Some(2025),
        occasion,
        occasion_number: 3,
        variant: String::new(),
        role,
        provenance: ExamProvenance::Original,
        answer_source: None,
        note: String::new(),
        request_id: None,
    }
}

/// The API accepts an occasion only for exam materials, a number only for
/// numbered occasions and an answer source only for answers; the forms keep
/// a draft that way and an upload is checked against the API's limits.
#[test]
fn a_draft_follows_its_role_and_the_api_limits() {
    let mut notes = draft(ExamRole::Notes, ExamOccasion::Quiz);
    upload::validate(&mut notes).unwrap();
    assert_eq!(
        (notes.occasion, notes.occasion_number, notes.answer_source),
        (ExamOccasion::NotApplicable, 0, None)
    );

    let mut answer = draft(ExamRole::Answer, ExamOccasion::NotApplicable);
    upload::validate(&mut answer).unwrap();
    assert_eq!(
        (
            answer.occasion,
            answer.occasion_number,
            answer.answer_source
        ),
        (ExamOccasion::Unknown, 0, Some(ExamAnswerSource::Unknown))
    );

    let mut quiz = draft(ExamRole::Question, ExamOccasion::Quiz);
    upload::validate(&mut quiz).unwrap();
    assert_eq!(quiz.occasion_number, 3);

    let mut crowded = draft(ExamRole::Question, ExamOccasion::Final);
    crowded.subject_codes = (0..=form::limits().subjects)
        .map(|n| format!("LAS.B1{n:02}"))
        .collect();
    assert!(matches!(
        upload::validate(&mut crowded),
        Err(ExamError::InvalidInput)
    ));
    let mut long_note = draft(ExamRole::Question, ExamOccasion::Final);
    long_note.note = "x".repeat(41);
    assert!(matches!(
        upload::validate(&mut long_note),
        Err(ExamError::UploadNoteLength)
    ));
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
fn upload_readiness_validates_staged_bytes_and_the_actual_draft() {
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
    let mut draft = draft(ExamRole::Question, ExamOccasion::Final);
    let ready = |draft: &ExamUploadDraft, files: &[ExamUploadFile], reason: Option<&str>| {
        assert_eq!(upload_readiness(draft, files).reason_key.as_deref(), reason);
    };
    ready(&draft, &[], Some("error.exam-share-upload-no-files"));
    ready(
        &draft,
        &[photo.clone(), document],
        Some("error.exam-share-upload-mixed"),
    );
    ready(&draft, &[photo.clone(), photo.clone()], None);
    draft.new_teachers.push(ExamNewTeacher {
        family_name: " ".into(),
        given_name: "A".into(),
    });
    ready(
        &draft,
        std::slice::from_ref(&photo),
        Some("error.exam-share-upload-teacher-name"),
    );
    draft.new_teachers[0].family_name = "𠮷".repeat(51);
    ready(
        &draft,
        std::slice::from_ref(&photo),
        Some("error.exam-share-upload-teacher-length"),
    );
    draft.new_teachers[0].family_name = "𠮷".repeat(50);
    draft.variant = "𠮷".repeat(51);
    ready(
        &draft,
        std::slice::from_ref(&photo),
        Some("error.exam-share-upload-variant-length"),
    );
    draft.variant.clear();
    draft.note = "𠮷".repeat(21);
    ready(
        &draft,
        &[photo],
        Some("error.exam-share-upload-note-length"),
    );
    std::fs::remove_dir_all(folder).unwrap();
}
