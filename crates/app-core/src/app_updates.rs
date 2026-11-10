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

/// The App Store's record of this app in the Japanese storefront, where it
/// is listed (`approach/app-store-github-release`); iPhone and iPad share it.
pub const APP_STORE_LOOKUP: &str =
    "https://itunes.apple.com/lookup?bundleId=app.isctcompanion&country=jp";

/// A newer release for a phone or tablet: its version and where it comes
/// from (the APK on Android, the App Store page on iOS and iPadOS).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobileRelease {
    pub version: String,
    pub url: String,
}

/// Reads GitHub's latest-release record for Android: a published release newer
/// than `running` whose APK is attached. A release still waiting for its APK
/// (the Android build attaches it after the release is published) is not
/// offered yet, so nobody is sent to a download that does not exist.
pub fn android_release(
    release: &Value,
    running: &str,
) -> Result<Option<MobileRelease>, AppUpdateError> {
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
    Ok(apk.map(|url| MobileRelease {
        version: version.trim_start_matches('v').to_owned(),
        url: url.to_owned(),
    }))
}

/// The App Store's listing, when it names a version newer than `running`
/// and links to its page on the App Store. No listing (not yet released in
/// the storefront) offers nothing. The listing changes only once Apple
/// releases the version, so nobody is sent to a page still on the old one.
pub fn app_store_release(
    lookup: &Value,
    running: &str,
) -> Result<Option<MobileRelease>, AppUpdateError> {
    let Some(app) = lookup.pointer("/results/0") else {
        return Ok(None);
    };
    let (Some(version), Some(url)) = (
        app.get("version").and_then(Value::as_str),
        app.get("trackViewUrl").and_then(Value::as_str),
    ) else {
        return Ok(None);
    };
    if !url.starts_with("https://apps.apple.com/") || !newer_release(running, version)? {
        return Ok(None);
    }
    Ok(Some(MobileRelease {
        version: version.trim_start_matches('v').to_owned(),
        url: url.to_owned(),
    }))
}

async fn read_json(url: &str, accept: &str) -> Result<Value, AppUpdateError> {
    let network = |error: reqwest::Error| AppUpdateError::Network(error.to_string());
    crate::net::client_builder()
        .timeout(std::time::Duration::from_secs(20))
        .user_agent(crate::net::USER_AGENT)
        .build()
        .map_err(network)?
        .get(url)
        .header("Accept", accept)
        .send()
        .await
        .map_err(network)?
        .error_for_status()
        .map_err(network)?
        .json()
        .await
        .map_err(network)
}

/// Asks GitHub for a newer Android release with its APK.
pub async fn fetch_android(running: &str) -> Result<Option<MobileRelease>, AppUpdateError> {
    android_release(
        &read_json(RELEASE_API, "application/vnd.github+json").await?,
        running,
    )
}

/// Asks the App Store for a newer iOS and iPadOS release.
pub async fn fetch_app_store(running: &str) -> Result<Option<MobileRelease>, AppUpdateError> {
    app_store_release(
        &read_json(APP_STORE_LOOKUP, "application/json").await?,
        running,
    )
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

/// Whether this device looks for a newer version on its own; absent reads as
/// on. Turned off, only a check the person starts runs.
pub fn update_checks(workspace: &Value) -> bool {
    workspace
        .pointer("/settings/appUpdateChecks")
        .and_then(Value::as_bool)
        != Some(false)
}

/// The release the person chose to skip.
pub fn skipped_version(workspace: &Value) -> Option<&str> {
    workspace
        .pointer("/settings/skippedAppVersion")
        .and_then(Value::as_str)
        .map(|version| version.trim_start_matches('v'))
}

/// Whether to ask for the newest release now: the person asked (`manual`),
/// or checks are on and the last one is `check_due`.
pub fn check_now(
    workspace: &Value,
    last_checked: Option<&str>,
    now: DateTime<Utc>,
    manual: bool,
) -> bool {
    manual || (update_checks(workspace) && check_due(last_checked, now, false))
}

/// Whether a newer `version` a check found is offered in the update prompt:
/// only a check of its own found it, checks are on and the person did not
/// skip it. What a check the person started finds shows where they started
/// it, prompt or not.
pub fn prompts(workspace: &Value, version: &str, manual: bool) -> bool {
    !manual
        && update_checks(workspace)
        && skipped_version(workspace) != Some(version.trim_start_matches('v'))
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
