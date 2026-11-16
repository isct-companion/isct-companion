//! Process-wide application log. A free function, not a `Session` method, so
//! a shell can still export the log when opening the session failed.
use crate::SdkError;

/// Core's redacted log as text; `about` names the app build and platform.
#[uniffi::export]
pub fn export_application_logs(about: String) -> Result<String, SdkError> {
    app_core::diagnostics::export_text(&about).map_err(|e| crate::error::coded(&e))
}

/// The feedback form, with this build's `version` and `device` filled in.
#[uniffi::export]
pub fn feedback_form_url(version: String, device: String) -> String {
    app_core::feedback::form_url(&version, &device)
}
