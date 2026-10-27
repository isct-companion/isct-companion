use thiserror::Error;

#[derive(Debug, Error)]
pub enum DataError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Compatibility(String),
    #[error("{0}")]
    Storage(String),
}

impl DataError {
    pub(crate) fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict(message.into())
    }

    pub(crate) fn compatibility(message: impl Into<String>) -> Self {
        Self::Compatibility(message.into())
    }

    pub(crate) fn storage(message: impl Into<String>) -> Self {
        Self::Storage(message.into())
    }
}

impl From<String> for DataError {
    fn from(message: String) -> Self {
        Self::InvalidInput(message)
    }
}

impl From<&str> for DataError {
    fn from(message: &str) -> Self {
        Self::InvalidInput(message.to_string())
    }
}

impl From<rusqlite::Error> for DataError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

impl From<std::io::Error> for DataError {
    fn from(error: std::io::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

impl From<serde_json::Error> for DataError {
    fn from(error: serde_json::Error) -> Self {
        Self::Storage(error.to_string())
    }
}
