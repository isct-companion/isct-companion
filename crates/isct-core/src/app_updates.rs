//! Shared update preference and admission policy; native ports own installers.
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum AppUpdateError {
    #[error("Invalid application version: {0}")]
    Version(#[from] semver::Error),
    #[error("The release could not be read: {0}")]
    Network(String),
}

/// GitHub's record of the newest published release, which every platform
/// reads to learn whether there is a newer version.
pub const RELEASE_API: &str =
    "https://api.github.com/repos/isct-companion/isct-companion/releases/latest";

/// The Android build of a newer release: its version and the APK to install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndroidRelease {
    pub version: String,
    pub apk_url: String,
}

/// Reads GitHub's latest-release record for Android: a published release newer
/// than `running` whose APK is attached. A release still waiting for its APK
/// (the Android build attaches it after the release is published) is not
/// offered yet, so nobody is sent to a download that does not exist.
pub fn android_release(
    release: &Value,
    running: &str,
) -> Result<Option<AndroidRelease>, AppUpdateError> {
    let Some(version) = release.get("tag_name").and_then(Value::as_str) else {
        return Ok(None);
    };
    let flag = |key: &str| release.get(key).and_then(Value::as_bool) == Some(true);
    if flag("draft") || flag("prerelease") || !newer_release(running, version)? {
        return Ok(None);
    }
    let apk = release
        .get("assets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|asset| {
            asset
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| {
                    name.starts_with("IsctCompanion_") && name.ends_with("_android.apk")
                })
        })
        .filter_map(|asset| asset.get("browser_download_url").and_then(Value::as_str))
        .find(|url| {
            url.starts_with("https://github.com/isct-companion/isct-companion/releases/download/")
        });
    Ok(apk.map(|url| AndroidRelease {
        version: version.trim_start_matches('v').to_owned(),
        apk_url: url.to_owned(),
    }))
}

/// Asks GitHub for a newer Android release when a check is due (`check_due`);
/// `None` for the time when it was not due.
pub async fn check_android(
    running: &str,
    last_checked: Option<&str>,
    now: DateTime<Utc>,
    manual: bool,
) -> Result<Option<Option<AndroidRelease>>, AppUpdateError> {
    if !check_due(last_checked, now, manual) {
        return Ok(None);
    }
    let network = |error: reqwest::Error| AppUpdateError::Network(error.to_string());
    let release: Value = crate::net::client_builder()
        .timeout(std::time::Duration::from_secs(20))
        .user_agent(crate::net::USER_AGENT)
        .build()
        .map_err(network)?
        .get(RELEASE_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(network)?
        .error_for_status()
        .map_err(network)?
        .json()
        .await
        .map_err(network)?;
    android_release(&release, running).map(Some)
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
