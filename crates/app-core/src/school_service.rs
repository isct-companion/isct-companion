//! The result of reading one school page, as shells receive it in
//! `CoreEvent::SchoolIngest`. Reads themselves are sync keys
//! (`crate::sync_engine`).

use crate::lifecycle::SchoolSite;
use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

/// One extractor result. The wire contract is shared by desktop and mobile.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestOutcome {
    pub site: SchoolSite,
    pub kind: String,
    pub state: IngestState,
    pub changed: usize,
    pub message: String,
}

/// How one extractor result ended.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum IngestState {
    Ok,
    Unavailable,
    Maintenance,
    ReconnectRequired,
    Error,
}

#[cfg(feature = "ts")]
pub fn declarations(cfg: &ts_rs::Config) -> Vec<String> {
    vec![
        crate::contracts::SchoolPageAvailabilityState::decl(cfg),
        crate::contracts::SchoolPageAvailability::decl(cfg),
    ]
}
