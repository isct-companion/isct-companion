//! Shared update preference and admission policy; native ports own installers.
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum AppUpdateError {
    #[error("Invalid application version: {0}")]
    Version(#[from] semver::Error),
}

pub fn newer_release(current: &str, available: &str) -> Result<bool, AppUpdateError> {
    Ok(semver::Version::parse(available.trim_start_matches('v'))?
        > semver::Version::parse(current.trim_start_matches('v'))?)
}

pub fn automatic_updates(workspace: &Value) -> bool {
    workspace
        .pointer("/settings/automaticAppUpdates")
        .and_then(Value::as_bool)
        == Some(true)
}

pub fn check_due(last_checked: Option<&str>, now: DateTime<Utc>, manual: bool) -> bool {
    manual
        || last_checked
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .is_none_or(|last| now.signed_duration_since(last) >= Duration::hours(12))
}

#[cfg(test)]
#[path = "tests/app_updates.rs"]
mod tests;
