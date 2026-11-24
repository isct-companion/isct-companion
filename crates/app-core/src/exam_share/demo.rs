//! The past exams of the demo scene (`crate::demo`): a connected account
//! with no email and made-up sittings for the scene's courses, answered
//! without the ScienceTokyoExam service. Every read answers, so no panel
//! shows a load error; every document opens the scene's sample exam.

use super::cache::document_export_name;
use super::model::*;
use super::remote::{content_language, RemoteDocument, RemoteSubject, SubjectRecords};
use super::{course_subject, form, ExamError};
use crate::data::DataStore;

const WEB_URL: &str = "https://example.invalid/past-exams";
const SAMPLE_EXAM_PDF: &[u8] = include_bytes!("../demo/sample-exam.pdf");

pub(crate) fn act(action: ExamAction, store: &DataStore) -> Result<ExamAnswer, ExamError> {
    let pages = &crate::demo::scene()
        .map_err(|_| ExamError::Unavailable)?
        .past_exams;
    // Every read answers: a subject the scene has no sittings for is an
    // empty page, as the service shows one nobody has uploaded to.
    let page = |code: &str, class_id| -> Result<ExamAnswer, ExamError> {
        let records = pages
            .iter()
            .find(|page| page.subject.code == code)
            .cloned()
            .unwrap_or_else(|| SubjectRecords {
                subject: RemoteSubject {
                    code: code.to_owned(),
                    name: code.to_owned(),
                    name_en: None,
                },
                rows: Vec::new(),
                exams: Vec::new(),
            });
        Ok(ExamAnswer::Subject(records.present(
            class_id,
            content_language(store)?,
            WEB_URL.into(),
        )))
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
        ExamAction::Choices => ExamAnswer::Choices(ExamChoices {
            site: ExamSitePages {
                search: WEB_URL.into(),
                requests: WEB_URL.into(),
                library: WEB_URL.into(),
                terms: WEB_URL.into(),
                privacy: WEB_URL.into(),
            },
            ..form::choices()
        }),
        ExamAction::CourseSubject { course_id } => {
            let (codes, class_id) = course_subject(store, &course_id)?;
            let code = codes
                .iter()
                .find(|code| pages.iter().any(|page| &page.subject.code == *code))
                .or(codes.first())
                .ok_or(ExamError::NotFound)?;
            page(code, class_id)?
        }
        ExamAction::Subject { code, class_id } => page(&code, class_id)?,
        ExamAction::Search { query } => {
            let language = content_language(store)?;
            ExamAnswer::Subjects(
                pages
                    .iter()
                    .map(|page| page.subject.clone().present(language))
                    .filter(|subject| {
                        subject.code.contains(&query) || subject.name.contains(&query)
                    })
                    .collect(),
            )
        }
        ExamAction::Open { id, .. } => {
            let document = document(pages, &id)?;
            // Under the past-exam files' root, where the shells' open and
            // save-copy actions look for them.
            let folder = store.paths().root().join("exam-share").join("demo");
            std::fs::create_dir_all(&folder).map_err(|_| ExamError::Storage)?;
            let path = folder.join(format!("{id}.pdf"));
            std::fs::write(&path, SAMPLE_EXAM_PDF).map_err(|_| ExamError::Storage)?;
            let viewer = crate::file_preview::viewer_of_file(
                &path,
                &document.file_name,
                Some(&document.file_type),
            );
            ExamAnswer::Document(ExamLocalDocument {
                export_name: document_export_name(document)?,
                document: document.clone().present(content_language(store)?),
                path: path.to_string_lossy().into_owned(),
                downloaded: true,
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
        }),
        ExamAction::Requests { .. } => ExamAnswer::Requests(ExamRequestPage {
            items: Vec::new(),
            cursor: None,
        }),
        _ => return Err(ExamError::Unavailable),
    })
}

fn document<'a>(pages: &'a [SubjectRecords], id: &str) -> Result<&'a RemoteDocument, ExamError> {
    pages
        .iter()
        .flat_map(|page| &page.exams)
        .find(|document| document.id == id)
        .ok_or(ExamError::NotFound)
}

/// The name a saved copy of a scene document takes.
pub(crate) fn export_name(id: &str) -> Result<String, ExamError> {
    let pages = &crate::demo::scene()
        .map_err(|_| ExamError::Unavailable)?
        .past_exams;
    document_export_name(document(pages, id)?)
}
