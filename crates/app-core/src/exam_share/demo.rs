//! The past exams of the demo scene (`crate::demo`): a connected account
//! with no email and made-up sittings for the scene's courses, answered
//! without the ScienceTokyoExam service. Every read answers, so no panel
//! shows a load error; every document opens the scene's sample exam.

use super::model::*;
use super::{course_subject, form, ExamError};
use crate::data::DataStore;

const WEB_URL: &str = "https://example.invalid/past-exams";

pub(crate) fn act(action: ExamAction, store: &DataStore) -> Result<ExamAnswer, ExamError> {
    let pages = &crate::demo::scene()
        .map_err(|_| ExamError::Unavailable)?
        .past_exams;
    // Every read answers: a subject the scene has no sittings for is an
    // empty page, as the service shows one nobody has uploaded to.
    let page = |code: &str| {
        ExamAnswer::Subject(
            pages
                .iter()
                .find(|page| page.subject.code == code)
                .cloned()
                .unwrap_or_else(|| ExamSubjectPage {
                    subject: ExamSubject {
                        code: code.to_owned(),
                        name: code.to_owned(),
                    },
                    rows: Vec::new(),
                    class_id: None,
                    web_url: WEB_URL.into(),
                    sets: Vec::new(),
                }),
        )
    };
    Ok(match action {
        ExamAction::Account => ExamAnswer::Account(ExamAccount {
            status: ExamAccountStatus::Connected,
            email: None,
            upload: ExamUploadStatus {
                phase: ExamUploadPhase::Idle,
                sent: 0,
                total: 0,
                cancellable: false,
            },
        }),
        ExamAction::Choices => ExamAnswer::Choices(form::choices()),
        ExamAction::CourseSubject { course_id } => {
            let (codes, _) = course_subject(store, &course_id)?;
            let code = codes
                .iter()
                .find(|code| pages.iter().any(|page| &page.subject.code == *code))
                .or(codes.first())
                .ok_or(ExamError::NotFound)?;
            page(code)
        }
        ExamAction::Subject { code, .. } => page(&code),
        ExamAction::Search { query } => ExamAnswer::Subjects(
            pages
                .iter()
                .map(|page| page.subject.clone())
                .filter(|subject| subject.code.contains(&query) || subject.name.contains(&query))
                .collect(),
        ),
        ExamAction::Open { id, .. } => {
            let document = pages
                .iter()
                .flat_map(|page| &page.sets)
                .flat_map(|set| &set.documents)
                .find(|document| document.id == id)
                .cloned()
                .ok_or(ExamError::NotFound)?;
            let folder = store.paths().root().join("demo-past-exams");
            std::fs::create_dir_all(&folder).map_err(|_| ExamError::Storage)?;
            let path = folder.join(format!("{id}.pdf"));
            std::fs::write(&path, crate::demo::SAMPLE_EXAM_PDF).map_err(|_| ExamError::Storage)?;
            let viewer = crate::file_preview::viewer_of_file(
                &path,
                &document.file_name,
                Some(&document.file_type),
            );
            ExamAnswer::Document(ExamLocalDocument {
                export_name: document.file_name.clone(),
                document,
                path: path.to_string_lossy().into_owned(),
                downloaded: false,
                saved: Some(false),
                offline: false,
                web_url: WEB_URL.into(),
                viewer,
            })
        }
        ExamAction::Library => ExamAnswer::Library(ExamLibrary {
            offline: false,
            saved: Vec::new(),
            downloaded: Vec::new(),
            submitted: Vec::new(),
            web_url: WEB_URL.into(),
        }),
        ExamAction::Requests { .. } => ExamAnswer::Requests(ExamRequestPage {
            items: Vec::new(),
            cursor: None,
            web_url: WEB_URL.into(),
        }),
        _ => return Err(ExamError::Unavailable),
    })
}
