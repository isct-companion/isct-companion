use crate::file_preview::Viewer;
use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
/// A subject as shells show it: its name already in the content language
/// (`approach/content-language-in-core`). Core builds it only from the
/// service's record (`remote::RemoteSubject::present`), so no answer can skip
/// the choice.
pub struct ExamSubject {
    pub code: String,
    pub name: String,
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
    /// No teachers and `all_teachers` false is "teacher unknown".
    pub teachers: Vec<ExamTeacher>,
    /// The material is common to every teacher of the subject.
    pub all_teachers: bool,
    pub academic_year: Option<u32>,
    /// The API's occasion code; a code this build does not know is shown as
    /// "other".
    pub occasion: Option<String>,
    /// The name of an `other` occasion; empty otherwise.
    pub occasion_text: String,
    /// Shown when above 0.
    pub occasion_number: u32,
    pub variant: String,
    pub role: Option<String>,
    /// The name of an `other` role; empty otherwise.
    pub role_text: String,
    pub note: Option<String>,
    pub file_name: String,
    pub file_type: String,
    /// In a sitting (`ExamSet`) with several documents of this one role and
    /// role name, its place among them from 1, so they can be told apart
    /// ("Question 2"); otherwise none.
    pub number: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamSubjectPage {
    pub subject: ExamSubject,
    pub rows: Vec<ExamClass>,
    /// The local course's class when one of `rows` lists it. Reopening the
    /// subject passes it back so the order survives a refresh.
    pub class_id: Option<u32>,
    /// This subject on the ScienceTokyoExam website.
    pub web_url: String,
    /// The subject's documents by sitting: the class's own first, then newest
    /// first and in occasion order. A question and its answer are one row on
    /// every shell.
    pub sets: Vec<ExamSet>,
}

/// The documents of one sitting, as the service pairs them (its `pairKey`):
/// one year, occasion, round, variant and set of teachers. A document the
/// service pairs with none is a sitting of its own. Documents run question,
/// both, answer, notes, other.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamSet {
    pub academic_year: Option<u32>,
    pub occasion: Option<String>,
    pub occasion_text: String,
    pub occasion_number: u32,
    pub variant: String,
    pub teachers: Vec<ExamTeacher>,
    pub all_teachers: bool,
    /// The sitting belongs to the course's own class, as the service decides
    /// it: only on a page with a class, for a class with no teachers listed,
    /// a material common to all teachers, or one naming a class teacher.
    pub mine: bool,
    /// Whether a row names the sitting's teachers (`teachers`, else common
    /// to all or unknown): all but a sitting of the class's own listed
    /// teachers, which would only repeat them.
    pub show_teachers: bool,
    pub documents: Vec<ExamDocument>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamRequest {
    pub id: String,
    pub subject: ExamSubject,
    pub teachers: Vec<ExamTeacher>,
    pub academic_year: Option<u32>,
    /// None when the request names none, or one this build does not know.
    pub occasion: Option<ExamOccasion>,
    pub wanted_role: Option<ExamRole>,
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
    /// This document on the ScienceTokyoExam website.
    pub web_url: String,
    /// How the shells show it (`file_preview`).
    pub viewer: Viewer,
    /// The name a saved or shared copy takes: the uploader's name, with the
    /// validated type's suffix when its own extension is not one that type
    /// allows. The cached file at `path` has a generated name.
    pub export_name: String,
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
    WrongInfo,
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
    /// Common to every teacher; then it names none.
    pub all_teachers: bool,
    pub academic_year: Option<u32>,
    /// None until the person picks one, as on the website.
    pub occasion: Option<ExamOccasion>,
    /// The name of an `other` occasion.
    pub occasion_text: String,
    pub occasion_number: u32,
    pub variant: String,
    pub role: ExamRole,
    /// The name of an `other` role.
    pub role_text: String,
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

/// When a material was used. An `Other` occasion carries a name; notes and
/// other materials have none (`NotApplicable`, the API's `none`).
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ExamOccasion {
    Final,
    Midterm,
    Quiz,
    Report,
    Other,
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

/// One upload role and what it allows, in the service's order.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamRoleChoice {
    pub role: ExamRole,
    /// A single entry is set without asking (`none` for notes).
    pub occasions: Vec<ExamOccasion>,
    /// Empty when the role has no answer.
    pub answer_sources: Vec<ExamAnswerSource>,
}

/// The values each past-exams form offers, in the service's order, from its
/// `client-config`. Forms take their pickers from here, never from a list of
/// their own. Codes this build has no words for are left out; every list a
/// form needs has at least one entry.
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

/// The ScienceTokyoExam website's pages, which a shell links to in every
/// account state: before connecting, offline or on an old app version too
/// (`ExamAction::Site`). A view's own object (a subject, a document) has its
/// own `web_url`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamSitePages {
    pub search: String,
    pub requests: String,
    pub library: String,
    pub terms: String,
    pub privacy: String,
}

/// The service's limits, for the forms' hints only (a field's maximum
/// length, a disabled "add teacher", the years a picker offers): the service
/// checks every form. Text lengths count UTF-16 code units. A limit the
/// service stops sending is absent, and its hint with it.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", default)]
pub struct ExamLimits {
    /// Subjects one upload may list.
    pub subjects: Option<u32>,
    /// Listed teachers one upload may name.
    pub teachers: Option<u32>,
    /// Teachers one upload may add to the list.
    pub new_teachers: Option<u32>,
    /// A variant, an occasion or role name, a new teacher's family or given
    /// name.
    pub metadata_text: Option<u32>,
    pub note: Option<u32>,
    pub report_detail: Option<u32>,
    pub occasion_number_max: Option<u32>,
    pub request_description: Option<u32>,
    pub search_query: Option<u32>,
    pub max_upload_bytes: Option<u32>,
    /// The academic years an upload may name.
    pub earliest_year: Option<u32>,
    pub latest_year: Option<u32>,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamUploadReadiness {
    pub reason_key: Option<String>,
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
    const ALL: [Self; 6] = [
        Self::Copyright,
        Self::Inappropriate,
        Self::WrongInfo,
        Self::WrongAnswer,
        Self::BrokenFile,
        Self::Other,
    ];

    /// The API's snake_case code, which differs from the camelCase name
    /// shells use.
    pub(super) fn api_code(&self) -> &'static str {
        match self {
            Self::Copyright => "copyright",
            Self::Inappropriate => "inappropriate",
            Self::WrongInfo => "wrong_info",
            Self::WrongAnswer => "wrong_answer",
            Self::BrokenFile => "broken_file",
            Self::Other => "other",
        }
    }

    pub(super) fn from_api_code(code: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|reason| reason.api_code() == code)
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
    /// The website's pages; answered without the network or an account.
    Site,
    /// The choices the forms offer, from the service (cached for five
    /// minutes). A form asks when it opens.
    Choices,
    Connect,
    CancelConnect,
    AcceptTerms,
    /// Refuse the terms on the consent page: the account is left, and the
    /// service told to forget an account that never registered.
    Decline,
    Disconnect,
    /// Delete the account on the service, then sign out and forget it here.
    DeleteAccount,
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
    /// Whether the staged files can be sent; the service checks the draft.
    UploadReadiness {
        files: Vec<ExamUploadFile>,
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
    Site(ExamSitePages),
    Choices(ExamChoices),
    UploadForm(ExamUploadForm),
    UploadReadiness(ExamUploadReadiness),
    /// The upload is published; the shell reads its view again. Carries the
    /// new document's id, so the shell can open it, unless the attempt that
    /// registered it lost the answer.
    Uploaded(Option<String>),
    Done,
}
