mod apply;
mod client;
mod feeds;

pub use apply::{apply_class_schedules, apply_exam_feed, apply_important_dates, apply_reference};
pub use client::{ReferenceFetch, SubscriptionClient};
pub use feeds::{
    validate_academic_calendar, validate_class_schedules, validate_departments, validate_exam_feed,
    validate_important_dates, AcademicCalendar, CalendarDay, CalendarDayType, ClassScheduleEntry,
    ClassSchedules, ClassSession, Credits, DateRange, Department, Departments, ExamEntry, ExamFeed,
    ExamFeedKind, FeedSource, ImportantDateEntry, ImportantDates, PublishedExamFeed,
    RecommendationMark, RecommendedCourse, SourceLink,
};

use crate::data::DataError;

#[derive(Debug, thiserror::Error)]
pub enum SubscriptionError {
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
