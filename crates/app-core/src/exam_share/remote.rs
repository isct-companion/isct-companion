//! The service's records as it sends them and the download cache keeps them:
//! subject names in both languages. Shells never see these types; `present`
//! builds the read models with each name already in the content language, so
//! no answer can leave one unchosen (`approach/content-language-in-core`).

use super::model::{ExamDocument, ExamSubject, ExamTeacher};
use super::ExamError;
use crate::data::DataStore;
use crate::locale::{ContentLanguage, Localized};
use serde::{Deserialize, Serialize};

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
    name: String,
    name_en: Option<String>,
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
    file_name: String,
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
