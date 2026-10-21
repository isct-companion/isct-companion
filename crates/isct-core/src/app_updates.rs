//! Shared update preference and admission policy; native ports own installers.
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum AppUpdateError {
    #[error("Invalid application version: {0}")]
    Version(#[from] semver::Error),
}

pub fn newer_release(current: &str, available: &str) -> Result<bool, AppUpdateError> {
    let mut current = semver::Version::parse(current.trim_start_matches('v'))?;
    // A development build (`1.1.3-dev.<stamp>`) is built after the release it
    // names, so it compares as that release rather than sorting below it.
    let pre = current.pre.as_str();
    if let Some(dev) = pre.split('.').position(|id| id == "dev") {
        let base = pre.split('.').take(dev).collect::<Vec<_>>().join(".");
        current.pre = semver::Prerelease::new(&base)?;
    }
    Ok(semver::Version::parse(available.trim_start_matches('v'))? > current)
}

pub fn automatic_updates(workspace: &Value) -> bool {
    workspace
        .pointer("/settings/automaticAppUpdates")
        .and_then(Value::as_bool)
        == Some(true)
}

/// What the native installer port records just before the process exits. On
/// Windows the installer runs after exit, so the record stays at `installing`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InstallRecord {
    pub phase: InstallPhase,
    pub version: String,
    pub message: Option<String>,
    pub at: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InstallPhase {
    Installing,
    Installed,
    Error,
}

/// The previous install attempt, judged against the version now running.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub enum LastInstall {
    Installed {
        version: String,
    },
    /// The installer was started but this launch is still the old version:
    /// it was blocked, cancelled or failed after the app exited.
    Incomplete {
        version: String,
    },
    Failed {
        version: String,
        message: String,
    },
}

pub fn last_install(record: &InstallRecord, running: &str) -> Result<LastInstall, AppUpdateError> {
    let version = record.version.clone();
    if !newer_release(running, &record.version)? {
        return Ok(LastInstall::Installed { version });
    }
    Ok(match (record.phase, &record.message) {
        (InstallPhase::Error, Some(message)) => LastInstall::Failed {
            version,
            message: message.clone(),
        },
        _ => LastInstall::Incomplete { version },
    })
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
