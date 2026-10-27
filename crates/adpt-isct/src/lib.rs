//! What is particular to Institute of Science Tokyo: parsers for its LMS and
//! 教務Web pages, the scripts its school pages run, and its URLs.
//!
//! This crate holds no state. It must not open network connections, touch a
//! platform, or write to SQLite; acquisition, session recovery, downloads and
//! persistence stay in Core, which depends on this crate directly.

pub mod kyomu;
pub mod lms;
pub mod scripts;
pub mod urls;

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
