mod apply;
mod client;
pub(crate) mod feeds;

pub(crate) use {
    apply::apply_class_schedules, apply::apply_exam_feed, apply::apply_important_dates,
    apply::apply_reference,
};
pub(crate) use {client::ReferenceFetch, client::SubscriptionClient};
pub(crate) use {
    feeds::validate_academic_calendar, feeds::validate_class_schedules,
    feeds::validate_departments, feeds::validate_exam_feed, feeds::validate_important_dates,
    feeds::AcademicCalendar, feeds::ClassSchedules, feeds::Departments, feeds::ExamFeedKind,
    feeds::ImportantDates, feeds::PublishedExamFeed,
};

use crate::data::DataError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum SubscriptionError {
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    Network(String),
    #[error("{0}")]
    Storage(String),
}

impl From<String> for SubscriptionError {
    fn from(message: String) -> Self {
        Self::Validation(message)
    }
}

impl From<&str> for SubscriptionError {
    fn from(message: &str) -> Self {
        Self::Validation(message.to_string())
    }
}

impl From<DataError> for SubscriptionError {
    fn from(error: DataError) -> Self {
        Self::Storage(error.to_string())
    }
}

impl From<rusqlite::Error> for SubscriptionError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

impl From<serde_json::Error> for SubscriptionError {
    fn from(error: serde_json::Error) -> Self {
        Self::Validation(error.to_string())
    }
}

impl From<reqwest::Error> for SubscriptionError {
    fn from(error: reqwest::Error) -> Self {
        Self::Network(error.to_string())
    }
}

#[cfg(test)]
#[path = "../tests/subscriptions.rs"]
mod tests;
