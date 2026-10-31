//! The stable codes shells receive for a failure, and the one table that
//! maps every public Core error to one.
//!
//! Core never hands a shell text to show: a shell looks the code up as
//! `error.<code>` in the shared `resources/locales` catalogs. The error's
//! `Display` text stays for logs and bug reports. Several variants share a
//! code when a person would act on them the same way. Errors defined in
//! `school_browser` are mapped here so that module stays untouched by this
//! table.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

macro_rules! codes {
    ($($variant:ident => $name:literal,)*) => {
        /// A stable failure code. Names never change once shipped; a new
        /// failure a person acts on differently gets a new code.
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
        #[cfg_attr(feature = "ts", derive(ts_rs::TS))]
        pub enum ErrorCode {
            $(#[serde(rename = $name)] $variant,)*
        }

        impl ErrorCode {
            pub const ALL: &'static [Self] = &[$(Self::$variant,)*];

            /// The code shells receive, e.g. `note-too-large`.
            pub fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)*
                }
            }
        }
    };
}

codes! {
    InvalidInput => "invalid-input",
    Conflict => "conflict",
    Storage => "storage",
    Compatibility => "compatibility",
    Network => "network",
    Offline => "offline",
    Timeout => "timeout",
    Cancelled => "cancelled",
    Stopped => "stopped",
    Busy => "busy",
    Unavailable => "unavailable",
    Internal => "internal",
    Platform => "platform",
    Callback => "callback",
    WriteDenied => "write-denied",
    TooLarge => "too-large",
    VaultUnavailable => "vault-unavailable",
    SignInRequired => "sign-in-required",
    SchoolMaintenance => "school-maintenance",
    SchoolSiteChanged => "school-site-changed",
    SchoolPaused => "school-paused",
    HostPaused => "host-paused",
    WorkspaceFolderMissing => "workspace-folder-missing",
    HandInFilesOnly => "hand-in-files-only",
    HandInEmpty => "hand-in-empty",
    HandInNeedsZip => "hand-in-needs-zip",
    HandInTooLarge => "hand-in-too-large",
    DesktopOnly => "desktop-only",
    NoteTooLarge => "note-too-large",
    StudyAdmissionTerm => "study-admission-term",
    StudyEntryGrade => "study-entry-grade",
    StudyDuplicateTerm => "study-duplicate-term",
    SubscriptionInvalid => "subscription-invalid",
    CloudPasswordLength => "cloud-password-length",
    CloudPasswordWrong => "cloud-password-wrong",
    CloudCredentialFile => "cloud-credential-file",
    CloudConflict => "cloud-conflict",
    CloudConflictExhausted => "cloud-conflict-exhausted",
    CloudFormat => "cloud-format",
    CloudVersionMissing => "cloud-version-missing",
    CloudTooLarge => "cloud-too-large",
    CloudRetrying => "cloud-retrying",
    CloudNotAttached => "cloud-not-attached",
    ResetBusy => "reset-busy",
    ResetUnsupported => "reset-unsupported",
    ResetCredentials => "reset-credentials",
    ResetFiles => "reset-files",
    ResetSignOut => "reset-sign-out",
    ResetStopped => "reset-stopped",
    OtpSignInRequired => "otp-sign-in-required",
    OtpUnexpectedPage => "otp-unexpected-page",
    OtpUnavailable => "otp-unavailable",
    OtpTimedOut => "otp-timed-out",
    OtpRejected => "otp-rejected",
    OtpAlreadyRegistered => "otp-already-registered",
    OtpVault => "otp-vault",
    OtpImportInvalid => "otp-import-invalid",
    OtpImportMultiple => "otp-import-multiple",
    OtpImportUnsupported => "otp-import-unsupported",
    AgentDisabled => "agent-disabled",
    MatrixScanEnded => "matrix-scan-ended",
    MatrixUnreadable => "matrix-unreadable",
    GuideTooManyCourses => "guide-too-many-courses",
    LogsUnavailable => "logs-unavailable",
    LmsNotEnrolled => "lms-not-enrolled",
}

/// A public Core error: the code a shell shows copy for, and the values
/// that copy interpolates.
pub trait Coded: std::fmt::Display {
    fn code(&self) -> ErrorCode;
    /// Named values for the copy, e.g. `{{term}}`. Never free text.
    fn params(&self) -> BTreeMap<&'static str, String> {
        BTreeMap::new()
    }
}

/// What a shell receives for a failure. `detail` is the error's diagnostic
/// text for logs; shells never show it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CoreFailure {
    pub code: ErrorCode,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, string> | undefined"))]
    pub params: BTreeMap<String, String>,
    pub detail: String,
}

impl CoreFailure {
    pub fn new(code: ErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            params: BTreeMap::new(),
            detail: detail.into(),
        }
    }

    pub fn of(error: &(impl Coded + ?Sized)) -> Self {
        Self {
            code: error.code(),
            params: error
                .params()
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value))
                .collect(),
            detail: error.to_string(),
        }
    }
}

impl std::fmt::Display for CoreFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.name(), self.detail)
    }
}

impl std::error::Error for CoreFailure {}

impl Coded for CoreFailure {
    fn code(&self) -> ErrorCode {
        self.code
    }
}

#[cfg(feature = "agent")]
use crate::agent::AgentError;
use crate::{
    app_updates::AppUpdateError,
    backup::BackupError,
    catalog::CatalogError,
    cloud_credentials::{
        CredentialBackupError, CredentialDecryptionError, CredentialEncryptionError,
        PASSWORD_LENGTH,
    },
    cloud_sync::{document::DocumentError, CloudSyncError},
    course_materials::MaterialError,
    course_query::FinderError,
    data::DataError,
    diagnostics::DiagnosticsError,
    downloads::FetchError,
    facade::{CredentialMutationError, ReadModelError},
    kyomu_guide::GuideError,
    lms_ingest::IngestError,
    matrix_scan::MatrixScanError,
    net::NetError,
    notes::NoteError,
    notification::NotificationContentError,
    ports::{EventError, SecretError},
    reference_refresh::ReferenceError,
    reset::ResetError,
    runtime::{CloudWakeError, RuntimeError},
    runtime_tasks::RuntimeTaskError,
    schedule::{NotificationSinkError, ReminderError, ScheduleError},
    scheduler::{ExportError, SchedulerError},
    school_auth::{OtpImportError, OtpSetupError, SchoolAuthError},
    school_browser::{BrowserError, SchoolBrowserError, SubmissionError},
    storage::StorageError,
    study_profile::StudyProfileError,
    subscriptions::SubscriptionError,
    workspace::WorkspaceError,
};
use ErrorCode as C;

macro_rules! coded {
    ($error:ty, |$this:ident| $body:expr) => {
        impl Coded for $error {
            fn code(&self) -> ErrorCode {
                let $this = self;
                $body
            }
        }
    };
}

coded!(DataError, |e| match e {
    DataError::InvalidInput(_) => C::InvalidInput,
    DataError::Conflict(_) => C::Conflict,
    DataError::Compatibility(_) => C::Compatibility,
    DataError::Storage(_) => C::Storage,
});
coded!(WorkspaceError, |e| match e {
    WorkspaceError::InvalidIntent(_) => C::InvalidInput,
    WorkspaceError::Conflict(_) => C::Conflict,
    WorkspaceError::Storage(_) => C::Storage,
});
coded!(crate::workspaces::WorkspaceError, |e| {
    use crate::workspaces::WorkspaceError as E;
    match e {
        E::Input(_) => C::InvalidInput,
        E::Storage(_) | E::Archive(_) => C::Storage,
        E::Missing => C::WorkspaceFolderMissing,
    }
});
coded!(crate::workspaces::HandInError, |e| {
    use crate::workspaces::HandInError as E;
    match e {
        E::Workspace(e) => e.code(),
        E::FilesOnly => C::HandInFilesOnly,
        E::Empty => C::HandInEmpty,
        E::NeedsZip => C::HandInNeedsZip,
        E::TooLarge => C::HandInTooLarge,
    }
});
coded!(ScheduleError, |e| match e {
    ScheduleError::InvalidInput(_) => C::InvalidInput,
    ScheduleError::Compatibility(_) => C::Compatibility,
});
coded!(FinderError, |e| match e {
    FinderError::InvalidInput(_) => C::InvalidInput,
    FinderError::Schedule(e) => e.code(),
    FinderError::Data(e) => e.code(),
    FinderError::Compatibility(_) => C::Compatibility,
});
coded!(CatalogError, |e| match e {
    CatalogError::Data(e) => e.code(),
    CatalogError::Json(_) => C::Compatibility,
    CatalogError::InvalidInput(_) => C::InvalidInput,
});
coded!(ReadModelError, |e| match e {
    ReadModelError::Workspace(e) => e.code(),
    ReadModelError::Schedule(e) => e.code(),
    ReadModelError::Finder(e) => e.code(),
    ReadModelError::Catalog(e) => e.code(),
    ReadModelError::InvalidInput(_) => C::InvalidInput,
});
coded!(RuntimeTaskError, |e| match e {
    RuntimeTaskError::Stopped => C::Stopped,
    RuntimeTaskError::Unavailable | RuntimeTaskError::Panicked => C::Internal,
    RuntimeTaskError::Interrupted => C::Cancelled,
});
coded!(RuntimeError, |e| match e {
    RuntimeError::Task(e) => e.code(),
    RuntimeError::Clock | RuntimeError::CloudAlreadyAttached => C::Internal,
});
coded!(CloudWakeError, |e| match e {
    CloudWakeError::NotAttached => C::CloudNotAttached,
    CloudWakeError::Sync(e) => e.code(),
});
coded!(CloudSyncError, |e| match e {
    CloudSyncError::Storage(_) => C::Storage,
    CloudSyncError::Compatibility(_) => C::CloudFormat,
    CloudSyncError::Transport(_) => C::Network,
    CloudSyncError::Conflict(_) => C::CloudConflict,
    CloudSyncError::Interrupted | CloudSyncError::Task(_) | CloudSyncError::StorageTask(_) =>
        C::CloudRetrying,
    CloudSyncError::Stopped => C::Stopped,
    CloudSyncError::Cancelled => C::Cancelled,
});
coded!(DocumentError, |e| match e {
    DocumentError::InvalidJson
    | DocumentError::InvalidFormat
    | DocumentError::RevisionExhausted => C::CloudFormat,
    DocumentError::Conflict => C::CloudConflict,
    DocumentError::ConflictExhausted => C::CloudConflictExhausted,
    DocumentError::VersionMissing => C::CloudVersionMissing,
    DocumentError::TooLarge => C::CloudTooLarge,
    DocumentError::Transport(_) => C::Network,
});
impl Coded for CredentialEncryptionError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::InvalidPassword => C::CloudPasswordLength,
            _ => C::Internal,
        }
    }

    fn params(&self) -> BTreeMap<&'static str, String> {
        match self {
            Self::InvalidPassword => BTreeMap::from([
                ("min", PASSWORD_LENGTH.min.to_string()),
                ("max", PASSWORD_LENGTH.max.to_string()),
            ]),
            _ => BTreeMap::new(),
        }
    }
}
coded!(CredentialDecryptionError, |e| match e {
    CredentialDecryptionError::InvalidPassword | CredentialDecryptionError::Authentication =>
        C::CloudPasswordWrong,
    CredentialDecryptionError::Setup => C::Internal,
    _ => C::CloudCredentialFile,
});
impl Coded for CredentialBackupError {
    fn code(&self) -> ErrorCode {
        self.inner().code()
    }

    fn params(&self) -> BTreeMap<&'static str, String> {
        self.inner().params()
    }
}
impl CredentialBackupError {
    fn inner(&self) -> &dyn Coded {
        match self {
            Self::Encryption(e) => e,
            Self::Decryption(e) => e,
            Self::Cloud(e) => e,
            Self::Vault(e) => e,
            Self::School(e) => e,
        }
    }
}
coded!(SecretError, |e| match e {
    SecretError::Callback(_) => C::Callback,
    SecretError::InvalidAccount | SecretError::InvalidPayload => C::Internal,
    SecretError::Access(_) => C::VaultUnavailable,
});
coded!(EventError, |_e| C::Callback);
coded!(NotificationSinkError, |_e| C::Callback);
coded!(SchoolAuthError, |e| match e {
    SchoolAuthError::InvalidInput(_) => C::InvalidInput,
    SchoolAuthError::Vault(e) => e.code(),
    SchoolAuthError::Compatibility(_) => C::Compatibility,
    SchoolAuthError::Storage(_) => C::Storage,
});
coded!(OtpSetupError, |e| match e {
    OtpSetupError::SignInRequired => C::OtpSignInRequired,
    OtpSetupError::UnexpectedPage => C::OtpUnexpectedPage,
    OtpSetupError::Unavailable => C::OtpUnavailable,
    OtpSetupError::Cancelled => C::Cancelled,
    OtpSetupError::TimedOut => C::OtpTimedOut,
    OtpSetupError::Rejected => C::OtpRejected,
    OtpSetupError::AlreadyRegistered => C::OtpAlreadyRegistered,
    OtpSetupError::Storage => C::OtpVault,
});
coded!(OtpImportError, |e| match e {
    OtpImportError::Invalid => C::OtpImportInvalid,
    OtpImportError::MultiplePages => C::OtpImportMultiple,
    OtpImportError::Unsupported => C::OtpImportUnsupported,
});
coded!(SchoolBrowserError, |e| match e {
    SchoolBrowserError::InvalidInput(_) => C::InvalidInput,
    SchoolBrowserError::Platform(_) => C::Platform,
    SchoolBrowserError::Storage(_) => C::Storage,
    SchoolBrowserError::Auth(_) => C::SignInRequired,
    SchoolBrowserError::Maintenance(_) => C::SchoolMaintenance,
    SchoolBrowserError::SiteChanged(_) => C::SchoolSiteChanged,
    SchoolBrowserError::Timeout(_) => C::Timeout,
    SchoolBrowserError::Cancelled => C::Cancelled,
    SchoolBrowserError::Preempted => C::SchoolPaused,
    SchoolBrowserError::Stopped => C::Stopped,
});
coded!(BrowserError, |e| match e {
    BrowserError::Unavailable(_) => C::Unavailable,
    BrowserError::Dispatch(_) => C::Platform,
    BrowserError::Stopped => C::Stopped,
});
coded!(SubmissionError, |e| match e {
    SubmissionError::InvalidInput(_) => C::InvalidInput,
    SubmissionError::Storage(_) => C::Storage,
    SubmissionError::Browser(_) => C::Platform,
    SubmissionError::WorkspaceMissing => C::WorkspaceFolderMissing,
});
coded!(CredentialMutationError, |e| match e {
    CredentialMutationError::Auth(e) => e.code(),
    CredentialMutationError::Browser(e) => e.code(),
});
coded!(NetError, |e| match e {
    NetError::Offline => C::Offline,
    NetError::Dns | NetError::Tls(_) | NetError::Connect(_) | NetError::Http(_) => C::Network,
    NetError::Stalled | NetError::Deadline => C::Timeout,
    NetError::TooLarge => C::TooLarge,
    NetError::HostPaused { .. } => C::HostPaused,
    NetError::SignInRequired => C::SignInRequired,
    NetError::Cancelled => C::Cancelled,
    NetError::Invalid(_) => C::InvalidInput,
});
coded!(FetchError, |e| match e {
    FetchError::Login => C::SignInRequired,
    FetchError::Invalid => C::InvalidInput,
    FetchError::Network => C::Network,
    FetchError::Timeout => C::Timeout,
    FetchError::Cancelled => C::Cancelled,
    FetchError::WriteDenied => C::WriteDenied,
});
coded!(MaterialError, |e| match e {
    MaterialError::Input(_) => C::InvalidInput,
    MaterialError::Storage(_) => C::Storage,
    MaterialError::Denied(_) => C::WriteDenied,
    MaterialError::UnsupportedResolution | MaterialError::UnsupportedDownload => C::DesktopOnly,
});
coded!(StorageError, |e| match e {
    StorageError::Input(_) => C::InvalidInput,
    StorageError::Storage(_) => C::Storage,
    StorageError::Denied(_) => C::WriteDenied,
});
coded!(NoteError, |e| match e {
    NoteError::Input(_) => C::InvalidInput,
    NoteError::TooLarge => C::NoteTooLarge,
    NoteError::Storage(_) | NoteError::Lock => C::Storage,
});
coded!(BackupError, |e| match e {
    BackupError::Data(e) => e.code(),
    BackupError::Invalid(_) | BackupError::Json(_) => C::Compatibility,
    BackupError::Storage(_) => C::Storage,
});
coded!(IngestError, |e| match e {
    IngestError::InvalidPayload(_) => C::SchoolSiteChanged,
    IngestError::Storage(_) => C::Storage,
    IngestError::NotEnrolled(_) => C::LmsNotEnrolled,
});
coded!(NotificationContentError, |_e| C::TooLarge);
coded!(DiagnosticsError, |e| match e {
    DiagnosticsError::Unavailable => C::LogsUnavailable,
    DiagnosticsError::AlreadyConfigured => C::Internal,
});
coded!(SubscriptionError, |e| match e {
    SubscriptionError::Validation(_) => C::SubscriptionInvalid,
    SubscriptionError::Network(_) => C::Network,
    SubscriptionError::Storage(_) => C::Storage,
});
coded!(ResetError, |e| match e {
    ResetError::Busy => C::ResetBusy,
    ResetError::Unsupported(_) => C::ResetUnsupported,
    ResetError::Storage(_) => C::Storage,
    ResetError::Credentials(_) => C::ResetCredentials,
    ResetError::Files(_) => C::ResetFiles,
    ResetError::SignOut(_) => C::ResetSignOut,
    ResetError::Stopped => C::ResetStopped,
});
coded!(AppUpdateError, |e| match e {
    AppUpdateError::Version(_) => C::Compatibility,
    AppUpdateError::Network(_) => C::Network,
});
coded!(ExportError, |_e| C::Callback);
coded!(SchedulerError, |e| match e {
    SchedulerError::InvalidTask(_) => C::InvalidInput,
    SchedulerError::Storage(e) => e.code(),
    SchedulerError::Export(e) => e.code(),
});
coded!(ReferenceError, |e| match e {
    ReferenceError::InvalidInput(_) => C::InvalidInput,
    ReferenceError::Network(_) => C::Network,
    ReferenceError::Storage(_) => C::Storage,
    ReferenceError::Conflict(_) => C::Conflict,
    ReferenceError::Compatibility(_) => C::SchoolSiteChanged,
    ReferenceError::Cancelled | ReferenceError::Superseded => C::Cancelled,
});
coded!(ReminderError, |e| match e {
    ReminderError::InvalidInput(_) => C::InvalidInput,
    ReminderError::Data(e) => e.code(),
    ReminderError::Storage(_) => C::Storage,
    ReminderError::Policy(e) => e.code(),
    ReminderError::Task(e) => e.code(),
    ReminderError::Delivery(e) => e.code(),
});
coded!(GuideError, |e| match e {
    GuideError::TooManyCourses => C::GuideTooManyCourses,
    GuideError::InvalidItem => C::InvalidInput,
});
coded!(MatrixScanError, |e| match e {
    MatrixScanError::Ended => C::MatrixScanEnded,
    MatrixScanError::InvalidFrame(_) => C::InvalidInput,
    MatrixScanError::Unreadable(_) => C::MatrixUnreadable,
    MatrixScanError::Unavailable => C::Unavailable,
    MatrixScanError::Backend(_) => C::Internal,
});
#[cfg(feature = "agent")]
coded!(AgentError, |e| match e {
    AgentError::Disabled => C::AgentDisabled,
    AgentError::UnknownTool | AgentError::InvalidInput(_) => C::InvalidInput,
    AgentError::Conflict(_) => C::Conflict,
    AgentError::Compatibility(_) => C::Compatibility,
    AgentError::Storage(_) => C::Storage,
});

impl Coded for StudyProfileError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::InvalidAdmissionTerm { .. } => C::StudyAdmissionTerm,
            Self::InvalidEntryGrade => C::StudyEntryGrade,
            Self::DuplicateAdmissionTerm { .. } => C::StudyDuplicateTerm,
        }
    }

    fn params(&self) -> BTreeMap<&'static str, String> {
        match self {
            Self::InvalidAdmissionTerm { term } | Self::DuplicateAdmissionTerm { term } => {
                BTreeMap::from([("term", term.clone())])
            }
            Self::InvalidEntryGrade => BTreeMap::new(),
        }
    }
}

#[cfg(test)]
#[path = "tests/error_code.rs"]
mod tests;
