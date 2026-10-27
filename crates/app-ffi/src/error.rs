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
impl From<app_core::schedule::ReminderError> for SdkError {
    fn from(e: app_core::schedule::ReminderError) -> Self {
        use app_core::schedule::ReminderError as E;
        match e {
            E::InvalidInput(detail) => Self::InvalidInput { detail },
            E::Data(e) => e.into(),
            E::Storage(detail) => Self::Storage { detail },
            E::Policy(e) => e.into(),
            E::Task(e) => e.into(),
            E::Delivery(e) => Self::Callback {
                detail: e.to_string(),
            },
        }
    }
}
impl From<app_core::schedule::ScheduleError> for SdkError {
    fn from(e: app_core::schedule::ScheduleError) -> Self {
        match e {
            app_core::schedule::ScheduleError::InvalidInput(detail) => {
                Self::InvalidInput { detail }
            }
            app_core::schedule::ScheduleError::Compatibility(detail) => {
                Self::Compatibility { detail }
            }
        }
    }
}
impl From<app_core::facade::CredentialMutationError> for SdkError {
    fn from(e: app_core::facade::CredentialMutationError) -> Self {
        match e {
            app_core::facade::CredentialMutationError::Auth(e) => e.into(),
            app_core::facade::CredentialMutationError::Browser(e) => e.into(),
        }
    }
}
impl From<app_core::runtime::CloudWakeError> for SdkError {
    fn from(e: app_core::runtime::CloudWakeError) -> Self {
        match e {
            app_core::runtime::CloudWakeError::Sync(e) => e.into(),
            app_core::runtime::CloudWakeError::NotAttached => Self::Unavailable {
                detail: e.to_string(),
            },
        }
    }
}
impl From<uniffi::UnexpectedUniFFICallbackError> for SdkError {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Callback {
            detail: e.to_string(),
        }
    }
}
impl From<app_core::data::DataError> for SdkError {
    fn from(e: app_core::data::DataError) -> Self {
        use app_core::data::DataError as E;
        match e {
            E::InvalidInput(detail) => Self::InvalidInput { detail },
            E::Conflict(detail) => Self::Conflict { detail },
            E::Compatibility(detail) => Self::Compatibility { detail },
            E::Storage(detail) => Self::Storage { detail },
        }
    }
}
impl From<app_core::workspace::WorkspaceError> for SdkError {
    fn from(e: app_core::workspace::WorkspaceError) -> Self {
        use app_core::workspace::WorkspaceError as E;
        match e {
            E::InvalidIntent(detail) => Self::InvalidInput { detail },
            E::Conflict(detail) => Self::Conflict { detail },
            E::Storage(detail) => Self::Storage { detail },
        }
    }
}
impl From<app_core::runtime_tasks::RuntimeTaskError> for SdkError {
    fn from(e: app_core::runtime_tasks::RuntimeTaskError) -> Self {
        use app_core::runtime_tasks::RuntimeTaskError as E;
        match e {
            E::Stopped => Self::Closed,
            other => Self::Internal {
                detail: other.to_string(),
            },
        }
    }
}
impl From<app_core::facade::ReadModelError> for SdkError {
    fn from(e: app_core::facade::ReadModelError) -> Self {
        use app_core::facade::ReadModelError as E;
        match e {
            E::Workspace(e) => e.into(),
            E::InvalidInput(detail) => Self::InvalidInput { detail },
            E::Schedule(e) => e.into(),
            E::Finder(e) => match e {
                app_core::course_query::FinderError::InvalidInput(detail) => {
                    Self::InvalidInput { detail }
                }
                app_core::course_query::FinderError::Schedule(e) => e.into(),
                app_core::course_query::FinderError::Data(e) => e.into(),
                app_core::course_query::FinderError::Compatibility(e) => Self::Compatibility {
                    detail: e.to_string(),
                },
            },
            E::Catalog(e) => e.into(),
        }
    }
}
impl From<app_core::school_auth::SchoolAuthError> for SdkError {
    fn from(e: app_core::school_auth::SchoolAuthError) -> Self {
        use app_core::school_auth::SchoolAuthError as E;
        match e {
            E::InvalidInput(detail) => Self::InvalidInput { detail },
            E::Compatibility(detail) => Self::Compatibility { detail },
            E::Storage(detail) => Self::Storage { detail },
            E::Vault(app_core::ports::SecretError::Callback(detail)) => Self::Callback { detail },
            E::Vault(e) => Self::Unavailable {
                detail: e.to_string(),
            },
        }
    }
}
impl From<app_core::school_browser::SchoolBrowserError> for SdkError {
    fn from(e: app_core::school_browser::SchoolBrowserError) -> Self {
        use app_core::school_browser::SchoolBrowserError as E;
        match e {
            E::InvalidInput(detail) => Self::InvalidInput { detail },
            E::Storage(detail) => Self::Storage { detail },
            E::Auth(detail) => Self::Authentication { detail },
            E::Timeout(detail) => Self::Timeout { detail },
            E::Cancelled | E::Preempted => Self::Cancelled,
            E::Stopped => Self::Closed,
            E::Maintenance(detail) => Self::Maintenance { detail },
            E::Platform(detail) | E::SiteChanged(detail) => Self::Unavailable { detail },
        }
    }
}
impl From<app_core::catalog::CatalogError> for SdkError {
    fn from(e: app_core::catalog::CatalogError) -> Self {
        use app_core::catalog::CatalogError as E;
        match e {
            E::Data(e) => e.into(),
            E::InvalidInput(detail) => Self::InvalidInput { detail },
            E::Json(e) => Self::Compatibility {
                detail: e.to_string(),
            },
        }
    }
}
impl From<app_core::reference_refresh::ReferenceError> for SdkError {
    fn from(e: app_core::reference_refresh::ReferenceError) -> Self {
        use app_core::reference_refresh::ReferenceError as E;
        match e {
            E::InvalidInput(detail) => Self::InvalidInput { detail },
            E::Network(detail) => Self::Network { detail },
            E::Storage(detail) => Self::Storage { detail },
            E::Conflict(detail) => Self::Conflict { detail },
            E::Compatibility(detail) => Self::Compatibility { detail },
            E::Cancelled => Self::Cancelled,
            E::Superseded => Self::Cancelled,
        }
    }
}
impl From<app_core::cloud_sync::CloudSyncError> for SdkError {
    fn from(e: app_core::cloud_sync::CloudSyncError) -> Self {
        use app_core::cloud_sync::CloudSyncError as E;
        match e {
            E::Storage(detail) => Self::Storage { detail },
            E::Compatibility(detail) => Self::Compatibility { detail },
            E::Conflict(detail) => Self::Conflict { detail },
            E::Transport(detail) => Self::Network { detail },
            E::Interrupted => Self::Timeout {
                detail: e.to_string(),
            },
            E::Stopped => Self::Closed,
            E::Task(e) | E::StorageTask(e) => e.into(),
        }
    }
}
