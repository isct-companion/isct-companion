//! The Windows system printer: a queue on the inbox IPP class driver, so any
//! app's Print dialog reaches the campus printer. Windows renders the job to
//! PDF and posts it over IPP to a receiver on 127.0.0.1 that this plugin
//! runs while the app does; Core (`app_core::campus_printer::ipp`) answers
//! IPP, bills and sends, and answers each job once its send has ended. This
//! module serves a fixed loopback URL, installs the queue behind UAC and
//! tells the person about a job that did not print.
use super::{CampusPrinterSetup, Setup};
use crate::{core_ports, CommandError};
use ::windows::{
    core::{w, HSTRING, PCWSTR},
    Win32::{
        Foundation::{CloseHandle, ERROR_CANCELLED, WAIT_OBJECT_0},
        System::Threading::{GetExitCodeProcess, WaitForSingleObject},
        UI::{
            Shell::{
                ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
            },
            WindowsAndMessaging::SW_HIDE,
        },
    },
};
use app_core::campus_printer::{
    ipp::{CampusPrintQueue, Failure},
    CampusPrintMessage, PrinterAccount, CAPABILITIES,
};
use axum::{
    body::to_bytes,
    extract::{Request, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use base64::Engine;
use std::{
    net::Ipv4Addr, os::windows::process::CommandExt, path::PathBuf, process::Command, sync::Arc,
    time::Duration,
};
use tauri::async_runtime::{self, JoinHandle};
use tauri::Manager;
use tokio::{net::TcpListener, sync::Semaphore};
use tokio_util::sync::CancellationToken;

/// Below Windows' default dynamic range (49152+), where Hyper-V and WSL
/// reserve ports at boot, and above the well-known service ports.
const PORT: u16 = 24_761;
const IPP_PATH: &str = "/ipp/print";
const PRINTER_NAME: &str = "IsctCompanion Campus Printer";
const PRINTER_UUID: &str = "urn:uuid:c3581d21-8147-4bdb-b39f-bb74fe36fc19";

struct PrintServer {
    task: Result<JoinHandle<()>, String>,
    stop: CancellationToken,
}

#[derive(Clone)]
struct HttpState {
    app: tauri::AppHandle,
    queue: Arc<CampusPrintQueue>,
    requests: Arc<Semaphore>,
    small_requests: Arc<Semaphore>,
}

async fn receive(State(state): State<HttpState>, request: Request) -> Response {
    // `application/ipp` may carry parameters.
    if !request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim_start().starts_with("application/ipp"))
    {
        log::warn!("[campus-printer] refused a request that is not IPP");
        return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
    }
    // Small, explicitly sized IPP polls must not wait behind a document,
    // which is answered only once it has been sent. A missing or larger
    // length is treated as a document: one at a time, so one is in memory.
    let small = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length <= 65_536);
    let slots = if small {
        &state.small_requests
    } else {
        &state.requests
    };
    let Ok(permit) = slots.clone().acquire_owned().await else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let limit = if small {
        65_536
    } else {
        CAPABILITIES.max_document_bytes + 65_536
    };
    let body = match tokio::time::timeout(BODY_TIMEOUT, to_bytes(request.into_body(), limit)).await
    {
        Ok(Ok(body)) => body,
        Ok(Err(error)) => {
            log::warn!("[campus-printer] refused a request body: {error}");
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        }
        Err(_) => {
            log::warn!("[campus-printer] a request body did not arrive within {BODY_TIMEOUT:?}");
            return StatusCode::REQUEST_TIMEOUT.into_response();
        }
    };
    // In its own task, holding the permit: a client that hangs up while a
    // document is sending must not drop the job's end or its notification.
    let handled = tokio::spawn(async move {
        let _permit = permit;
        let reply = state.queue.handle(body).await?;
        if let Some(Failure { title, text }) = reply.failed {
            let app = state.app.clone();
            tauri::async_runtime::spawn(async move {
                notify(&app, format!("{title}: {text}")).await;
            });
        }
        Ok::<_, app_core::campus_printer::ipp::PrintQueueError>(reply.body)
    })
    .await;
    match handled {
        Ok(Ok(body)) => ([(header::CONTENT_TYPE, "application/ipp")], body).into_response(),
        Ok(Err(error)) => {
            log::warn!("[campus-printer] invalid IPP request: {error}");
            StatusCode::BAD_REQUEST.into_response()
        }
        Err(error) => {
            log::error!("[campus-printer] the IPP request task failed: {error}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Windows streams the document while it renders; ordinary documents are
/// done well within this.
const BODY_TIMEOUT: Duration = Duration::from_secs(120);

/// A notification titled with the tool's name, in the UI language.
async fn notify(app: &tauri::AppHandle, body: String) {
    let title = crate::window_close::text(app, "tools.campusPrinter.campusPrinter");
    crate::commands::notifications::show_text(app, title, body).await;
}

/// Core's message in the UI language, its placeholders filled.
fn message_text(app: &tauri::AppHandle, message: &CampusPrintMessage) -> String {
    let copy = crate::window_close::text(app, message.key);
    message.args.iter().fold(copy, |copy, (name, value)| {
        copy.replace(&format!("{{{{{name}}}}}"), &value.to_string())
    })
}

/// Starts after Core is available. Keep the binding result for the install
/// command so a busy port is reported there as well as in the startup log.
pub(crate) fn start(app: &tauri::AppHandle) {
    let result = async_runtime::block_on(TcpListener::bind((Ipv4Addr::LOCALHOST, PORT)));
    let stop = CancellationToken::new();
    let task = match result {
        Ok(listener) => {
            let wording_app = app.clone();
            let state = HttpState {
                app: app.clone(),
                queue: Arc::new(CampusPrintQueue::new(
                    core_ports::core(app),
                    format!("ipp://{}:{PORT}{IPP_PATH}", Ipv4Addr::LOCALHOST),
                    PRINTER_UUID.into(),
                    Box::new(move |message| message_text(&wording_app, message)),
                )),
                requests: Arc::new(Semaphore::new(1)),
                small_requests: Arc::new(Semaphore::new(8)),
            };
            let router = Router::new()
                .route(IPP_PATH, post(receive))
                .route(&format!("{IPP_PATH}/jobs/{{id}}"), post(receive))
                .with_state(state);
            let stopped = stop.clone();
            Ok(async_runtime::spawn(async move {
                if let Err(error) = axum::serve(listener, router)
                    .with_graceful_shutdown(stopped.cancelled_owned())
                    .await
                {
                    log::error!("[campus-printer] IPP receiver on port {PORT} stopped: {error}");
                }
            }))
        }
        Err(error) => {
            let message =
                format!("Could not start the campus print receiver on port {PORT}: {error}");
            log::error!("[campus-printer] {message}");
            Err(message)
        }
    };
    app.manage(PrintServer { task, stop });
}

pub(crate) fn stop(app: &tauri::AppHandle) {
    if let Some(server) = app.try_state::<PrintServer>() {
        server.stop.cancel();
    }
}

/// Adds or replaces the system printer behind UAC, for the verified student.
#[tauri::command]
pub(super) async fn install(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    state: tauri::State<'_, Setup>,
) -> Result<CampusPrinterSetup, CommandError> {
    crate::require_local_main(&webview)?;
    let _busy = state
        .0
        .try_lock()
        .map_err(|_| "The printer is already being added.")?;
    match &app.state::<PrintServer>().task {
        Ok(task) if !task.inner().is_finished() => {}
        Ok(_) => {
            return Err(format!("The campus print receiver on port {PORT} has stopped.").into())
        }
        Err(message) => return Err(message.clone().into()),
    }
    let account = core_ports::core(&app).campus_printer_account().await;
    if !matches!(account, PrinterAccount::Ready { .. }) {
        return Ok(CampusPrinterSetup::Account { account });
    }
    let added = async_runtime::spawn_blocking(install_queue)
        .await
        .map_err(crate::shell)??;
    let outcome = if added {
        CampusPrinterSetup::Added
    } else {
        CampusPrinterSetup::Cancelled
    };
    log::info!("[campus-printer] system printer {outcome:?}");
    Ok(outcome)
}

fn powershell() -> Result<PathBuf, String> {
    Ok(
        PathBuf::from(std::env::var_os("SystemRoot").ok_or("SystemRoot is not set")?)
            .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
    )
}
fn identity() -> Result<(PathBuf, String), String> {
    // Capture the interactive caller's SID before UAC; another administrator may
    // supply the elevation credentials, but must not become the queue's owner.
    let shell = powershell()?;
    let sid = Command::new(&shell)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value",
        ])
        .creation_flags(0x08000000)
        .output()
        .map_err(|e| e.to_string())?;
    if !sid.status.success() {
        return Err("Could not resolve the printing user's SID".into());
    }
    let sid = String::from_utf8(sid.stdout).map_err(|e| e.to_string())?;
    let sid = sid.trim();
    if !sid.starts_with("S-1-") || !sid[4..].chars().all(|c| c.is_ascii_digit() || c == '-') {
        return Err("Invalid printing user SID".into());
    }
    Ok((shell, sid.to_string()))
}

fn install_queue() -> Result<bool, String> {
    let (shell, sid) = identity()?;
    let diagnostic =
        std::env::temp_dir().join(format!("isct-campus-printer-{}.log", uuid::Uuid::new_v4()));
    let diagnostic_path = diagnostic.to_string_lossy().replace('\'', "''");
    let script = format!(
        r#"$ErrorActionPreference='Stop'
try {{
 $name='{PRINTER_NAME}'
 if (Get-Printer -Name $name -ErrorAction SilentlyContinue) {{ Remove-Printer -Name $name }}
 # AppContainers need both the user and package access checks to pass.
 $permissions='G:SYD:(A;OIIO;GA;;;CO)(A;OIIO;GA;;;AC)(A;;SWRC;;;{sid})(A;;SWRC;;;AC)(A;CIIO;GX;;;AC)(A;;LCSWDTSDRCWDWO;;;BA)(A;OICIIO;GA;;;BA)(A;;LCSWDTSDRCWDWO;;;SY)(A;OICIIO;GA;;;SY)'
 Add-Printer -Name $name -IppURL 'http://{localhost}:{PORT}{IPP_PATH}' -PermissionSDDL $permissions
 Set-Printer -Name $name -Shared $false -KeepPrintedJobs $false
 Set-PrintConfiguration -PrinterName $name -Color $false -DuplexingMode TwoSidedLongEdge
 exit 0
}} catch {{
 [System.IO.File]::WriteAllText('{diagnostic_path}', $_.Exception.Message)
 exit 1
}}"#,
        localhost = Ipv4Addr::LOCALHOST,
    );
    let encoded = base64::engine::general_purpose::STANDARD.encode(
        script
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    let file = HSTRING::from(shell.as_os_str());
    let parameters = HSTRING::from(format!(
        "-NoProfile -NonInteractive -EncodedCommand {encoded}"
    ));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    if let Err(error) = unsafe { ShellExecuteExW(&mut info) } {
        if error.code() == ERROR_CANCELLED.to_hresult() {
            return Ok(false);
        }
        return Err(error.to_string());
    }
    let handle = info.hProcess;
    if handle.is_invalid() {
        return Err("The printer installer did not start".into());
    }
    // Keep ownership until the elevated process has reached a terminal state.
    let wait = unsafe { WaitForSingleObject(handle, u32::MAX) };
    if wait != WAIT_OBJECT_0 {
        let error = ::windows::core::Error::from_thread();
        let _ = unsafe { CloseHandle(handle) };
        return Err(error.to_string());
    }
    let mut code = 1;
    let result = unsafe { GetExitCodeProcess(handle, &mut code) };
    let _ = unsafe { CloseHandle(handle) };
    let details = std::fs::read_to_string(&diagnostic).ok();
    let _ = std::fs::remove_file(&diagnostic);
    result.map_err(|error| error.to_string())?;
    if code != 0 {
        return Err(
            details.unwrap_or_else(|| format!("The printer installer exited with code {code}."))
        );
    }
    Ok(true)
}
