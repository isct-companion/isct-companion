use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamSubject {
    pub code: String,
    pub name: String,
    pub name_en: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamTeacher {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamClass {
    pub class_ids: Vec<u32>,
    pub labels: Vec<String>,
    pub taught_in: String,
    pub teachers: Vec<ExamTeacher>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamDocument {
    pub id: String,
    pub subjects: Vec<ExamSubject>,
    pub teachers: Vec<ExamTeacher>,
    pub academic_year: Option<u32>,
    pub occasion: Option<String>,
    pub occasion_number: u32,
    pub variant: String,
    pub role: Option<String>,
    pub note: Option<String>,
    pub file_name: String,
    pub file_type: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamSubjectPage {
    pub subject: ExamSubject,
    pub rows: Vec<ExamClass>,
    /// When the page was opened from a local course whose class is one of
    /// `rows`, materials naming that class's teachers come first.
    pub exams: Vec<ExamDocument>,
    /// Every teacher the materials name, for filtering, the class's first.
    pub teachers: Vec<ExamTeacher>,
    /// The years the materials are from, newest first, for filtering.
    pub years: Vec<u32>,
    /// The occasions the materials name, in `ExamChoices` order.
    pub occasions: Vec<String>,
    /// The local course's class when one of `rows` lists it. Reopening the
    /// subject passes it back so the order survives a refresh.
    pub class_id: Option<u32>,
    /// This subject on the ScienceTokyoExam website.
    pub web_url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamRequest {
    pub id: String,
    pub subject: ExamSubject,
    pub teachers: Vec<ExamTeacher>,
    pub academic_year: Option<u32>,
    pub occasion: Option<String>,
    pub wanted_role: Option<String>,
    pub description: String,
    pub status: String,
    pub answer_exam_id: Option<String>,
    pub mine: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamRequestCursor {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub created_at: u64,
    pub id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamRequestPage {
    pub items: Vec<ExamRequest>,
    pub cursor: Option<ExamRequestCursor>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamRequestDraft {
    pub subject_code: String,
    pub class_id: Option<u32>,
    pub academic_year: Option<u32>,
    pub occasion: Option<ExamOccasion>,
    pub wanted_role: Option<ExamRole>,
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamLocalDocument {
    pub document: ExamDocument,
    pub path: String,
    pub downloaded: bool,
    pub saved: Option<bool>,
    pub offline: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamLibrary {
    pub offline: bool,
    pub saved: Vec<ExamDocument>,
    pub downloaded: Vec<ExamDocument>,
    pub submitted: Vec<ExamDocument>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ExamAccountStatus {
    Disconnected,
    NeedsConsent,
    Connected,
    /// The service needs a newer app version.
    Unsupported,
}

/// What the past-exams page shows of the account. Core sends it again as
/// `CoreEvent::ExamAccount` whenever it changes, upload progress included.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamAccount {
    pub status: ExamAccountStatus,
    pub email: Option<String>,
    pub upload: ExamUploadStatus,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ExamReportReason {
    Copyright,
    Inappropriate,
    WrongAnswer,
    BrokenFile,
    Other,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamUploadFile {
    pub path: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamNewTeacher {
    pub family_name: String,
    pub given_name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamUploadDraft {
    pub subject_codes: Vec<String>,
    pub teacher_ids: Vec<String>,
    pub new_teachers: Vec<ExamNewTeacher>,
    pub academic_year: Option<u32>,
    pub occasion: ExamOccasion,
    pub occasion_number: u32,
    pub variant: String,
    pub role: ExamRole,
    pub provenance: ExamProvenance,
    pub answer_source: Option<ExamAnswerSource>,
    pub note: String,
    pub request_id: Option<String>,
}

/// What a material is. The API names them in these camelCase words.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ExamRole {
    Question,
    Answer,
    Both,
    Notes,
    Other,
}

/// When a material was used. Notes and other materials have none
/// (`NotApplicable`, the API's `none`).
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ExamOccasion {
    Unknown,
    Final,
    Midterm,
    Makeup,
    Quiz,
    Check,
    Report,
    Exercise,
    Textbook,
    Mock,
    #[serde(rename = "none")]
    NotApplicable,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ExamProvenance {
    Original,
    Reconstructed,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ExamAnswerSource {
    Unknown,
    Official,
    Student,
}

/// One upload role and what it allows, the default first.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamRoleChoice {
    pub role: ExamRole,
    pub occasions: Vec<ExamOccasion>,
    /// Empty when the role has no answer.
    pub answer_sources: Vec<ExamAnswerSource>,
}

/// The values each past-exams form offers, in display order, the default
/// first. Forms take their pickers from here, never from a list of their own.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamChoices {
    pub roles: Vec<ExamRoleChoice>,
    /// Occasions that take a number, such as the third quiz.
    pub numbered: Vec<ExamOccasion>,
    pub provenances: Vec<ExamProvenance>,
    pub request_roles: Vec<ExamRole>,
    pub request_occasions: Vec<ExamOccasion>,
    pub report_reasons: Vec<ExamReportReason>,
    pub limits: ExamLimits,
}

/// The API's limits a form keeps to. Text lengths count UTF-16 code units.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamLimits {
    /// Subjects one upload may list.
    pub subjects: u32,
    /// Listed teachers one upload may name, and as many new ones.
    pub teachers: u32,
    pub occasion_number: u32,
    pub variant: u32,
    pub note: u32,
    pub request_description: u32,
    pub report_detail: u32,
    pub query: u32,
    /// A new teacher's family and given name, each.
    pub teacher_name: u32,
    /// The academic years an upload or a request may name.
    pub earliest_year: u32,
    pub latest_year: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamUploadSubject {
    pub subject: ExamSubject,
    pub teachers: Vec<ExamTeacher>,
    /// Whether `RemoveSubject` accepts it: not the last one, nor the
    /// subject of the request the upload answers.
    pub removable: bool,
}

/// An upload form between edits. Core keeps `draft` consistent with
/// `subjects`, so a shell renders it and sends it back with each edit.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamUploadForm {
    pub draft: ExamUploadDraft,
    pub subjects: Vec<ExamUploadSubject>,
    /// Every teacher of `subjects`, once: the form's teacher choices.
    pub teachers: Vec<ExamTeacher>,
    /// The open request this upload answers.
    pub request: Option<ExamRequest>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
// A message decoded once per call; boxing would only complicate the
// projected FFI type.
#[allow(clippy::large_enum_variant)]
pub enum ExamUploadStart {
    Subject { code: String },
    Request { request: ExamRequest },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ExamUploadEdit {
    AddSubject { code: String },
    RemoveSubject { code: String },
    Role { role: ExamRole },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ExamUploadPhase {
    Idle,
    Preparing,
    Transferring,
    Registering,
    Pending,
    Cleaning,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamUploadStatus {
    pub phase: ExamUploadPhase,
    pub sent: u32,
    pub total: u32,
    /// Whether `CancelUpload` would stop it now.
    pub cancellable: bool,
}

impl ExamReportReason {
    pub(super) fn api_code(&self) -> &'static str {
        match self {
            Self::Copyright => "copyright",
            Self::Inappropriate => "inappropriate",
            Self::WrongAnswer => "wrong_answer",
            Self::BrokenFile => "broken_file",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
// A message decoded once per call; boxing would only complicate the
// projected FFI type.
#[allow(clippy::large_enum_variant)]
pub enum ExamAction {
    /// The account, resumed from its stored session when needed.
    Account,
    /// The choices the forms offer; answered without the network.
    Choices,
    Connect,
    CancelConnect,
    AcceptTerms,
    Disconnect,
    Search {
        query: String,
    },
    Subject {
        code: String,
        /// A page's `class_id`, when refreshing it.
        class_id: Option<u32>,
    },
    CourseSubject {
        course_id: String,
    },
    Open {
        id: String,
        download: bool,
    },
    Library,
    Save {
        id: String,
        saved: bool,
    },
    /// Answers the library without the removed download.
    RemoveDownload {
        id: String,
    },
    /// Answers the library without the deleted submission.
    DeleteSubmission {
        id: String,
    },
    Report {
        id: String,
        reason: ExamReportReason,
        detail: String,
    },
    Requests {
        mine: bool,
        cursor: Option<ExamRequestCursor>,
    },
    CreateRequest {
        draft: ExamRequestDraft,
    },
    /// Answers the first page of the board it was changed on.
    SetRequestOpen {
        id: String,
        open: bool,
        mine: bool,
    },
    /// Answers the first page of the board it was deleted from.
    DeleteRequest {
        id: String,
        mine: bool,
    },
    UploadForm {
        start: ExamUploadStart,
    },
    EditUpload {
        form: ExamUploadForm,
        edit: ExamUploadEdit,
    },
    Upload {
        files: Vec<ExamUploadFile>,
        draft: ExamUploadDraft,
    },
    CancelUpload,
    RecoverUpload,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum ExamAnswer {
    Account(ExamAccount),
    Subjects(Vec<ExamSubject>),
    Subject(ExamSubjectPage),
    Document(ExamLocalDocument),
    Library(ExamLibrary),
    Saved(bool),
    Requests(ExamRequestPage),
    Choices(ExamChoices),
    UploadForm(ExamUploadForm),
    /// The upload is published; the shell reads its view again.
    Uploaded,
    Done,
}

impl ExamAnswer {
    pub fn localize_subject_names(
        &mut self,
        store: &crate::data::DataStore,
    ) -> Result<(), super::ExamError> {
        use crate::locale::{ContentLanguage, Localized};

        let subjects: Vec<&mut ExamSubject> = match self {
            Self::Subjects(subjects) => subjects.iter_mut().collect(),
            Self::Subject(page) => std::iter::once(&mut page.subject)
                .chain(page.exams.iter_mut().flat_map(|exam| &mut exam.subjects))
                .collect(),
            Self::Document(file) => file.document.subjects.iter_mut().collect(),
            Self::Library(library) => library
                .saved
                .iter_mut()
                .chain(&mut library.downloaded)
                .chain(&mut library.submitted)
                .flat_map(|exam| &mut exam.subjects)
                .collect(),
            Self::Requests(page) => page
                .items
                .iter_mut()
                .map(|request| &mut request.subject)
                .collect(),
            Self::UploadForm(form) => form
                .subjects
                .iter_mut()
                .map(|entry| &mut entry.subject)
                .chain(form.request.iter_mut().map(|request| &mut request.subject))
                .collect(),
            _ => return Ok(()),
        };
        let (workspace, _, _) = store
            .view(&["settings"].into())
            .map_err(|_| super::ExamError::Storage)?;
        let language = ContentLanguage::of(&workspace);
        for subject in subjects {
            let names = Localized {
                ja: Some(std::mem::take(&mut subject.name)),
                en: subject.name_en.take(),
            };
            subject.name = names.pick(language).unwrap_or(&subject.code).to_owned();
            subject.name_en = names.en;
        }
        Ok(())
    }
}
