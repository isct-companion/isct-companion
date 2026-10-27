/// A failure across the SDK boundary. When Core raised it, `detail` is its
/// stable `ErrorCode` name (e.g. `plan-invalid-title`): shells show the copy
/// `error.<detail>` from the shared locale catalogs, and the variant says how
/// to react. Failures shells raise into Core carry free text, which Core
/// only logs.
#[derive(Clone, Debug, thiserror::Error, uniffi::Error)]
pub enum SdkError {
    #[error("Invalid input: {detail}")]
    InvalidInput { detail: String },
    #[error("Revision conflict: {detail}")]
    Conflict { detail: String },
    #[error("Storage failure: {detail}")]
    Storage { detail: String },
    #[error("Unsupported stored data: {detail}")]
    Compatibility { detail: String },
    #[error("Authentication required: {detail}")]
    Authentication { detail: String },
    #[error("School service under maintenance: {detail}")]
    Maintenance { detail: String },
    #[error("Network failure: {detail}")]
    Network { detail: String },
    #[error("Operation timed out: {detail}")]
    Timeout { detail: String },
    #[error("Capability unavailable: {detail}")]
    Unavailable { detail: String },
    #[error("Capability busy: {detail}")]
    Busy { detail: String },
    #[error("Platform callback failed: {detail}")]
    Callback { detail: String },
    #[error("Operation cancelled")]
    Cancelled,
    #[error("Session is closed")]
    Closed,
    #[error("Internal worker failed: {detail}")]
    Internal { detail: String },
}
use app_core::error_code::{Coded, ErrorCode as C};

/// The SDK failure for a Core error: its code as `detail`, under the variant
/// a shell reacts to.
pub(crate) fn coded(error: &(impl Coded + ?Sized)) -> SdkError {
    let code = error.code();
    let detail = code.name().to_owned();
    match code {
        C::Conflict | C::CloudConflict | C::CloudConflictExhausted => SdkError::Conflict { detail },
        C::Storage
        | C::WriteDenied
        | C::VaultUnavailable
        | C::OtpVault
        | C::ResetCredentials
        | C::ResetFiles => SdkError::Storage { detail },
        C::Compatibility | C::CloudFormat | C::CloudVersionMissing => {
            SdkError::Compatibility { detail }
        }
        C::SignInRequired | C::OtpSignInRequired => SdkError::Authentication { detail },
        C::SchoolMaintenance => SdkError::Maintenance { detail },
        C::Network | C::Offline | C::HostPaused => SdkError::Network { detail },
        C::Timeout | C::OtpTimedOut | C::CloudRetrying => SdkError::Timeout { detail },
        C::Busy | C::ResetBusy => SdkError::Busy { detail },
        C::Cancelled | C::SchoolPaused | C::ResetStopped => SdkError::Cancelled,
        C::Stopped => SdkError::Closed,
        C::Callback | C::ResetSignOut => SdkError::Callback { detail },
        C::Internal => SdkError::Internal { detail },
        C::Unavailable
        | C::Platform
        | C::DesktopOnly
        | C::LogsUnavailable
        | C::CloudNotAttached
        | C::SchoolSiteChanged
        | C::OtpUnavailable
        | C::OtpUnexpectedPage
        | C::ResetUnsupported
        | C::MatrixScanEnded
        | C::AgentDisabled
        | C::AgentWritesDisabled => SdkError::Unavailable { detail },
        C::InvalidInput
        | C::TooLarge
        | C::NoteTooLarge
        | C::PlanInvalidStart
        | C::PlanInvalidTitle
        | C::PlanInvalidDuration
        | C::PlanInvalidExdate
        | C::PlanInvalidReminder
        | C::PlanInvalidRule
        | C::PlanRuleOnly
        | C::PlanRuleFrequency
        | C::PlanRepeatUnsupported
        | C::PlanRepeatSubdaily
        | C::StudyAdmissionTerm
        | C::StudyEntryGrade
        | C::StudyDuplicateTerm
        | C::SubscriptionInvalid
        | C::CloudPasswordLength
        | C::CloudPasswordWrong
        | C::CloudCredentialFile
        | C::CloudTooLarge
        | C::OtpRejected
        | C::OtpAlreadyRegistered
        | C::OtpImportInvalid
        | C::OtpImportMultiple
        | C::OtpImportUnsupported
        | C::MatrixUnreadable
        | C::GuideTooManyCourses
        | C::LmsNotEnrolled
        | C::WorkspaceFolderMissing
        | C::HandInFilesOnly
        | C::HandInEmpty
        | C::HandInNeedsZip
        | C::HandInTooLarge => SdkError::InvalidInput { detail },
    }
}

macro_rules! from_coded {
    ($($error:ty),* $(,)?) => {
        $(impl From<$error> for SdkError {
            fn from(e: $error) -> Self {
                coded(&e)
            }
        })*
    };
}

from_coded!(
    app_core::schedule::ReminderError,
    app_core::schedule::ScheduleError,
    app_core::facade::CredentialMutationError,
    app_core::runtime::CloudWakeError,
    app_core::data::DataError,
    app_core::workspace::WorkspaceError,
    app_core::runtime_tasks::RuntimeTaskError,
    app_core::facade::ReadModelError,
    app_core::school_auth::SchoolAuthError,
    app_core::school_browser::SchoolBrowserError,
    app_core::catalog::CatalogError,
    app_core::reference_refresh::ReferenceError,
    app_core::cloud_sync::CloudSyncError,
    app_core::reset::ResetError,
);

impl From<uniffi::UnexpectedUniFFICallbackError> for SdkError {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Callback {
            detail: e.to_string(),
        }
    }
}
