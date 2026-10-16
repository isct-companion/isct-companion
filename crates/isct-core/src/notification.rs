//! Content limits for a native notification, shared by Core-planned
//! reminders and shell-requested notifications.

pub const MAX_TITLE_BYTES: usize = 160;
pub const MAX_BODY_BYTES: usize = 2_000;

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum NotificationContentError {
    #[error("notification content is too large")]
    TooLarge,
}

/// A title and body within the platform-safe limits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationContent {
    title: String,
    body: String,
}

impl NotificationContent {
    pub fn new(
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> Result<Self, NotificationContentError> {
        let (title, body) = (title.into(), body.into());
        if title.len() > MAX_TITLE_BYTES || body.len() > MAX_BODY_BYTES {
            return Err(NotificationContentError::TooLarge);
        }
        Ok(Self { title, body })
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn body(&self) -> &str {
        &self.body
    }
}

#[cfg(test)]
#[path = "tests/notification.rs"]
mod tests;
