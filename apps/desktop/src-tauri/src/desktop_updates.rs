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

const RELEASE_API: &str =
    "https://api.github.com/repos/isct-companion/Isct-Companion-App/releases/latest";
const MANIFEST: &str =
    "https://github.com/isct-companion/Isct-Companion-App/releases/latest/download/latest.json";
const RELEASES: &str = "https://github.com/isct-companion/Isct-Companion-App/releases/latest";
const PUBLIC_KEY: &str = match option_env!("ISCT_UPDATER_PUBLIC_KEY") {
    Some(key) => key,
    None => "",
};

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
            signed_updates: !PUBLIC_KEY.trim().is_empty(),
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
        .map(|s| s.status.clone())
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
        let response = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("IsctCompanion/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| e.to_string())?
            .get(RELEASE_API)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?;
        let release: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
        let version = release["tag_name"]
            .as_str()
            .ok_or("Release version is missing.")?
            .to_string();
        if release["draft"] == true || release["prerelease"] == true {
            return Err("The release channel is not stable.".into());
        }
        let newer = isct_core::app_updates::newer_release(env!("CARGO_PKG_VERSION"), &version)
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
        if PUBLIC_KEY.trim().is_empty() {
            return snapshot(&app);
        }
        let update = app
            .updater_builder()
            .pubkey(PUBLIC_KEY)
            .endpoints(vec![MANIFEST
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
        if update.download_url.scheme() != "https"
            || update.download_url.host_str() != Some("github.com")
            || !update
                .download_url
                .path()
                .starts_with("/isct-companion/Isct-Companion-App/releases/download/")
        {
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
        if let Err(error) = update.install(bytes) {
            eprintln!("[updater] installation failed: {error}");
        }
    }
}
