//! The campus VPN tool, as the `campus-vpn` plugin
//! (`plugin:campus-vpn|status`, `connect`, `disconnect`, `on_demand`,
//! `install`): install once, then one press signs in to the APM with the
//! saved Tokyo Tech account and connects, or on-demand connecting signs in
//! when something uses the campus network (`standby`). The helper ends any
//! tunnel left unused. Decisions are app-core's; this plugin carries them to
//! the webview and does the IO (`helper`).
use crate::{core_ports::core, require_local_main, CommandError};
use app_core::vpn::{VpnError, VpnService, VpnState};
use serde::Serialize;
use std::sync::Arc;
use tauri::Manager;

mod helper;
#[cfg(target_os = "macos")]
mod macos;
mod standby;
#[cfg(target_os = "windows")]
mod windows;

pub fn init() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("campus-vpn")
        .invoke_handler(tauri::generate_handler![
            status, connect, disconnect, on_demand, install
        ])
        .setup(|app, _| {
            app.manage(helper::service());
            super::grant_main(app, "campus-vpn")?;
            Ok(())
        })
        // After the app's setup: the watch reads Core's sessions and emits to
        // the main window, both of which exist by then.
        .on_event(|app, event| {
            if let tauri::RunEvent::Ready = event {
                helper::watch(app.clone());
                // This device's choice, kept across launches.
                let stored = core(app)
                    .store
                    .device_preference("/campusVpnOnDemand")
                    .unwrap_or_else(|error| {
                        log::warn!("[vpn] on-demand preference is unreadable: {error}");
                        None
                    });
                if stored.and_then(|value| value.as_bool()) == Some(true) {
                    service(app).set_on_demand(true);
                    standby::ensure(app.clone());
                }
            }
        })
        .build()
}

/// What the webview renders: a stable state name plus, for failures, the
/// error code the UI keys its guidance on.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct VpnStatus {
    state: &'static str,
    address: Option<String>,
    /// When the APM session ends (RFC 3339), for the countdown.
    expires_at: Option<String>,
    /// Whether on-demand connecting is on now; it turns itself off on what
    /// retrying cannot fix.
    on_demand: bool,
    /// While on-demand has spent the hour's sign-ins, when it resumes
    /// (RFC 3339).
    demand_paused_until: Option<String>,
    /// On the campus network, where the VPN is not needed.
    on_campus: bool,
    error: Option<&'static str>,
    detail: Option<String>,
}

impl From<VpnState> for VpnStatus {
    fn from(state: VpnState) -> Self {
        let blank = |state| Self {
            state,
            address: None,
            expires_at: None,
            on_demand: false,
            demand_paused_until: None,
            on_campus: false,
            error: None,
            detail: None,
        };
        match state {
            VpnState::Disconnected => blank("disconnected"),
            VpnState::Connecting => blank("connecting"),
            VpnState::Connected {
                address,
                expires_at,
            } => Self {
                address,
                expires_at: expires_at.map(|at| at.to_rfc3339()),
                ..blank("connected")
            },
            VpnState::Error(error) => Self {
                error: Some(error.code()),
                detail: Some(error.to_string()),
                ..blank("error")
            },
        }
    }
}

/// This device's addresses, for Core to tell the campus network apart.
pub(crate) fn addresses() -> Vec<std::net::IpAddr> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .map(|entry| entry.ip())
        .collect()
}

/// The webview's status: `state` with whether on-demand connecting is on.
fn report(service: &VpnService, state: VpnState) -> VpnStatus {
    VpnStatus {
        on_demand: service.on_demand(),
        on_campus: service.network(addresses()),
        demand_paused_until: service
            .demand_paused_until(chrono::Utc::now())
            .map(|at| at.to_rfc3339()),
        ..state.into()
    }
}

fn service(app: &tauri::AppHandle) -> Arc<VpnService> {
    app.state::<Arc<VpnService>>().inner().clone()
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, CommandError> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| CommandError::from(error.to_string()))
}

#[tauri::command]
async fn status(webview: tauri::Webview, app: tauri::AppHandle) -> Result<VpnStatus, CommandError> {
    require_local_main(&webview)?;
    let status = settled(service(&app), VpnService::refresh).await?;
    // A build without the payload (Intel Macs) has nothing to offer unless a
    // helper is already installed.
    if status.error == Some("helper_not_installed") && !payload(&app)?.exists() {
        return Ok(VpnStatus {
            state: "unavailable",
            error: None,
            detail: None,
            ..status
        });
    }
    Ok(status)
}

/// The bundled helper. `tauri.mjs` bundles `resources/vpn/` under its own path.
fn payload(app: &tauri::AppHandle) -> Result<std::path::PathBuf, CommandError> {
    Ok(app
        .path()
        .resource_dir()
        .map_err(|error| CommandError::from(error.to_string()))?
        .join("resources/vpn")
        .join(helper::HELPER))
}

/// Runs `work` off the main thread and reports where it left the service.
async fn settled(
    service: Arc<VpnService>,
    work: fn(&VpnService) -> VpnState,
) -> Result<VpnStatus, CommandError> {
    let running = service.clone();
    let state = blocking(move || work(&running)).await?;
    Ok(report(&service, state))
}

/// Signs in to the APM and connects. The helper is asked first: a login is
/// one-time, so none is spent on a missing helper or a tunnel already up.
#[tauri::command]
async fn connect(
    webview: tauri::Webview,
    app: tauri::AppHandle,
) -> Result<VpnStatus, CommandError> {
    require_local_main(&webview)?;
    let service = service(&app);
    let before = {
        let service = service.clone();
        blocking(move || service.refresh()).await?
    };
    if matches!(
        before,
        VpnState::Error(
            VpnError::HelperNotInstalled
                | VpnError::HelperOutdated
                | VpnError::HelperUnreachable(_)
        )
    ) || !service.needs_login()
    {
        return Ok(report(&service, before));
    }
    // A reconnect in progress is doing this already; sessions are one-time.
    let Some(_login) = service.begin_login() else {
        return Ok(report(&service, VpnState::Connecting));
    };
    let login = match core(&app).sessions().campus_vpn_login().await {
        Ok(login) => login,
        Err(error) => {
            let state = service.failed(VpnError::Login(error));
            return Ok(report(&service, state));
        }
    };
    let connecting = service.clone();
    let state = blocking(move || connecting.connect(&login.cookie)).await?;
    Ok(report(&service, state))
}

/// Turns on-demand connecting on or off for now; the page keeps the choice
/// in this device's preferences, which the next launch reads.
#[tauri::command]
async fn on_demand(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    enabled: bool,
) -> Result<VpnStatus, CommandError> {
    require_local_main(&webview)?;
    let service = service(&app);
    service.set_on_demand(enabled);
    if enabled {
        standby::ensure(app.clone());
    }
    Ok(report(&service, service.state()))
}

#[tauri::command]
async fn disconnect(
    webview: tauri::Webview,
    app: tauri::AppHandle,
) -> Result<VpnStatus, CommandError> {
    require_local_main(&webview)?;
    settled(service(&app), VpnService::disconnect).await
}

/// One administrator prompt installs or updates the helper and its
/// openconnect. A cancelled prompt returns the unchanged status.
#[tauri::command]
async fn install(
    webview: tauri::Webview,
    app: tauri::AppHandle,
) -> Result<VpnStatus, CommandError> {
    require_local_main(&webview)?;
    let helper = payload(&app)?;
    if !helper.exists() {
        return Err("this build does not include the campus VPN helper".into());
    }
    let resources = helper
        .parent()
        .ok_or("this build does not include the campus VPN helper")?
        .to_path_buf();
    let installed = blocking(move || helper::run_install(&resources))
        .await?
        .map_err(CommandError::from)?;
    let service = service(&app);
    if !installed {
        return settled(service, VpnService::refresh).await;
    }
    // launchd and the service manager start the helper asynchronously; give
    // it a moment to listen.
    for _ in 0..10 {
        let reading = service.clone();
        let state = blocking(move || reading.refresh()).await?;
        if !matches!(
            state,
            VpnState::Error(VpnError::HelperNotInstalled | VpnError::HelperUnreachable(_))
        ) {
            return Ok(report(&service, state));
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    settled(service, VpnService::refresh).await
}
