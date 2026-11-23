//! Records used by the service, download cache, and demo scene:
//! subject names in both languages. Shells never see these types; `present`
//! builds the read models with each name already in the content language, so
//! no answer can leave one unchosen (`approach/content-language-in-core`).

use super::model::{ExamClass, ExamDocument, ExamSubject, ExamSubjectPage, ExamTeacher};
use super::sets;
use super::ExamError;
use crate::data::DataStore;
use crate::locale::{ContentLanguage, Localized};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// The content language the answer to this action is built in.
pub(super) fn content_language(store: &DataStore) -> Result<ContentLanguage, ExamError> {
    let (workspace, _, _) = store
        .view(&["settings"].into())
        .map_err(|_| ExamError::Storage)?;
    Ok(ContentLanguage::of(&workspace))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RemoteSubject {
    pub(super) code: String,
    pub(super) name: String,
    pub(super) name_en: Option<String>,
}

impl RemoteSubject {
    pub(super) fn present(self, language: ContentLanguage) -> ExamSubject {
        let names = Localized {
            ja: Some(self.name),
            en: self.name_en,
        };
        ExamSubject {
            name: names.pick(language).unwrap_or(&self.code).to_owned(),
            code: self.code,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RemoteDocument {
    pub(super) id: String,
    subjects: Vec<RemoteSubject>,
    pub(super) teachers: Vec<ExamTeacher>,
    academic_year: Option<u32>,
    occasion: Option<String>,
    occasion_number: u32,
    variant: String,
    role: Option<String>,
    note: Option<String>,
    pub(super) file_name: String,
    pub(super) file_type: String,
}

impl RemoteDocument {
    pub(super) fn present(self, language: ContentLanguage) -> ExamDocument {
        ExamDocument {
            id: self.id,
            subjects: self
                .subjects
                .into_iter()
                .map(|subject| subject.present(language))
                .collect(),
            teachers: self.teachers,
            academic_year: self.academic_year,
            occasion: self.occasion,
            occasion_number: self.occasion_number,
            variant: self.variant,
            role: self.role,
            note: self.note,
            file_name: self.file_name,
            file_type: self.file_type,
            number: None,
        }
    }
}

pub(super) fn present_all(
    documents: Vec<RemoteDocument>,
    language: ContentLanguage,
) -> Vec<ExamDocument> {
    documents
        .into_iter()
        .map(|document| document.present(language))
        .collect()
}

/// Subject records before content language, class priority, and exam grouping
/// are applied.
#[derive(Clone, Deserialize)]
pub(crate) struct SubjectRecords {
    pub(super) subject: RemoteSubject,
    pub(super) rows: Vec<ExamClass>,
    pub(super) exams: Vec<RemoteDocument>,
}

impl SubjectRecords {
    /// Materials naming a teacher of the course's own class come first. The
    /// class and teachers are matched by their IDs.
    pub(super) fn present(
        self,
        class_id: Option<u32>,
        language: ContentLanguage,
        web_url: String,
    ) -> ExamSubjectPage {
        let class_id = class_id.filter(|id| has_class(&self.rows, Some(*id)));
        let ours: HashSet<String> = self
            .rows
            .iter()
            .filter(|row| class_id.is_some_and(|id| row.class_ids.contains(&id)))
            .flat_map(|row| &row.teachers)
            .map(|teacher| teacher.id.clone())
            .collect();
        let mut exams = present_all(self.exams, language);
        exams.sort_by_key(|exam| !exam.teachers.iter().any(|t| ours.contains(&t.id)));
        ExamSubjectPage {
            subject: self.subject.present(language),
            rows: self.rows,
            sets: sets::group(exams, &ours),
            class_id,
            web_url,
        }
    }
}

pub(super) fn has_class(rows: &[ExamClass], id: Option<u32>) -> bool {
    id.is_some_and(|id| rows.iter().any(|row| row.class_ids.contains(&id)))
}
