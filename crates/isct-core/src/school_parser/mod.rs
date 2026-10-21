//! Pure parsing boundary for school-owned pages and extractor payloads.
//!
//! Code in this module must not open network connections, touch Tauri, or
//! write to SQLite. Acquisition, session recovery, downloads, and persistence
//! are deliberately kept in their own services.

pub mod kyomu;
pub mod lms;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct SchoolParseError(String);

impl From<String> for SchoolParseError {
    fn from(message: String) -> Self {
        Self(message)
    }
}

impl From<&str> for SchoolParseError {
    fn from(message: &str) -> Self {
        Self(message.to_string())
    }
}

impl From<serde_json::Error> for SchoolParseError {
    fn from(error: serde_json::Error) -> Self {
        Self(error.to_string())
    }
}
