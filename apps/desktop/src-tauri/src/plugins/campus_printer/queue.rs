//! The Windows system printer: a queue on the inbox IPP class driver, so any
//! app's Print dialog reaches the campus printer. Windows renders the job to
//! PDF and posts it over IPP to a receiver on 127.0.0.1 that this plugin
//! runs while the app does; Core (`app_core::campus_printer::ipp`) answers
//! IPP, bills and delivers. This module installs the queue behind UAC, keeps
//! the receiver on the URL the queue stores, and keeps the Windows print list
//! clear: the campus printer holds jobs until release and never reports
//! back, so a received job is finished as far as Windows is concerned, and
//! failures reach the person as notifications.
use super::{CampusPrinterSetup, Setup};
use crate::{core_ports, CommandError};
use ::windows::{
    core::{w, HSTRING, PCWSTR},
    Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_CANCELLED},
        Graphics::{
            Gdi::DEVMODEW,
            Printing::{
                ClosePrinter, EnumJobsW, OpenPrinterW, SetJobW, JOB_CONTROL_DELETE,
                JOB_CONTROL_RESTART, JOB_INFO_2W, JOB_STATUS_BLOCKED_DEVQ, JOB_STATUS_COMPLETE,
                JOB_STATUS_DELETED, JOB_STATUS_DELETING, JOB_STATUS_ERROR, JOB_STATUS_OFFLINE,
                JOB_STATUS_PAPEROUT, JOB_STATUS_PAUSED, JOB_STATUS_PRINTED, JOB_STATUS_PRINTING,
                JOB_STATUS_RENDERING_LOCALLY, JOB_STATUS_RESTART, JOB_STATUS_RETAINED,
                JOB_STATUS_SPOOLING, JOB_STATUS_USER_INTERVENTION, PRINTER_ACCESS_ADMINISTER,
                PRINTER_DEFAULTSW, PRINTER_HANDLE,
            },
        },
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
    ipp::{CampusPrintQueue, MAX_DOCUMENT_BYTES},
    CampusPrintOutcome, PrinterAccount,
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
    collections::{HashMap, HashSet},
    os::windows::process::CommandExt,
    path::PathBuf,
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};
use tauri::Manager;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Endpoint {
    port: u16,
    token: uuid::Uuid,
    #[serde(default)]
    private_token: bool,
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
        self.private_token && STABLE_PORTS.contains(&self.port)
    }
    fn new_private() -> Self {
        Endpoint {
            port: 0,
            token: uuid::Uuid::new_v4(),
            private_token: true,
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
    cleanup_task: tokio::task::JoinHandle<()>,
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
    queue: Arc<CampusPrintQueue>,
    requests: Arc<tokio::sync::Semaphore>,
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
    // No CORS and an unguessable per-user route; the Windows queue is also ACLed.
    // Requests wait for a slot instead of failing: a refused status poll
    // makes Windows show the queue as offline.
    let Ok(_permit) = state.requests.acquire().await else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let body = match tokio::time::timeout(
        BODY_TIMEOUT,
        to_bytes(request.into_body(), MAX_DOCUMENT_BYTES + 65536),
    )
    .await
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
    match state.queue.handle(body).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, "application/ipp")], bytes).into_response(),
        Err(error) => {
            log::warn!("[campus-printer] invalid IPP request: {error}");
            StatusCode::BAD_REQUEST.into_response()
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
        // queue cleanup before anything new starts, so only one loop runs.
        if let Some(old) = running.take() {
            old.stop.cancel();
            let _ = old.task.await;
            let _ = old.cleanup_task.await;
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
            // No endpoint yet, a legacy public token or an ephemeral port.
            (Start::Repair, _) => {
                let endpoint = Endpoint::new_private();
                (bind_new(&endpoint).await?, endpoint)
            }
        };
        endpoint.port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let app_for_report = app.clone();
        let state = HttpState {
            queue: CampusPrintQueue::new_with_delivery_reporter(
                core_ports::core(app),
                endpoint.ipp_uri(),
                format!("urn:uuid:{}", endpoint.token),
                Arc::new(move |title: String, outcome: CampusPrintOutcome| {
                    let app = app_for_report.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Some(body) = delivery_failure_text(&app, &title, &outcome) {
                            notify(&app, body).await;
                        }
                    });
                }),
            ),
            requests: Arc::new(tokio::sync::Semaphore::new(4)),
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
        let cleanup_stop = stop.clone();
        let app = app.clone();
        let cleanup_task = tokio::spawn(async move {
            cleanup_queue(app, cleanup_stop).await;
        });
        *running = Some(Running {
            endpoint,
            stop,
            task,
            cleanup_task,
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
            let _ = running.cleanup_task.await;
        }
    }
}

const QUEUE_POLL_INTERVAL: Duration = Duration::from_secs(2);
/// How long a job restarted after an error gets to reach the receiver.
const RETRY_GRACE: Duration = Duration::from_secs(30);
/// How long a job may sit in a state none of the rules name.
const UNKNOWN_JOB_GRACE: Duration = Duration::from_secs(60);

#[derive(Clone)]
struct SpoolJob {
    id: u32,
    status: u32,
    /// What Windows says about the job (its status text, data type, driver
    /// and size), for the log when it fails.
    detail: String,
}

/// What the cleanup loop does with one Windows job. The receiver answers
/// every document as completed at once (Core delivers it and reports
/// failures itself), so the Windows list only ever holds jobs on their way
/// to the receiver, finished ones, or ones Windows could not hand over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpoolAction {
    Keep,
    /// Finished: drop it from the list, nothing to tell.
    Remove,
    /// Windows could not reach the receiver, usually because the app was
    /// closed when the job was printed. The receiver is up now: send again.
    Restart,
    /// Still failing after a restart, or stuck: tell the person, then drop it.
    Fail(u32),
}

fn spool_job_action(status: u32, waited: Duration, restarted: bool) -> SpoolAction {
    if status & JOB_STATUS_DELETING != 0 {
        return SpoolAction::Keep;
    }
    if status & (JOB_STATUS_PRINTED | JOB_STATUS_COMPLETE | JOB_STATUS_DELETED) != 0 {
        return SpoolAction::Remove;
    }
    let failed = status
        & (JOB_STATUS_ERROR
            | JOB_STATUS_OFFLINE
            | JOB_STATUS_PAPEROUT
            | JOB_STATUS_BLOCKED_DEVQ
            | JOB_STATUS_USER_INTERVENTION);
    if failed != 0 {
        return if !restarted {
            SpoolAction::Restart
        } else if waited >= RETRY_GRACE {
            SpoolAction::Fail(failed)
        } else {
            SpoolAction::Keep
        };
    }
    // Paused by the person: theirs to resume or delete.
    if status & JOB_STATUS_PAUSED != 0 {
        return SpoolAction::Keep;
    }
    let active = status
        & (JOB_STATUS_SPOOLING
            | JOB_STATUS_PRINTING
            | JOB_STATUS_RESTART
            | JOB_STATUS_RENDERING_LOCALLY)
        != 0;
    // Zero means normally queued; retained alone does not mean delivery finished.
    if active || status == 0 || status == JOB_STATUS_RETAINED || waited < UNKNOWN_JOB_GRACE {
        SpoolAction::Keep
    } else {
        SpoolAction::Fail(status)
    }
}

struct Tracked {
    since: Instant,
    restarted: bool,
}

/// Runs while the receiver runs: keeps the Windows list empty of finished
/// jobs and turns jobs Windows could not deliver into notifications.
async fn cleanup_queue(app: tauri::AppHandle, stop: CancellationToken) {
    let mut printer_name = None::<String>;
    let mut tracked = HashMap::<u32, Tracked>::new();
    let mut reported = HashSet::<u32>::new();
    let mut interval = tokio::time::interval(QUEUE_POLL_INTERVAL);
    // The same failure every two seconds (the queue not installed yet) is
    // logged once, until it changes.
    let mut last_warning = None::<String>;
    let mut warn_once = move |message: String| {
        if last_warning.as_ref() != Some(&message) {
            log::warn!("[campus-printer] {message}");
            last_warning = Some(message);
        }
    };
    loop {
        tokio::select! {
            _ = stop.cancelled() => break,
            _ = interval.tick() => {
                if printer_name.is_none() {
                    match tauri::async_runtime::spawn_blocking(identity).await {
                        Ok(Ok((_, _, name))) => printer_name = Some(name),
                        Ok(Err(error)) => {
                            warn_once(format!("could not identify the Windows queue for cleanup: {error}"));
                            continue;
                        }
                        Err(error) => {
                            log::warn!("[campus-printer] queue identification task failed: {error}");
                            continue;
                        }
                    }
                }
                let Some(name) = printer_name.clone() else { continue; };
                let jobs = match tauri::async_runtime::spawn_blocking(move || enum_spool_jobs(&name)).await {
                    Ok(Ok(jobs)) => jobs,
                    Ok(Err(error)) => {
                        warn_once(format!("could not inspect print queue: {error}"));
                        continue;
                    }
                    Err(error) => {
                        log::warn!("[campus-printer] print queue inspection task failed: {error}");
                        continue;
                    }
                };
                let present: HashSet<u32> = jobs.iter().map(|job| job.id).collect();
                tracked.retain(|id, _| present.contains(id));
                reported.retain(|id| present.contains(id));
                for job in jobs {
                    let entry = tracked.entry(job.id).or_insert_with(|| Tracked {
                        since: Instant::now(),
                        restarted: false,
                    });
                    let action = spool_job_action(job.status, entry.since.elapsed(), entry.restarted);
                    let command = match action {
                        SpoolAction::Keep => continue,
                        SpoolAction::Restart => {
                            log::info!("[campus-printer] Windows job {} failed (0x{:08X}): {}", job.id, job.status, job.detail);
                            entry.restarted = true;
                            entry.since = Instant::now();
                            JOB_CONTROL_RESTART
                        }
                        SpoolAction::Remove => JOB_CONTROL_DELETE,
                        SpoolAction::Fail(code) => {
                            if reported.insert(job.id) {
                                let message = crate::window_close::text(&app, "campusPrint.notice.queueFailed")
                                    .replace("{{code}}", &format!("0x{code:08X}"));
                                log::warn!("[campus-printer] Windows job {} failed with status 0x{code:08X}: {}", job.id, job.detail);
                                notify(&app, message).await;
                            }
                            JOB_CONTROL_DELETE
                        }
                    };
                    let Some(name) = printer_name.clone() else { continue; };
                    let id = job.id;
                    match tauri::async_runtime::spawn_blocking(move || control_spool_job(&name, id, command)).await {
                        Ok(Ok(())) => log::info!("[campus-printer] {action:?} Windows print job {id}"),
                        // A finished job often leaves the list on its own between
                        // the poll and this call; the next poll tries again.
                        Ok(Err(error)) => log::warn!("[campus-printer] could not {action:?} Windows job {id}: {error}"),
                        Err(error) => log::warn!("[campus-printer] print job control task failed: {error}"),
                    }
                }
            }
        }
    }
}

/// A notification titled with the tool's name, in the UI language.
async fn notify(app: &tauri::AppHandle, body: String) {
    let title = crate::window_close::text(app, "tools.campusPrinter.campusPrinter");
    crate::commands::notifications::show_text(app, title, body).await;
}

/// The notification for a document Core could not deliver; `None` when it
/// was delivered. Uses the print window's wording for the same outcomes.
fn delivery_failure_text(
    app: &tauri::AppHandle,
    title: &str,
    outcome: &CampusPrintOutcome,
) -> Option<String> {
    let text = |key: &str| crate::window_close::text(app, key);
    let after = |key: &str, printed: u32| text(key).replace("{{printed}}", &printed.to_string());
    let detail = match outcome {
        CampusPrintOutcome::Printed { .. } => return None,
        CampusPrintOutcome::Account { account } => text(match account {
            PrinterAccount::NotConnected => "tools.campusPrinter.saveTheTokyoTechAccount",
            PrinterAccount::NotStudentId => "tools.campusPrinter.accountIsNotAStudentId",
            PrinterAccount::NeedsAttention => "tools.campusPrinter.accountNeedsAttention",
            _ => "tools.campusPrinter.accountNotVerified",
        }),
        CampusPrintOutcome::NotPrintable => text("campusPrint.window.notAPdf"),
        CampusPrintOutcome::InvalidCopies => text("campusPrint.window.copiesRange"),
        CampusPrintOutcome::Unreadable => text("campusPrint.window.couldNotReadTheFile"),
        CampusPrintOutcome::Unreachable { printed: 0 } => {
            text("tools.campusPrinter.cannotReachThePrinter")
        }
        CampusPrintOutcome::Unreachable { printed } => {
            after("campusPrint.window.unreachableAfter", *printed)
        }
        CampusPrintOutcome::NotPrinted { printed: 0 } => text("campusPrint.window.notPrinted"),
        CampusPrintOutcome::NotPrinted { printed } => {
            after("campusPrint.window.notPrintedAfter", *printed)
        }
        CampusPrintOutcome::Uncertain { printed: 0 } => text("campusPrint.window.uncertain"),
        CampusPrintOutcome::Uncertain { printed } => {
            after("campusPrint.window.uncertainAfter", *printed)
        }
    };
    Some(format!("{title}: {detail}"))
}

fn enum_spool_jobs(printer_name: &str) -> Result<Vec<SpoolJob>, String> {
    let name = HSTRING::from(printer_name);
    let defaults = PRINTER_DEFAULTSW {
        pDatatype: ::windows::core::PWSTR::null(),
        pDevMode: std::ptr::null_mut::<DEVMODEW>(),
        DesiredAccess: PRINTER_ACCESS_ADMINISTER,
    };
    let mut printer = PRINTER_HANDLE::default();
    unsafe { OpenPrinterW(&name, &mut printer, Some(&defaults)) }
        .map_err(|error| format!("OpenPrinterW: {error}"))?;
    let result = (|| {
        let mut needed = 0u32;
        let mut returned = 0u32;
        let _ = unsafe { EnumJobsW(printer, 0, u32::MAX, 2, None, &mut needed, &mut returned) };
        if needed == 0 {
            return Ok(Vec::new());
        }
        if needed > 16 * 1024 * 1024 {
            return Err(format!(
                "print queue response is too large ({needed} bytes)"
            ));
        }
        let word_count = (needed as usize).div_ceil(std::mem::size_of::<u64>());
        let mut buffer = vec![0u64; word_count];
        let byte_len = buffer.len() * std::mem::size_of::<u64>();
        let bytes =
            unsafe { std::slice::from_raw_parts_mut(buffer.as_mut_ptr().cast::<u8>(), byte_len) };
        unsafe {
            EnumJobsW(
                printer,
                0,
                u32::MAX,
                2,
                Some(bytes),
                &mut needed,
                &mut returned,
            )
        }
        .map_err(|error| format!("EnumJobsW: {error}"))?;
        if returned as usize > byte_len / std::mem::size_of::<JOB_INFO_2W>() {
            return Err("Windows returned an invalid print-job count".into());
        }
        let infos = unsafe {
            std::slice::from_raw_parts(buffer.as_ptr().cast::<JOB_INFO_2W>(), returned as usize)
        };
        Ok(infos
            .iter()
            .map(|job| SpoolJob {
                id: job.JobId,
                status: job.Status,
                detail: format!(
                    "status text {:?}, data type {:?}, processor {:?}, driver {:?}, {} bytes, {} pages",
                    text(job.pStatus),
                    text(job.pDatatype),
                    text(job.pPrintProcessor),
                    text(job.pDriverName),
                    job.Size,
                    job.TotalPages,
                ),
            })
            .collect())
    })();
    let _ = unsafe { ClosePrinter(printer) };
    result
}

/// A string the spooler returned inside the job buffer, or empty.
fn text(value: ::windows::core::PWSTR) -> String {
    if value.is_null() {
        return String::new();
    }
    // SAFETY: EnumJobsW points these into the buffer it filled, which
    // outlives this call, and each is NUL-terminated.
    unsafe { value.to_string() }.unwrap_or_default()
}

fn control_spool_job(printer_name: &str, job_id: u32, command: u32) -> Result<(), String> {
    let name = HSTRING::from(printer_name);
    let defaults = PRINTER_DEFAULTSW {
        pDatatype: ::windows::core::PWSTR::null(),
        pDevMode: std::ptr::null_mut::<DEVMODEW>(),
        DesiredAccess: PRINTER_ACCESS_ADMINISTER,
    };
    let mut printer = PRINTER_HANDLE::default();
    unsafe { OpenPrinterW(&name, &mut printer, Some(&defaults)) }
        .map_err(|error| format!("OpenPrinterW: {error}"))?;
    let done = unsafe { SetJobW(printer, job_id, 0, None, command) }.as_bool();
    let control_error = if done {
        None
    } else {
        Some(unsafe { GetLastError() }.0)
    };
    let close_result = unsafe { ClosePrinter(printer) };
    if let Some(error) = control_error {
        return Err(format!("SetJobW failed with Windows error {error}"));
    }
    close_result.map_err(|error| format!("ClosePrinter: {error}"))
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
    let (uri, marker) = {
        let running = server.0.lock().await;
        let endpoint = &running
            .as_ref()
            .ok_or("Printer receiver did not start")?
            .endpoint;
        (endpoint.http_uri(), endpoint.marker())
    };
    // Save the new address before Windows can be pointed at it: a queue on a
    // URL the app would not serve after a restart is what this order rules
    // out. If saving fails, Windows is not touched.
    if let Err(error) = server.persist_endpoint(&app).await {
        log::warn!("[campus-printer] the receiver address was not saved: {error}");
        server.revert(&app, previous).await;
        return Ok(CampusPrinterSetup::Failed);
    }
    let installed_marker = marker.clone();
    let result = tauri::async_runtime::spawn_blocking(move || install_queue(&uri, &marker))
        .await
        .map_err(|e| e.to_string())?;
    let outcome = match result {
        Ok(true) => CampusPrinterSetup::Added,
        Ok(false) => {
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
                "configure-queue-access" | "configure-print-defaults" | "verify-queue"
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

fn install_queue(uri: &str, marker: &str) -> Result<bool, InstallFailure> {
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
 if ($existing -and ($existing.DriverName -ne 'Microsoft IPP Class Driver' -or $existing.Location -ne $owner)) {{ throw 'Printer name belongs to another queue' }}
 # Only migrate this user's old app queues, never arbitrary printers or jobs.
 $stage='inspect-legacy-queues'
 $legacy=@(Get-Printer -Name 'IsctCompanion Campus Printer - *' -Full -ErrorAction SilentlyContinue | Where-Object {{ $_.Name -match '^IsctCompanion Campus Printer - [0-9a-f]{{32}}$' -and $_.DriverName -eq 'Microsoft IPP Class Driver' -and $_.PermissionSDDL -and $_.PermissionSDDL.Contains(';;;{sid})') }})
 $replace=$existing -and $existing.Comment -ne $marker
 $remove=@($legacy)
 if ($replace) {{ $remove+= $existing }}
 # Before anything is removed: a receiver that does not answer leaves the old queue in place.
 $stage='check-receiver'
 Invoke-WebRequest -Uri ('http://127.0.0.1:' + ([uri]$uri).Port + '/') -UseBasicParsing -ErrorAction Stop | Out-Null
 $stage='check-pending-jobs'
 foreach ($printer in $remove) {{ if (@(Get-PrintJob -PrinterName $printer.Name -ErrorAction Stop).Count -gt 0) {{ exit 2 }} }}
 $stage='remove-previous-queue'
 foreach ($printer in $remove) {{ Remove-Printer -Name $printer.Name -ErrorAction Stop }}
 $stage='add-ipp-queue'
 if (-not $existing -or $replace) {{ Add-Printer -Name $name -IppURL $uri -ErrorAction Stop }}
 $stage='configure-queue-access'
 # Windows' default permissions for a local printer. The component that hands
 # IPP jobs to the receiver opens the queue under its own identity: a list
 # narrowed to this user, SYSTEM and administrators refused it (access denied)
 # and every job failed before reaching the receiver. Repair resets older
 # queues installed with that list.
 Set-Printer -Name $name -Location $owner -Comment $marker -Shared $false -KeepPrintedJobs $false -PermissionSDDL 'G:SYD:(A;OIIO;GA;;;CO)(A;OIIO;GA;;;AC)(A;;SWRC;;;WD)(A;CIIO;GX;;;WD)(A;;SWRC;;;AC)(A;CIIO;GX;;;AC)(A;;LCSWDTSDRCWDWO;;;BA)(A;OICIIO;GA;;;BA)' -ErrorAction Stop
 $stage='configure-print-defaults'
 Set-PrintConfiguration -PrinterName $name -Color $false -DuplexingMode TwoSidedLongEdge -ErrorAction Stop
 $stage='verify-queue'
 $printer=Get-Printer -Name $name -Full -ErrorAction Stop
 if ($printer.DriverName -ne 'Microsoft IPP Class Driver' -or $printer.Comment -ne $marker) {{ throw 'Printer installation could not be verified' }}
 exit 0
}} catch {{
 $details=$_.Exception.Message
 foreach ($secret in @($uri,$marker,$owner)) {{ if ($secret) {{ $details=$details.Replace($secret,'[redacted]') }} }}
 try {{ [System.IO.File]::WriteAllText({diagnostic}, "$stage|$details", [System.Text.Encoding]::UTF8) }} catch {{}}
 $failureCode=49
 switch ($stage) {{
  'inspect-queue' {{ $failureCode=41 }}
  'inspect-legacy-queues' {{ $failureCode=42 }}
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
        diagnostic = quote(&diagnostic.to_string_lossy())
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
    // The installer performs only idempotent queue configuration. A late finish
    // after timeout is harmless; no document is passed to the elevated process.
    let wait = unsafe { WaitForSingleObject(handle, 120_000) };
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
            42 => Some("inspect-legacy-queues"),
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
        return Err("Windows did not confirm printer installation".into());
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
    let state = tauri::async_runtime::spawn_blocking(move || probe_queue(marker.as_deref()))
        .await
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|error| {
            log::warn!("[campus-printer] could not read the system printer: {error}");
            QueueState::Unavailable
        });
    log::info!("[campus-printer] system printer state {state:?}");
    Ok(state)
}

fn probe_queue(marker: Option<&str>) -> Result<QueueState, String> {
    let (shell, sid, name) = identity()?;
    let script = format!(
        r#"$ErrorActionPreference='Stop'
$p=Get-Printer -Name '{name}' -Full -ErrorAction SilentlyContinue
if ($p) {{ $p | Select-Object DriverName,Location,Comment | ConvertTo-Json -Compress }}
else {{
 $old=@(Get-Printer -Name 'IsctCompanion Campus Printer - *' -Full -ErrorAction SilentlyContinue | Where-Object {{ $_.Name -match '^IsctCompanion Campus Printer - [0-9a-f]{{32}}$' -and $_.DriverName -eq 'Microsoft IPP Class Driver' -and $_.PermissionSDDL -and $_.PermissionSDDL.Contains(';;;{sid})') }})
 if ($old.Count -gt 0) {{ '{{}}' }}
}}"#
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
