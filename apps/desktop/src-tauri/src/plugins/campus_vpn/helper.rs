//! The helper's local socket behind app-core's `VpnPort`, and the watch that
//! checks a tunnel that is up. Decisions and the APM sign-in stay in
//! app-core; this file and its platform halves (`macos.rs`: launchd and a
//! Unix socket, `windows.rs`: a service and a named pipe) do the IO.
//! Blocking calls run under `spawn_blocking`.
use app_core::vpn::{HelperReport, VpnCheck, VpnError, VpnNotice, VpnPort, VpnService, VpnState};
use std::{
    io::{BufRead, BufReader, ErrorKind, Read, Write},
    sync::Arc,
    time::Duration,
};
use vpn_protocol::{Reply, Request, SessionCookie};

#[cfg(target_os = "macos")]
use super::macos as platform;
#[cfg(target_os = "windows")]
use super::windows as platform;
pub(crate) use platform::{run_install, standby_stream, HELPER};

/// Where one install press left the helper.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Install {
    /// Installed or updated; the helper starts on its own.
    Ready,
    /// The person declined the UAC prompt.
    #[cfg(target_os = "windows")]
    Declined,
    /// Registered, and waiting for the person to allow it in the system's
    /// settings, which are open now (Login Items).
    #[cfg(target_os = "macos")]
    NeedsApproval,
    /// The old registration would not go, so a new one cannot take its
    /// place; launchd keeps the old copy's program.
    #[cfg(target_os = "macos")]
    Stuck,
}

/// Status answers at once and a stop within the helper's 5-second wait for
/// openconnect. A start waits up to its own 30-second deadline, plus such a
/// stop of an old tunnel before it and of a failed one after it.
const QUICK: Duration = Duration::from_secs(10);
const START: Duration = Duration::from_secs(45);

/// One request line out, one reply line back. The line carries the session
/// cookie on a socket only the helper serves; it is not logged and not kept
/// after this call.
pub(crate) fn converse(
    mut stream: impl Read + Write,
    request: &Request,
) -> Result<HelperReport, VpnError> {
    let line =
        serde_json::to_string(request).map_err(|error| VpnError::Protocol(error.to_string()))?;
    writeln!(stream, "{line}").map_err(io_error)?;
    let mut reply = String::new();
    BufReader::new(stream)
        .read_line(&mut reply)
        .map_err(io_error)?;
    // Hanging up without a line is a helper that went away mid-request.
    if reply.trim().is_empty() {
        return Err(VpnError::HelperUnreachable(
            "the helper closed the connection".into(),
        ));
    }
    let reply: Reply = serde_json::from_str(reply.trim())
        .map_err(|error| VpnError::Protocol(error.to_string()))?;
    Ok(HelperReport {
        state: reply.state,
        address: reply.address,
        detail: reply.detail,
        expires_at: reply
            .expires
            .and_then(|at| chrono::DateTime::from_timestamp(at, 0)),
        revision: reply.revision,
        ended: reply.ended,
        program: reply.program.map(std::path::PathBuf::from),
    })
}

pub(crate) fn io_error(error: std::io::Error) -> VpnError {
    match error.kind() {
        ErrorKind::TimedOut | ErrorKind::WouldBlock => VpnError::Timeout,
        _ => VpnError::HelperUnreachable(error.to_string()),
    }
}

impl VpnPort for platform::HelperSocket {
    fn status(&self) -> Result<HelperReport, VpnError> {
        self.exchange(&Request::Status, QUICK)
    }

    fn start(&self, cookie: &SessionCookie) -> Result<HelperReport, VpnError> {
        self.exchange(
            &Request::Start {
                cookie: cookie.clone(),
            },
            START,
        )
    }

    fn stop(&self) -> Result<HelperReport, VpnError> {
        self.exchange(&Request::Stop, QUICK)
    }

    fn routes(
        &self,
        v4: &[std::net::Ipv4Addr],
        v6: &[std::net::Ipv6Addr],
    ) -> Result<HelperReport, VpnError> {
        self.exchange(
            &Request::Routes {
                v4: v4.to_vec(),
                v6: v6.to_vec(),
            },
            QUICK,
        )
    }
}

/// The service over the installed helper. `own_helper` is the helper file
/// this app registers (macOS: the bundled one, which launchd runs in place);
/// Core takes a helper answering from any other file for another copy's.
pub(crate) fn service(own_helper: Option<std::path::PathBuf>) -> Arc<VpnService> {
    let service = VpnService::new(Arc::new(platform::HelperSocket::installed()));
    Arc::new(match own_helper {
        Some(own) => service.with_own_helper(own),
        None => service,
    })
}

/// How often a tunnel that is up is checked: a local socket read.
const WATCH_EVERY: Duration = Duration::from_secs(15);

/// What the webview hears without asking: the state after each check, and a
/// reminder when Core says one is due. The page words the reminder in the UI
/// language and shows it as a system notification.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct NoticePayload {
    kind: &'static str,
    minutes: Option<i64>,
    code: Option<&'static str>,
}

impl From<VpnNotice> for NoticePayload {
    fn from(notice: VpnNotice) -> Self {
        let plain = |kind| Self {
            kind,
            minutes: None,
            code: None,
        };
        match notice {
            VpnNotice::EndingSoon { minutes } => Self {
                minutes: Some(minutes),
                ..plain("ending")
            },
            VpnNotice::Ended => plain("ended"),
            VpnNotice::Unused => plain("unused"),
            VpnNotice::DemandStopped { code } => Self {
                code: Some(code),
                ..plain("demandStopped")
            },
        }
    }
}

/// Whether the last state told was a connected tunnel.
static TUNNEL_UP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Tells the main window where the VPN stands, and a reminder if one is due.
pub(crate) fn tell(
    app: &tauri::AppHandle,
    service: &VpnService,
    state: VpnState,
    notice: Option<VpnNotice>,
) {
    use tauri::Emitter;
    // A tunnel that came up is a new way to the school: Core may probe a
    // school host it paused, instead of waiting out the pause.
    let connected = matches!(state, VpnState::Connected { .. });
    if connected && !TUNNEL_UP.swap(true, std::sync::atomic::Ordering::AcqRel) {
        crate::core_ports::core(app).sync_hint(app_core::plan::Hint::NetworkPath);
    } else if !connected {
        TUNNEL_UP.store(false, std::sync::atomic::Ordering::Release);
    }
    let status = super::report(service, state);
    if let Err(error) = app.emit_to("main", "vpn-status", &status) {
        log::warn!("[vpn] status was not delivered: {error}");
    }
    if let Some(notice) = notice {
        if let Err(error) = app.emit_to("main", "vpn-notice", NoticePayload::from(notice)) {
            log::warn!("[vpn] notice was not delivered: {error}");
        }
    }
}

/// Reads the helper once at launch (a tunnel may have outlived the app), then
/// checks it while a tunnel is up.
pub(crate) fn watch(app: tauri::AppHandle) {
    use tauri::Manager;
    tauri::async_runtime::spawn(async move {
        let service = app.state::<Arc<VpnService>>().inner().clone();
        let first = service.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || first.refresh()).await;
        let mut ticks = tokio::time::interval(WATCH_EVERY);
        loop {
            ticks.tick().await;
            if !matches!(
                service.state(),
                VpnState::Connected { .. } | VpnState::Connecting
            ) {
                continue;
            }
            let checking = service.clone();
            let Ok(VpnCheck { state, notice }) = tauri::async_runtime::spawn_blocking(move || {
                checking.watch(app_core::clock::now())
            })
            .await
            else {
                continue;
            };
            tell(&app, &service, state, notice);
        }
    });
}
