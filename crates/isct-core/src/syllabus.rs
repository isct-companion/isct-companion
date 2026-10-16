mod api;
pub(crate) mod model;
pub(crate) mod transport;

#[derive(Debug, thiserror::Error)]
pub enum SyllabusError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Network(String),
    #[error("{0}")]
    SiteChanged(String),
}

impl SyllabusError {
    pub(crate) fn invalid_input(message: impl Into<String>) -> Self {
        Self::InvalidInput(message.into())
    }

    pub(crate) fn network(message: impl Into<String>) -> Self {
        Self::Network(message.into())
    }
}

impl From<String> for SyllabusError {
    fn from(message: String) -> Self {
        Self::SiteChanged(message)
    }
}

impl From<&str> for SyllabusError {
    fn from(message: &str) -> Self {
        Self::SiteChanged(message.to_string())
    }
}

impl From<url::ParseError> for SyllabusError {
    fn from(error: url::ParseError) -> Self {
        Self::InvalidInput(error.to_string())
    }
}

impl From<reqwest::Error> for SyllabusError {
    fn from(error: reqwest::Error) -> Self {
        Self::Network(error.to_string())
    }
}

pub(crate) use crate::school_parser::syllabus as parser;

pub use api::{
    department_catalog, search_bilingual, sync_offering, sync_offering_summary, SyllabusFilters,
};
pub use model::{CatalogSearchItem, CatalogSearchResult, DepartmentSyncResult, SyncOfferingResult};

pub(crate) use api::utc_now;

#[cfg(test)]
#[path = "tests/syllabus.rs"]
mod tests;
