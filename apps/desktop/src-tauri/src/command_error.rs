//! What a native command rejects with. A Core failure carries only its
//! stable code (`app_core::error_code`); the frontend shows `error.<code>`
//! from the shared locale catalogs. A failure of the desktop shell itself
//! carries its English copy, which is a key into the same catalogs.
use app_core::error_code::{Coded, CoreFailure};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum CommandError {
    Core(CoreFailure),
    Shell { message: String },
}

impl From<CoreFailure> for CommandError {
    fn from(failure: CoreFailure) -> Self {
        Self::Core(failure)
    }
}

impl From<String> for CommandError {
    fn from(message: String) -> Self {
        Self::Shell { message }
    }
}

impl From<&str> for CommandError {
    fn from(message: &str) -> Self {
        Self::Shell {
            message: message.to_owned(),
        }
    }
}

impl std::error::Error for CommandError {}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Core(failure) => failure.fmt(f),
            Self::Shell { message } => f.write_str(message),
        }
    }
}

/// Core errors commands propagate with `?`.
macro_rules! from_coded {
    ($($error:ty),* $(,)?) => {
        $(impl From<$error> for CommandError {
            fn from(error: $error) -> Self {
                coded(error)
            }
        })*
    };
}

from_coded!(
    app_core::workspaces::WorkspaceError,
    app_core::school_browser::SubmissionError,
    app_core::school_auth::OtpSetupError,
    app_core::school_auth::OtpImportError,
);

/// `map_err(coded)` for a Core error.
pub(crate) fn coded(error: impl Coded) -> CommandError {
    CommandError::Core(CoreFailure::of(&error))
}

/// `map_err(shell)` for a failure of the desktop shell, whose text is
/// already its English copy.
pub(crate) fn shell(error: impl std::fmt::Display) -> CommandError {
    CommandError::Shell {
        message: error.to_string(),
    }
}
