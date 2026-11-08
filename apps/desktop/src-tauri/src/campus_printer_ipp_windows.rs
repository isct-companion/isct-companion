//! Native transport and queue installation only. Core owns jobs and billing.
use crate::{
    campus_printer::{CampusPrinterSetup, Setup},
    core_ports, CommandError,
};
use app_core::campus_printer::{
    ipp::{CampusPrintQueue, MAX_DOCUMENT_BYTES},
    PrinterAccount,
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
use serde::{Deserialize, Serialize};
use std::{
    os::windows::process::CommandExt, path::PathBuf, process::Command, sync::Arc, time::Duration,
};
use tauri::Manager;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use windows::{
    core::{w, HSTRING, PCWSTR},
    Win32::{
        Foundation::{CloseHandle, ERROR_CANCELLED},
        System::Threading::{GetExitCodeProcess, WaitForSingleObject},
        UI::{
            Shell::{
                ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
            },
            WindowsAndMessaging::SW_HIDE,
        },
    },
};

#[derive(Serialize, Deserialize)]
struct Endpoint {
    port: u16,
    token: uuid::Uuid,
}
impl Endpoint {
    fn path(&self) -> String {
        format!("/ipp/{}", self.token)
    }
    fn http_uri(&self) -> String {
        format!("http://127.0.0.1:{}{}", self.port, self.path())
    }
    fn ipp_uri(&self) -> String {
        format!("ipp://127.0.0.1:{}{}", self.port, self.path())
    }
    fn name(&self) -> String {
        format!("IsctCompanion Campus Printer - {}", self.token.simple())
    }
}
struct Running {
    endpoint: Endpoint,
    stop: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}
#[derive(Default)]
pub(crate) struct PrintServer(Mutex<Option<Running>>);
#[derive(Clone)]
struct HttpState {
    queue: Arc<CampusPrintQueue>,
    requests: Arc<tokio::sync::Semaphore>,
}

async fn receive(State(state): State<HttpState>, request: Request) -> Response {
    if request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        != Some("application/ipp")
    {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
    }
    // No CORS and an unguessable per-user route; the Windows queue is also ACLed.
    let Ok(_permit) = state.requests.try_acquire() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let body = match tokio::time::timeout(
        Duration::from_secs(30),
        to_bytes(request.into_body(), MAX_DOCUMENT_BYTES + 65536),
    )
    .await
    {
        Ok(Ok(body)) => body,
        _ => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    match state.queue.handle(body.to_vec()).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, "application/ipp")], bytes).into_response(),
        Err(error) => {
            log::warn!("[campus-printer] invalid IPP request: {error}");
            StatusCode::BAD_REQUEST.into_response()
        }
    }
}

impl PrintServer {
    async fn start(&self, app: &tauri::AppHandle, create: bool) -> Result<(), String> {
        let mut running = self.0.lock().await;
        if running.as_ref().is_some_and(|r| !r.task.is_finished()) {
            return Ok(());
        }
        let config = app
            .path()
            .app_data_dir()
            .map_err(|e| e.to_string())?
            .join("campus-printer-endpoint.json");
        let saved = match std::fs::read(&config) {
            Ok(bytes) => {
                Some(serde_json::from_slice::<Endpoint>(&bytes).map_err(|e| e.to_string())?)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.to_string()),
        };
        if saved.is_none() && !create {
            return Ok(());
        }
        if saved.as_ref().is_some_and(|s| s.port == 0) {
            return Err("Invalid saved printer endpoint".into());
        }
        let listener = tokio::net::TcpListener::bind((
            std::net::Ipv4Addr::LOCALHOST,
            saved.as_ref().map_or(0, |s| s.port),
        ))
        .await
        .map_err(|e| e.to_string())?;
        let endpoint = match saved {
            Some(endpoint) => endpoint,
            None => {
                let endpoint = Endpoint {
                    port: listener.local_addr().map_err(|e| e.to_string())?.port(),
                    token: uuid::Uuid::new_v4(),
                };
                if let Some(parent) = config.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::write(
                    &config,
                    serde_json::to_vec(&endpoint).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                endpoint
            }
        };
        let state = HttpState {
            queue: CampusPrintQueue::new(core_ports::core(app), endpoint.ipp_uri()),
            requests: Arc::new(tokio::sync::Semaphore::new(4)),
        };
        let router = Router::new()
            .route(&endpoint.path(), post(receive))
            .route(&format!("{}/jobs/{{id}}", endpoint.path()), post(receive))
            .with_state(state);
        let stop = CancellationToken::new();
        let stopped = stop.clone();
        let task = tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, router)
                .with_graceful_shutdown(stopped.cancelled_owned())
                .await
            {
                log::error!("[campus-printer] IPP listener stopped: {error}");
            }
        });
        *running = Some(Running {
            endpoint,
            stop,
            task,
        });
        Ok(())
    }
    pub(crate) async fn shutdown(&self) {
        if let Some(running) = self.0.lock().await.take() {
            running.stop.cancel();
            let _ = running.task.await;
        }
    }
}

pub(crate) fn resume(app: &tauri::AppHandle) {
    app.manage(PrintServer::default());
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = app.state::<PrintServer>().start(&app, false).await {
            log::warn!("[campus-printer] could not resume Windows queue: {error}");
        }
    });
}

#[tauri::command]
pub async fn install_campus_printer(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    state: tauri::State<'_, Setup>,
    prompt: String,
) -> Result<CampusPrinterSetup, CommandError> {
    crate::require_local_main(&webview)?;
    let _busy = state
        .0
        .try_lock()
        .map_err(|_| "The printer is already being added.".to_string())?;
    let _ = prompt;
    let account = core_ports::core(&app).campus_printer_account().await;
    if !matches!(account, PrinterAccount::Ready { .. }) {
        return Ok(CampusPrinterSetup::Account { account });
    }
    let server = app.state::<PrintServer>();
    server.start(&app, true).await?;
    let (name, uri) = {
        let running = server.0.lock().await;
        let endpoint = &running
            .as_ref()
            .ok_or("Printer receiver did not start")?
            .endpoint;
        (endpoint.name(), endpoint.http_uri())
    };
    let result = tauri::async_runtime::spawn_blocking(move || install_queue(&name, &uri))
        .await
        .map_err(|e| e.to_string())?;
    Ok(match result {
        Ok(true) => CampusPrinterSetup::Added,
        Ok(false) => CampusPrinterSetup::Cancelled,
        Err(error) => {
            log::warn!("[campus-printer] Windows queue installation failed: {error}");
            CampusPrinterSetup::Failed
        }
    })
}

fn powershell() -> Result<PathBuf, String> {
    Ok(
        PathBuf::from(std::env::var_os("SystemRoot").ok_or("SystemRoot is not set")?)
            .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
    )
}
fn install_queue(name: &str, uri: &str) -> Result<bool, String> {
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
    let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
    let script = format!(
        r#"$ErrorActionPreference='Stop'
try {{
 $name={name}; $uri={uri}
 $existing=Get-Printer -Name $name -ErrorAction SilentlyContinue
 if (-not $existing) {{ Add-Printer -Name $name -IppURL $uri -ErrorAction Stop }}
 $printer=Get-Printer -Name $name -ErrorAction Stop
 if ($printer.DriverName -ne 'Microsoft IPP Class Driver') {{ throw 'Unexpected printer driver' }}
 Set-Printer -Name $name -Shared $false -KeepPrintedJobs $false -PermissionSDDL 'D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{sid})' -ErrorAction Stop
 exit 0
}} catch {{ exit 1 }}"#,
        name = quote(name),
        uri = quote(uri)
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
    // The installer performs only idempotent queue configuration. A late finish
    // after timeout is harmless; no document is passed to the elevated process.
    let wait = unsafe { WaitForSingleObject(handle, 120_000) };
    let mut code = 1;
    let result = unsafe { GetExitCodeProcess(handle, &mut code) };
    let _ = unsafe { CloseHandle(handle) };
    if wait.0 != 0 || result.is_err() || code != 0 {
        return Err("Windows did not confirm printer installation".into());
    }
    Ok(true)
}
