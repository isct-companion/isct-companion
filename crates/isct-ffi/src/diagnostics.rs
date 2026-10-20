//! Process-wide application log. A free function, not a `Session` method, so
//! a shell can still export the log when opening the session failed.
use crate::SdkError;

/// Core's redacted log as text; `about` names the app build and platform.
#[uniffi::export]
pub fn export_application_logs(about: String) -> Result<String, SdkError> {
    isct_core::diagnostics::export_text(&about).map_err(|e| SdkError::Unavailable {
        detail: e.to_string(),
    })
}
