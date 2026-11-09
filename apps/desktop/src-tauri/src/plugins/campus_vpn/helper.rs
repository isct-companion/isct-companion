//! The helper's local socket behind app-core's `VpnPort`, and the watch that
//! checks a tunnel that is up. Decisions and the APM sign-in stay in
//! app-core; this file and its platform halves (`macos.rs`: launchd and a
//! Unix socket, `windows.rs`: a service and a named pipe) do the IO.
//! Blocking calls run under `spawn_blocking`.
use app_core::vpn::{
    HelperReport, VpnAction, VpnCheck, VpnError, VpnNotice, VpnPort, VpnService, VpnState,
};
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
pub(crate) use platform::{run_install, HELPER};

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
}

pub(crate) fn service() -> Arc<VpnService> {
    Arc::new(VpnService::new(Arc::new(
        platform::HelperSocket::installed(),
    )))
}

/// How often a tunnel that is up, or auto-continue, is checked: a local
/// socket read, and how long a dropped tunnel waits to be reconnected.
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
            VpnNotice::KeepEnded => plain("keepEnded"),
            VpnNotice::KeepStopped { code } => Self {
                code: Some(code),
                ..plain("keepStopped")
            },
        }
    }
}

/// Carries out what a check asked for. Core decided; this does the IO.
async fn act(
    app: &tauri::AppHandle,
    service: &Arc<VpnService>,
    check: VpnCheck,
) -> (VpnState, Option<VpnNotice>) {
    let VpnCheck {
        state,
        notice,
        action,
    } = check;
    let blocking = |work: fn(&VpnService) -> VpnState| {
        let service = service.clone();
        tauri::async_runtime::spawn_blocking(move || work(&service))
    };
    match action {
        Some(VpnAction::Disconnect) => {
            let state = blocking(|service| service.disconnect())
                .await
                .unwrap_or(state);
            (state, notice)
        }
        Some(VpnAction::Reconnect) => {
            // A press in progress is doing this already.
            let Some(_login) = service.begin_login() else {
                return (state, notice);
            };
            let outcome = match crate::core_ports::core(app)
                .sessions()
                .campus_vpn_login()
                .await
            {
                // A disconnect while signing in: no tunnel behind its back.
                Ok(_) if !service.keeps() => service.state(),
                Ok(login) => {
                    let connecting = service.clone();
                    tauri::async_runtime::spawn_blocking(move || connecting.connect(&login.cookie))
                        .await
                        .unwrap_or(state)
                }
                Err(error) => service.failed(VpnError::Login(error)),
            };
            let stopped = service.reconnected(&outcome, chrono::Utc::now());
            (outcome, notice.or(stopped))
        }
        None => (state, notice),
    }
}

/// Reads the helper once at launch (a tunnel may have outlived the app), then
/// checks it while a tunnel is up or auto-continue is on.
pub(crate) fn watch(app: tauri::AppHandle) {
    use tauri::{Emitter, Manager};
    tauri::async_runtime::spawn(async move {
        let service = app.state::<Arc<VpnService>>().inner().clone();
        let first = service.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || first.refresh()).await;
        let mut ticks = tokio::time::interval(WATCH_EVERY);
        loop {
            ticks.tick().await;
            if !service.keeps()
                && !matches!(
                    service.state(),
                    VpnState::Connected { .. } | VpnState::Connecting
                )
            {
                continue;
            }
            let checking = service.clone();
            let Ok(check) =
                tauri::async_runtime::spawn_blocking(move || checking.watch(chrono::Utc::now()))
                    .await
            else {
                continue;
            };
            let (state, notice) = act(&app, &service, check).await;
            let status = super::report(&service, state);
            if let Err(error) = app.emit_to("main", "vpn-status", &status) {
                log::warn!("[vpn] status was not delivered: {error}");
            }
            if let Some(notice) = notice {
                if let Err(error) = app.emit_to("main", "vpn-notice", NoticePayload::from(notice)) {
                    log::warn!("[vpn] notice was not delivered: {error}");
                }
            }
        }
    });
}
