//! Native updater port. Tauri verifies signatures and installs native bundles;
//! Core decides version ordering, check cadence and the automatic preference.
use crate::core_ports;
use serde::Serialize;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::Duration,
};
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

const REPOSITORY: &str = "isct-companion/isct-companion";
const RELEASES: &str = "https://github.com/isct-companion/isct-companion/releases/latest";

struct UpdateSource {
    release_api: String,
    manifest: String,
    fixture_origin: Option<reqwest::Url>,
}
impl UpdateSource {
    fn current(_app: &AppHandle) -> Result<Self, String> {
        // Manual native verification uses signed fixtures on loopback. This
        // override is absent from release builds; the signature check is shared.
        #[cfg(debug_assertions)]
        if let Some(origin) = _app
            .config()
            .plugins
            .0
            .get("updater-test-fixture")
            .and_then(|config| config.get("origin"))
            .and_then(serde_json::Value::as_str)
        {
            let url = reqwest::Url::parse(&origin).map_err(|e| e.to_string())?;
            if url.scheme() != "http"
                || url.host_str() != Some("127.0.0.1")
                || url.port().is_none()
                || url.path() != "/"
                || url.query().is_some()
                || url.fragment().is_some()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err("The updater fixture must use a loopback HTTP origin.".into());
            }
            return Ok(Self {
                release_api: url
                    .join("release.json")
                    .map_err(|e| e.to_string())?
                    .to_string(),
                manifest: url
                    .join("latest.json")
                    .map_err(|e| e.to_string())?
                    .to_string(),
                fixture_origin: Some(url),
            });
        }
        Ok(Self {
            release_api: format!("https://api.github.com/repos/{REPOSITORY}/releases/latest"),
            manifest: format!(
                "https://github.com/{REPOSITORY}/releases/latest/download/latest.json"
            ),
            fixture_origin: None,
        })
    }
    fn approves(&self, url: &reqwest::Url) -> bool {
        if let Some(origin) = &self.fixture_origin {
            return url.origin() == origin.origin() && url.path().starts_with("/bundles/");
        }
        url.scheme() == "https"
            && url.host_str() == Some("github.com")
            && url
                .path()
                .starts_with(&format!("/{REPOSITORY}/releases/download/"))
    }
}
fn public_key(app: &AppHandle) -> Option<String> {
    app.config()
        .plugins
        .0
        .get("updater")
        .and_then(|config| config.get("pubkey"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    phase: &'static str,
    version: Option<String>,
    message: Option<String>,
    release_url: &'static str,
    signed_updates: bool,
}
impl Default for UpdateStatus {
    fn default() -> Self {
        Self {
            phase: "idle",
            version: None,
            message: None,
            release_url: RELEASES,
            signed_updates: false,
        }
    }
}
#[derive(Default)]
struct NativeState {
    status: UpdateStatus,
    pending: Option<(Update, Vec<u8>)>,
    install_requested: bool,
    last_checked: Option<String>,
}
#[derive(Default)]
pub struct DesktopUpdates {
    state: Mutex<NativeState>,
    busy: AtomicBool,
}
struct Busy<'a>(&'a AtomicBool);
impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub fn snapshot(app: &AppHandle) -> Result<UpdateStatus, String> {
    app.state::<DesktopUpdates>()
        .state
        .lock()
        .map(|s| {
            let mut status = s.status.clone();
            status.signed_updates = public_key(app).is_some();
            status
        })
        .map_err(|_| "Update state is unavailable.".into())
}
fn phase(
    app: &AppHandle,
    phase: &'static str,
    version: Option<String>,
    message: Option<String>,
) -> Result<UpdateStatus, String> {
    let updates = app.state::<DesktopUpdates>();
    let mut state = updates
        .state
        .lock()
        .map_err(|_| "Update state is unavailable.")?;
    state.status.signed_updates = public_key(app).is_some();
    state.status.phase = phase;
    state.status.version = version;
    state.status.message = message;
    Ok(state.status.clone())
}

pub async fn check(app: AppHandle, manual: bool, download: bool) -> Result<UpdateStatus, String> {
    let updates = app.state::<DesktopUpdates>();
    if updates
        .busy
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return snapshot(&app);
    }
    let _busy = Busy(&updates.busy);
    if snapshot(&app)?.phase == "ready" {
        return snapshot(&app);
    }
    let core = core_ports::core(&app);
    let workspace = core.store.load_workspace().map_err(|e| e.to_string())?;
    let checked = updates
        .state
        .lock()
        .map_err(|_| "Update state is unavailable.")?
        .last_checked
        .clone();
    let waiting_download = snapshot(&app)?.phase == "available"
        && isct_core::app_updates::automatic_updates(&workspace);
    if !isct_core::app_updates::check_due(
        checked.as_deref(),
        chrono::Utc::now(),
        manual || download || waiting_download,
    ) {
        return snapshot(&app);
    }
    phase(&app, "checking", None, None)?;
    let result = async {
        let source = UpdateSource::current(&app)?;
        let response = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("IsctCompanion/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| e.to_string())?
            .get(&source.release_api)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            updates
                .state
                .lock()
                .map_err(|_| "Update state is unavailable.")?
                .last_checked = Some(chrono::Utc::now().to_rfc3339());
            return phase(&app, "unpublished", None, None);
        }
        let response = response.error_for_status().map_err(|e| e.to_string())?;
        let release: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
        let version = release["tag_name"]
            .as_str()
            .ok_or("Release version is missing.")?
            .to_string();
        if release["draft"] == true || release["prerelease"] == true {
            return Err("The release channel is not stable.".into());
        }
        let newer = isct_core::app_updates::newer_release(
            &app.package_info().version.to_string(),
            &version,
        )
        .map_err(|e| e.to_string())?;
        updates
            .state
            .lock()
            .map_err(|_| "Update state is unavailable.")?
            .last_checked = Some(chrono::Utc::now().to_rfc3339());
        if !newer {
            return phase(&app, "current", Some(version), None);
        }
        phase(&app, "available", Some(version.clone()), None)?;
        let Some(public_key) = public_key(&app) else {
            return snapshot(&app);
        };
        let update = app
            .updater_builder()
            .pubkey(public_key)
            .endpoints(vec![source
                .manifest
                .parse()
                .map_err(|_| "Invalid update endpoint.")?])
            .map_err(|e| e.to_string())?
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())?
            .check()
            .await
            .map_err(|e| e.to_string())?
            .ok_or("The signed update manifest is not available.")?;
        if !source.approves(&update.download_url) {
            return Err("The update download source is not approved.".into());
        }
        if isct_core::app_updates::automatic_updates(&workspace) || download {
            phase(&app, "downloading", Some(version.clone()), None)?;
            let bytes = update
                .download(|_, _| {}, || {})
                .await
                .map_err(|e| e.to_string())?;
            updates
                .state
                .lock()
                .map_err(|_| "Update state is unavailable.")?
                .pending = Some((update, bytes));
            phase(&app, "ready", Some(version), None)
        } else {
            snapshot(&app)
        }
    }
    .await;
    match result {
        Ok(status) => Ok(status),
        Err(error) => {
            phase(&app, "error", snapshot(&app)?.version, Some(error.clone()))?;
            Err(error)
        }
    }
}

pub fn request_install(app: &AppHandle) -> Result<(), String> {
    let updates = app.state::<DesktopUpdates>();
    let mut state = updates
        .state
        .lock()
        .map_err(|_| "Update state is unavailable.")?;
    if state.pending.is_none() {
        return Err("No verified update is ready.".into());
    }
    state.install_requested = true;
    drop(state);
    // Exit goes through the application's existing owned shutdown and save path.
    app.exit(0);
    Ok(())
}

pub fn install_on_exit(app: &AppHandle) {
    let automatic = core_ports::core(app)
        .store
        .load_workspace()
        .map(|w| isct_core::app_updates::automatic_updates(&w))
        .unwrap_or(false);
    let updates = app.state::<DesktopUpdates>();
    let pending = updates.state.lock().ok().and_then(|mut s| {
        if automatic || s.install_requested {
            s.pending.take()
        } else {
            None
        }
    });
    if let Some((update, bytes)) = pending {
        record_install(app, "installing", &update.version, None);
        match update.install(bytes) {
            Ok(()) => record_install(app, "installed", &update.version, None),
            Err(error) => {
                record_install(app, "error", &update.version, Some(error.to_string()));
                log::error!("[updater] installation failed: {error}");
            }
        }
    }
}

fn record_install(app: &AppHandle, phase: &str, version: &str, message: Option<String>) {
    if let Ok(directory) = app.path().app_data_dir() {
        let result = serde_json::json!({"phase":phase,"version":version,"message":message,"at":chrono::Utc::now().to_rfc3339()});
        if let Err(error) = std::fs::write(
            directory.join("updater-install-result.json"),
            result.to_string(),
        ) {
            log::error!("[updater] could not record installation result: {error}");
        }
    }
}
