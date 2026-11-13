//! The Windows system printer: a queue on the inbox IPP class driver, so any
//! app's Print dialog reaches the campus printer. Windows renders the job to
//! PDF and posts it over IPP to a receiver on 127.0.0.1 that this plugin
//! runs while the app does; Core (`app_core::campus_printer::ipp`) answers
//! IPP, bills and sends, and answers each job once its send has ended. This
//! module installs the queue behind UAC, keeps the receiver on the URL the
//! queue stores, and tells the person about a job that did not print.
use super::{CampusPrinterSetup, Setup};
use crate::{core_ports, CommandError};
use ::windows::{
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
use app_core::campus_printer::{
    ipp::{CampusPrintQueue, Failure},
    CampusPrintMessage, PrinterAccount, CAPABILITIES,
};
use axum::{
    body::to_bytes,
    extract::{Request, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    os::windows::process::CommandExt,
    path::PathBuf,
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tauri::Manager;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Endpoint {
    port: u16,
    token: uuid::Uuid,
}
impl Endpoint {
    fn path(&self) -> String {
        format!("/ipp/{}", self.token)
    }
    /// The URL the Windows queue stores.
    fn http_uri(&self) -> String {
        format!("http://127.0.0.1:{}{}", self.port, self.path())
    }
    /// The same endpoint as the receiver reports it (`printer-uri-supported`,
    /// `job-uri`): Windows' IPP client expects the `ipp` scheme there.
    fn ipp_uri(&self) -> String {
        format!("ipp://127.0.0.1:{}{}", self.port, self.path())
    }
    /// A private token on a port outside the range Windows hands out and
    /// reserves for Hyper-V, WSL and Docker at boot. Anything else is
    /// replaced by the next repair.
    fn stable(&self) -> bool {
        STABLE_PORTS.contains(&self.port)
    }
    fn new_private() -> Self {
        Endpoint {
            port: 0,
            token: uuid::Uuid::new_v4(),
        }
    }
    fn marker(&self) -> String {
        format!(
            "IsctCompanion IPP v2 {}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(Sha256::digest(self.http_uri().as_bytes()))
        )
    }
}
struct Running {
    endpoint: Endpoint,
    stop: CancellationToken,
    task: tokio::task::JoinHandle<()>,
    enabled: Arc<AtomicBool>,
}
#[derive(Default)]
pub(crate) struct PrintServer(Mutex<Option<Running>>);

/// Below Windows' dynamic range (49152+), where boot-time port exclusions
/// land, and clear of the well-known services.
const STABLE_PORTS: std::ops::Range<u16> = 21_000..29_000;
const PORT_ATTEMPTS: u16 = 32;

/// Where a new endpoint tries to listen: a spread start from the token, so
/// two Windows users on one computer rarely collide, then the next ports.
fn candidate_ports(token: uuid::Uuid) -> impl Iterator<Item = u16> {
    let span = STABLE_PORTS.end - STABLE_PORTS.start;
    let first = (token.as_u128() % u128::from(span)) as u16;
    (0..PORT_ATTEMPTS).map(move |step| STABLE_PORTS.start + (first + step) % span)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Start {
    /// App start and status checks: only the saved endpoint, never a new
    /// one, because the Windows queue stores its URL.
    Resume,
    /// The administrator-approved repair: may choose a new endpoint, which
    /// the installer then writes into the queue.
    Repair,
}
#[derive(Clone)]
struct HttpState {
    app: tauri::AppHandle,
    queue: Arc<CampusPrintQueue>,
    requests: Arc<tokio::sync::Semaphore>,
    small_requests: Arc<tokio::sync::Semaphore>,
    enabled: Arc<AtomicBool>,
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
    // Discovery must remain available while Windows installs the queue.
    // Never accept a document until its access control has been installed.
    if body.len() >= 4
        && matches!(u16::from_be_bytes([body[2], body[3]]), 0x0002 | 0x0006)
        && !state.enabled.load(Ordering::Acquire)
    {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
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

/// A request for a path the receiver does not serve: logged, so a client
/// posting jobs somewhere unexpected shows in the application log. The
/// route's token is never logged.
async fn unmatched(request: Request) -> Response {
    let path = request
        .uri()
        .path()
        .split('/')
        .map(|part| {
            if uuid::Uuid::parse_str(part).is_ok() {
                "[token]"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join("/");
    log::warn!(
        "[campus-printer] no route for {} {path} (content type {:?})",
        request.method(),
        request.headers().get(header::CONTENT_TYPE)
    );
    StatusCode::NOT_FOUND.into_response()
}

async fn receiver_ready() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"status":"ready"}"#,
    )
        .into_response()
}

fn endpoint_file(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("campus-printer-endpoint.json"))
}

/// Replaces the saved endpoint, or removes it for `None`. Written to a
/// temporary file and renamed, so a failed write never leaves half a file.
fn write_endpoint(app: &tauri::AppHandle, endpoint: Option<&Endpoint>) -> Result<(), String> {
    let config = endpoint_file(app)?;
    let Some(endpoint) = endpoint else {
        return match std::fs::remove_file(&config) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        };
    };
    if let Some(parent) = config.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let pending = config.with_extension("json.pending");
    std::fs::write(
        &pending,
        serde_json::to_vec(endpoint).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::rename(&pending, &config).map_err(|e| e.to_string())
}

/// The endpoint the Windows queue was last installed with, if any.
fn saved_endpoint(app: &tauri::AppHandle) -> Result<Option<Endpoint>, String> {
    match std::fs::read(endpoint_file(app)?) {
        Ok(bytes) => serde_json::from_slice::<Endpoint>(&bytes)
            .map(Some)
            .map_err(|_| "Invalid printer configuration; use Repair system printer".to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

async fn bind_new(endpoint: &Endpoint) -> Result<tokio::net::TcpListener, String> {
    let mut last = String::from("no port tried");
    for port in candidate_ports(endpoint.token) {
        match bind_receiver(port).await {
            Ok(listener) => return Ok(listener),
            Err(error) => last = format!("port {port}: {error}"),
        }
    }
    Err(format!("No free port for the print receiver ({last})"))
}

impl PrintServer {
    async fn start(&self, app: &tauri::AppHandle, mode: Start) -> Result<(), String> {
        let mut running = self.0.lock().await;
        if running.as_ref().is_some_and(|r| {
            !r.task.is_finished() && (mode == Start::Resume || r.endpoint.stable())
        }) {
            return Ok(());
        }
        // A stopped listener, or one a repair replaces: stop it and its
        // listener before anything new starts.
        if let Some(old) = running.take() {
            old.stop.cancel();
            let _ = old.task.await;
        }
        let saved = match (mode, saved_endpoint(app)) {
            (_, Ok(saved)) => saved,
            (Start::Resume, Err(error)) => return Err(error),
            // Repair rewrites an unreadable configuration.
            (Start::Repair, Err(_)) => None,
        };
        let (listener, mut endpoint) = match (mode, saved) {
            (Start::Resume, None) => return Ok(()),
            // The Windows queue stores this URL: resume keeps it or fails.
            (Start::Resume, Some(saved)) => (
                bind_receiver(saved.port)
                    .await
                    .map_err(|e| format!("port {}: {e}", saved.port))?,
                saved,
            ),
            (Start::Repair, Some(saved)) if saved.stable() => match bind_receiver(saved.port).await
            {
                Ok(listener) => (listener, saved),
                Err(error) => {
                    log::warn!(
                        "[campus-printer] saved receiver port {} is unavailable; repair moves it: {error}",
                        saved.port
                    );
                    let endpoint = Endpoint::new_private();
                    (bind_new(&endpoint).await?, endpoint)
                }
            },
            // No usable endpoint yet.
            (Start::Repair, _) => {
                let endpoint = Endpoint::new_private();
                (bind_new(&endpoint).await?, endpoint)
            }
        };
        endpoint.port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let enabled = Arc::new(AtomicBool::new(
            !endpoint_file(app)?.with_extension("installing").exists(),
        ));
        let wording_app = app.clone();
        let state = HttpState {
            app: app.clone(),
            queue: Arc::new(CampusPrintQueue::new(
                core_ports::core(app),
                endpoint.ipp_uri(),
                format!("urn:uuid:{}", endpoint.token),
                Box::new(move |message| message_text(&wording_app, message)),
            )),
            requests: Arc::new(tokio::sync::Semaphore::new(1)),
            small_requests: Arc::new(tokio::sync::Semaphore::new(8)),
            enabled: enabled.clone(),
        };
        let router = Router::new()
            .route("/", get(receiver_ready))
            .route(&endpoint.path(), get(receiver_ready).post(receive))
            .route(
                &format!("{}/jobs/{{id}}", endpoint.path()),
                get(receiver_ready).post(receive),
            )
            .fallback(unmatched)
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
            enabled,
        });
        Ok(())
    }
    /// After a repair that did not reach the Windows queue: go back to the
    /// endpoint the queue still points at.
    async fn restore(&self, app: &tauri::AppHandle) {
        let saved = saved_endpoint(app).ok().flatten();
        let current = self
            .0
            .lock()
            .await
            .as_ref()
            .filter(|r| !r.task.is_finished())
            .map(|r| r.endpoint.clone());
        if current.is_some() && current == saved {
            return;
        }
        self.shutdown().await;
        if let Err(error) = self.start(app, Start::Resume).await {
            log::warn!("[campus-printer] could not return to the installed receiver: {error}");
        }
    }
    /// Saves the running endpoint as the one the Windows queue uses.
    async fn persist_endpoint(&self, app: &tauri::AppHandle) -> Result<(), String> {
        let endpoint = self
            .0
            .lock()
            .await
            .as_ref()
            .filter(|r| !r.task.is_finished())
            .map(|r| r.endpoint.clone())
            .ok_or("Printer receiver did not start")?;
        write_endpoint(app, Some(&endpoint))
    }
    /// After a repair that left the Windows queue on its old URL: put the
    /// saved endpoint back as it was and serve it again.
    async fn revert(&self, app: &tauri::AppHandle, previous: Option<Endpoint>) {
        if let Err(error) = write_endpoint(app, previous.as_ref()) {
            log::warn!("[campus-printer] could not restore the saved receiver address: {error}");
        }
        self.restore(app).await;
    }
    pub(crate) async fn shutdown(&self) {
        if let Some(running) = self.0.lock().await.take() {
            running.stop.cancel();
            let _ = running.task.await;
        }
    }
}

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

/// The plugin's state; the receiver starts once Core runs (`resume`).
pub(super) fn setup(app: &tauri::AppHandle) {
    app.manage(PrintServer::default());
}

/// Serves the URL the installed queue stores, if any, for as long as the app
/// runs.
pub(super) fn resume(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = app.state::<PrintServer>().start(&app, Start::Resume).await {
            log::warn!("[campus-printer] could not resume Windows queue: {error}");
            // Printing to the queue would only pile up errors until repaired.
            let message = crate::window_close::text(&app, "campusPrint.notice.receiverFailed");
            notify(&app, message).await;
        }
    });
}

/// Adds or repairs the system printer behind UAC, for the verified student.
/// `prompt` words macOS's administrator prompt; UAC words its own.
#[tauri::command]
pub(super) async fn install(
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
    // What the Windows queue points at now, to go back to if this repair
    // does not reach it.
    let previous = saved_endpoint(&app).ok().flatten();
    server.start(&app, Start::Repair).await?;
    let (uri, marker, enabled, was_enabled) = {
        let running = server.0.lock().await;
        let current = running.as_ref().ok_or("Printer receiver did not start")?;
        let was_enabled = current.enabled.swap(false, Ordering::AcqRel);
        (
            current.endpoint.http_uri(),
            current.endpoint.marker(),
            current.enabled.clone(),
            was_enabled,
        )
    };
    // Save the new address before Windows can be pointed at it: a queue on a
    // URL the app would not serve after a restart is what this order rules
    // out. If saving fails, Windows is not touched.
    if let Err(error) = server.persist_endpoint(&app).await {
        log::warn!("[campus-printer] the receiver address was not saved: {error}");
        enabled.store(was_enabled, Ordering::Release);
        server.revert(&app, previous).await;
        return Ok(CampusPrinterSetup::Failed);
    }
    let installed_marker = marker.clone();
    let journal = endpoint_file(&app)?.with_extension("installing");
    let result =
        tauri::async_runtime::spawn_blocking(move || install_queue(&uri, &marker, &journal))
            .await
            .map_err(|e| e.to_string())?;
    let outcome = match result {
        Ok(true) => {
            enabled.store(true, Ordering::Release);
            CampusPrinterSetup::Added
        }
        Ok(false) => {
            enabled.store(was_enabled, Ordering::Release);
            server.revert(&app, previous).await;
            CampusPrinterSetup::Cancelled
        }
        Err(failure) => {
            log::warn!(
                "[campus-printer] the system printer was not added: {}",
                failure.message
            );
            // The queue uses the new address once the installer has added
            // it, which is before it carries the new marker: a failure in a
            // later step leaves it there unmarked. Otherwise decide by what
            // Windows has now: a queue carrying the new marker uses the new
            // address, anything else still uses the old one.
            let moved = failure.repointed
                || tauri::async_runtime::spawn_blocking(move || {
                    probe_queue(Some(&installed_marker))
                })
                .await
                .map(|state| matches!(state, Ok(QueueState::Ready)))
                .unwrap_or(false);
            if !moved {
                enabled.store(was_enabled, Ordering::Release);
                server.revert(&app, previous).await;
            }
            CampusPrinterSetup::Failed
        }
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
fn identity() -> Result<(PathBuf, String, String), String> {
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
    let name = format!(
        "IsctCompanion Campus Printer ({:.12})",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(sid.as_bytes()))
    );
    Ok((shell, sid.to_string(), name))
}

/// Why the installer did not finish.
struct InstallFailure {
    message: String,
    /// The installer had already added the queue on the new address when it
    /// failed, so the receiver must stay there.
    repointed: bool,
}

impl InstallFailure {
    /// A failure in `stage` of the installer's script. The stages after
    /// `add-ipp-queue` run with the queue on the new address: just added,
    /// or already there under the same marker.
    fn at(stage: &str, message: String) -> Self {
        Self {
            message,
            repointed: matches!(
                stage,
                "remove-previous-queue"
                    | "add-ipp-queue"
                    | "configure-queue-access"
                    | "configure-print-defaults"
                    | "verify-queue"
            ),
        }
    }
}

/// A failure that left Windows' queue as it was.
impl From<String> for InstallFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            repointed: false,
        }
    }
}

impl From<&str> for InstallFailure {
    fn from(message: &str) -> Self {
        message.to_string().into()
    }
}

fn install_queue(
    uri: &str,
    marker: &str,
    journal: &std::path::Path,
) -> Result<bool, InstallFailure> {
    let (shell, sid, name) = identity()?;
    let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
    let diagnostic =
        std::env::temp_dir().join(format!("isct-campus-printer-{}.log", uuid::Uuid::new_v4()));
    let script = format!(
        r#"$ErrorActionPreference='Stop'
$stage='inspect-queue'
try {{
 $name={name}; $uri={uri}; $marker={marker}; $owner='IsctCompanion:{sid}'
 $existing=Get-Printer -Name $name -Full -ErrorAction SilentlyContinue
 $pending=Test-Path -LiteralPath {journal}
 if ($existing -and ($existing.DriverName -ne 'Microsoft IPP Class Driver' -or ($existing.Location -ne $owner -and -not ($pending -and -not $existing.Location)))) {{ throw 'Printer name belongs to another queue' }}
 $replace=$existing -and ($existing.Comment -ne $marker -or $existing.Location -ne $owner)
 $remove=@()
 if ($replace) {{ $remove+= $existing }}
 # Before anything is removed: a receiver that does not answer leaves the old queue in place.
 $stage='check-receiver'
 Invoke-WebRequest -Uri ('http://127.0.0.1:' + ([uri]$uri).Port + '/') -UseBasicParsing -ErrorAction Stop | Out-Null
 $stage='check-pending-jobs'
 foreach ($printer in $remove) {{ if (@(Get-PrintJob -PrinterName $printer.Name -ErrorAction Stop).Count -gt 0) {{ exit 2 }} }}
 $stage='remove-previous-queue'
 [System.IO.File]::WriteAllText({journal}, $marker)
 foreach ($printer in $remove) {{ Remove-Printer -Name $printer.Name -ErrorAction Stop }}
 $stage='add-ipp-queue'
 # AppContainers need both the user and package access checks to pass.
 # Keep their package ACE, but grant ordinary Print only to this user.
 $permissions='G:SYD:(A;OIIO;GA;;;CO)(A;OIIO;GA;;;AC)(A;;SWRC;;;{sid})(A;;SWRC;;;AC)(A;CIIO;GX;;;AC)(A;;LCSWDTSDRCWDWO;;;BA)(A;OICIIO;GA;;;BA)(A;;LCSWDTSDRCWDWO;;;SY)(A;OICIIO;GA;;;SY)'
 if (-not $existing -or $replace) {{
  Add-Printer -Name $name -IppURL $uri -PermissionSDDL $permissions -Location $owner -Comment $marker -ErrorAction Stop
 }}
 $stage='configure-queue-access'
 Set-Printer -Name $name -Location $owner -Comment $marker -Shared $false -KeepPrintedJobs $false -PermissionSDDL $permissions -ErrorAction Stop
 $stage='configure-print-defaults'
 Set-PrintConfiguration -PrinterName $name -Color $false -DuplexingMode TwoSidedLongEdge -ErrorAction Stop
 $stage='verify-queue'
 $printer=Get-Printer -Name $name -Full -ErrorAction Stop
 if ($printer.DriverName -ne 'Microsoft IPP Class Driver' -or $printer.Comment -ne $marker) {{ throw 'Printer installation could not be verified' }}
 Remove-Item -LiteralPath {journal} -Force -ErrorAction Stop
 exit 0
}} catch {{
 $details=$_.Exception.Message
 foreach ($secret in @($uri,$marker,$owner)) {{ if ($secret) {{ $details=$details.Replace($secret,'[redacted]') }} }}
 try {{ [System.IO.File]::WriteAllText({diagnostic}, "$stage|$details", [System.Text.Encoding]::UTF8) }} catch {{}}
 $failureCode=49
 switch ($stage) {{
  'inspect-queue' {{ $failureCode=41 }}
  'check-pending-jobs' {{ $failureCode=43 }}
  'remove-previous-queue' {{ $failureCode=44 }}
  'check-receiver' {{ $failureCode=45 }}
  'add-ipp-queue' {{ $failureCode=46 }}
  'configure-queue-access' {{ $failureCode=47 }}
  'configure-print-defaults' {{ $failureCode=48 }}
  'verify-queue' {{ $failureCode=49 }}
 }}
 exit $failureCode
}}"#,
        name = quote(&name),
        uri = quote(uri),
        marker = quote(marker),
        diagnostic = quote(&diagnostic.to_string_lossy()),
        journal = quote(&journal.to_string_lossy())
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
        return Err(error.to_string().into());
    }
    let handle = info.hProcess;
    if handle.is_invalid() {
        return Err("The printer installer did not start".into());
    }
    // Keep ownership until the elevated process has reached a terminal state.
    let wait = unsafe { WaitForSingleObject(handle, u32::MAX) };
    let mut code = 1;
    let result = unsafe { GetExitCodeProcess(handle, &mut code) };
    let _ = unsafe { CloseHandle(handle) };
    let details = std::fs::read_to_string(&diagnostic).ok();
    let _ = std::fs::remove_file(&diagnostic);
    if wait.0 == 0 && result.is_ok() && code == 2 {
        return Err("Finish or cancel pending jobs before repairing the printer".into());
    }
    if wait.0 != 0 || result.is_err() || code != 0 {
        if let Some((stage, message)) = details
            .as_deref()
            .and_then(|value| value.trim_start_matches('\u{feff}').split_once('|'))
        {
            return Err(InstallFailure::at(
                stage,
                format!("Printer installation failed at {stage}: {message}"),
            ));
        }
        let stage = match code {
            41 => Some("inspect-queue"),
            43 => Some("check-pending-jobs"),
            44 => Some("remove-previous-queue"),
            45 => Some("check-receiver"),
            46 => Some("add-ipp-queue"),
            47 => Some("configure-queue-access"),
            48 => Some("configure-print-defaults"),
            49 => Some("verify-queue"),
            _ => None,
        };
        if let Some(stage) = stage {
            return Err(InstallFailure::at(
                stage,
                format!(
                    "Printer installation failed at {stage}; Windows supplied no diagnostic (exit code {code})"
                ),
            ));
        }
        return Err(InstallFailure {
            message: "Windows did not confirm printer installation".into(),
            repointed: true,
        });
    }
    Ok(true)
}

async fn bind_receiver(port: u16) -> std::io::Result<tokio::net::TcpListener> {
    tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await
}

/// The system printer as the tool card shows it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum QueueState {
    NotInstalled,
    Ready,
    /// Installed, but not on the URL the running receiver serves (an older
    /// install, or a receiver that could not start): a repair fixes it.
    RepairRequired,
    Unavailable,
}

#[tauri::command]
pub(super) async fn queue_status(
    webview: tauri::Webview,
    app: tauri::AppHandle,
) -> Result<QueueState, CommandError> {
    crate::require_local_main(&webview)?;
    let server = app.state::<PrintServer>();
    if let Err(error) = server.start(&app, Start::Resume).await {
        log::warn!("[campus-printer] the receiver is not running: {error}");
    }
    let marker = server
        .0
        .lock()
        .await
        .as_ref()
        .filter(|r| !r.task.is_finished())
        .map(|r| r.endpoint.marker());
    let pending = endpoint_file(&app)?.with_extension("installing").exists();
    let state = tauri::async_runtime::spawn_blocking(move || probe_queue(marker.as_deref()))
        .await
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|error| {
            log::warn!("[campus-printer] could not read the system printer: {error}");
            QueueState::Unavailable
        });
    let state = if pending {
        QueueState::RepairRequired
    } else {
        state
    };
    log::info!("[campus-printer] system printer state {state:?}");
    Ok(state)
}

fn probe_queue(marker: Option<&str>) -> Result<QueueState, String> {
    let (shell, sid, name) = identity()?;
    let script = format!(
        r#"$ErrorActionPreference='Stop'
$p=Get-Printer -Name '{name}' -Full -ErrorAction SilentlyContinue
if ($p) {{ $p | Select-Object DriverName,Location,Comment | ConvertTo-Json -Compress }}"#
    );
    let output = Command::new(shell)
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x08000000)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "Windows printers could not be read: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    if output.stdout.iter().all(u8::is_ascii_whitespace) {
        return Ok(QueueState::NotInstalled);
    }
    let data: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
    Ok(
        if marker.is_some_and(|m| data["Comment"].as_str() == Some(m))
            && data["DriverName"] == "Microsoft IPP Class Driver"
            && data["Location"] == format!("IsctCompanion:{sid}")
        {
            QueueState::Ready
        } else {
            QueueState::RepairRequired
        },
    )
}

#[cfg(test)]
#[path = "../../tests/campus_printer_queue.rs"]
mod tests;
