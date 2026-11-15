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
    pub exam_count: Option<u32>,
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
    pub exams: Vec<ExamDocument>,
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
    pub occasion: Option<String>,
    pub wanted_role: Option<String>,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ExamAccountStatus {
    Disconnected,
    NeedsConsent,
    Connected,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamAccount {
    pub status: ExamAccountStatus,
    pub email: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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
    pub occasion: String,
    pub occasion_number: u32,
    pub variant: String,
    pub role: String,
    pub provenance: String,
    pub answer_source: Option<String>,
    pub note: String,
    pub request_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamUploadOptions {
    pub draft: ExamUploadDraft,
    pub teachers: Vec<ExamTeacher>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
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

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamUploadStatus {
    pub phase: ExamUploadPhase,
    pub sent: u32,
    pub total: u32,
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
pub enum ExamAction {
    Account,
    AccountSnapshot,
    Connect,
    CancelConnect,
    AcceptTerms,
    Disconnect,
    Search {
        query: String,
    },
    Subject {
        code: String,
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
    RemoveDownload {
        id: String,
    },
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
    SetRequestOpen {
        id: String,
        open: bool,
    },
    DeleteRequest {
        id: String,
    },
    UploadOptions {
        subject_code: String,
        request: Option<ExamRequest>,
    },
    Upload {
        files: Vec<ExamUploadFile>,
        draft: ExamUploadDraft,
    },
    UploadStatus,
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
    UploadOptions(ExamUploadOptions),
    UploadStatus(ExamUploadStatus),
    Uploaded(String),
    Done,
}
