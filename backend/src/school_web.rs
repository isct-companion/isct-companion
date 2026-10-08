use crate::kyomu_ingest::{apply_kyomu_payload, KYOMU_EXTRACT_SCRIPT};
use crate::lifecycle::SchoolSite;
use crate::lms_ingest::apply_lms_payload;
use crate::school_auth::{
    upcoming_otps, OtpWindow, SchoolAuthProfile, SchoolAuthService, SchoolCredentials,
};
use crate::school_extract::{sanitize_school_url, T2SCHOLA_EXTRACT_SCRIPT};
use crate::school_parser::{
    kyomu::{parse_payload as parse_kyomu_payload, KyomuStage},
    lms::{parse_payload as parse_lms_payload, LmsStage},
};
use crate::secrets::OsSecretVault;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::utils::config::BackgroundThrottlingPolicy;
use tauri::webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Webview, WebviewUrl};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::Notify;
use url::Url;

#[cfg(target_os = "macos")]
#[path = "school_pdf_save.rs"]
mod pdf_save;

const MAIN_WINDOW: &str = "main";
// Stable UUID shared by every embedded school view. On Apple platforms this
// selects one WKWebsiteDataStore, so cookies/session storage survive view
// switches and are shared by the resident pool.
pub(crate) const SCHOOL_DATA_STORE: [u8; 16] = [
    0x63, 0x61, 0x6d, 0x70, 0x75, 0x73, 0x2d, 0x6c, 0x69, 0x66, 0x65, 0x2d, 0x77, 0x65, 0x62, 0x31,
];

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolWebPolicy {
    pub restore_across_restarts: bool,
    // Retained in the command contract for compatibility. The per-site pool
    // stays resident until the application exits, so it no longer idles out.
    pub idle_minutes: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct WebBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Default for WebBounds {
    fn default() -> Self {
        Self {
            x: 72.0,
            y: 60.0,
            width: 900.0,
            height: 620.0,
        }
    }
}

impl WebBounds {
    fn sanitized(self) -> Self {
        let clamp = |value: f64, max: f64| {
            if value.is_finite() {
                value.clamp(0.0, max)
            } else {
                0.0
            }
        };
        Self {
            x: clamp(self.x, 20_000.0),
            y: clamp(self.y, 20_000.0),
            width: clamp(self.width, 20_000.0).max(1.0),
            height: clamp(self.height, 20_000.0).max(1.0),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolWebResult {
    pub site: SchoolSite,
    pub url: String,
    pub phase: &'static str,
}

/// One extractor result, reported to waiting pulls and to the main window.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestOutcome {
    pub site: SchoolSite,
    pub kind: String,
    /// `ok`, `reconnect-required` or `error`.
    pub state: &'static str,
    pub changed: usize,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolLoginEvent {
    pub profile: SchoolAuthProfile,
    pub state: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolPullProgress {
    pub site: SchoolSite,
    pub phase: String,
    pub completed: usize,
    pub total: usize,
    #[serde(default)]
    pub course_id: u64,
    #[serde(default)]
    pub course_title: String,
    #[serde(default)]
    pub term: String,
    #[serde(default)]
    pub item_title: String,
}

enum SchoolJob {
    Session(SchoolAuthProfile),
    Login {
        profile: SchoolAuthProfile,
        credentials: SchoolCredentials,
        started: mpsc::Sender<Result<SchoolWebResult, String>>,
    },
    Pull {
        site: SchoolSite,
        lms_course_id: Option<u64>,
        finished: mpsc::Sender<Result<Vec<IngestOutcome>, String>>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActiveSchoolJob {
    Session(SchoolAuthProfile),
    Login(SchoolAuthProfile),
    Pull {
        site: SchoolSite,
        lms_course_id: Option<u64>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SchoolPullProgressRelay {
    phase: String,
    completed: usize,
    total: usize,
    #[serde(default)]
    course_id: u64,
    #[serde(default)]
    course_title: String,
    #[serde(default)]
    term: String,
    #[serde(default)]
    item_title: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SchoolLoginRelay {
    profile: SchoolAuthProfile,
    run: String,
    state: String,
    message: String,
}

/// What a login relay from the page may do to the active attempt.
#[derive(Debug, PartialEq)]
enum RelayOutcome {
    /// A step started; the attempt stays armed.
    Progress,
    /// Automation cannot continue alone; stay armed so it resumes after the
    /// person completes the step in the visible page.
    NeedsUser,
    /// The attempt ended with a failure and must not auto-retry.
    Failed,
}

fn relay_outcome(state: &str, message: &str) -> Option<RelayOutcome> {
    match state {
        "progress" => matches!(
            message,
            "agree" | "identifier" | "password" | "method" | "otp" | "matrix"
        )
        .then_some(RelayOutcome::Progress),
        "needs-user" => Some(RelayOutcome::NeedsUser),
        "error"
        | "invalid-credentials"
        | "invalid-otp"
        | "invalid-matrix"
        | "locked"
        | "maintenance" => Some(RelayOutcome::Failed),
        _ => None,
    }
}

#[derive(Clone)]
struct ActiveLogin {
    profile: SchoolAuthProfile,
    credentials: SchoolCredentials,
    /// Identifies one attempt so the page can budget retries across reloads
    /// and stale relays from an earlier attempt are ignored.
    run: String,
}

fn outcome(site: SchoolSite, kind: &str, result: Result<usize, String>) -> IngestOutcome {
    match result {
        Ok(changed) => IngestOutcome {
            site,
            kind: kind.into(),
            state: "ok",
            changed,
            message: format!("{changed} rows changed"),
        },
        Err(message) if message == "reconnect-required" => IngestOutcome {
            site,
            kind: kind.into(),
            state: "reconnect-required",
            changed: 0,
            message: "Sign-in required".into(),
        },
        Err(message) => IngestOutcome {
            site,
            kind: kind.into(),
            state: "error",
            changed: 0,
            message,
        },
    }
}

const KYOMU_ROOT: &str = "https://kyomu0.gakumu.titech.ac.jp/Titech/";
/// Read-only pages the 教務Web pull loads, in order (GPA first: Today needs it).
const KYOMU_PULL_PAGES: [&str; 5] = [
    "Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_7.aspx",
    "Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_0.aspx",
    "Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_1.aspx",
    "Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_0.aspx",
    "Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_6.aspx",
];

fn new_run_id() -> String {
    let mut bytes = [0u8; 12];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginCaptureRelay {
    url: String,
    html: String,
}

fn clean_login_message(raw: &str) -> String {
    raw.chars()
        .filter(|character| !character.is_control() || character.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(300)
        .collect()
}

fn emit_login_status(app: &AppHandle, profile: SchoolAuthProfile, state: &str, message: &str) {
    let message = clean_login_message(message);
    let _ = app.emit_to(
        MAIN_WINDOW,
        "school-login-status",
        SchoolLoginEvent {
            profile,
            state: state.to_string(),
            message: if message.is_empty() {
                "The school portal did not provide an error message.".into()
            } else {
                message
            },
        },
    );
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolSessionEvent {
    pub profile: SchoolAuthProfile,
    /// `active` or `failed`; "not configured" is known from the vault.
    pub state: &'static str,
}

fn emit_session(
    controller: &Arc<Mutex<ControllerState>>,
    app: &AppHandle,
    profile: SchoolAuthProfile,
    state: &'static str,
) {
    let changed = controller.lock().ok().map(|mut controller| {
        controller.sessions.insert(profile, state);
        controller.session_changed.clone()
    });
    if let Some(changed) = changed {
        changed.notify_waiters();
    }
    eprintln!("[school-session] {profile:?} {state}");
    let _ = app.emit_to(
        MAIN_WINDOW,
        "school-session",
        SchoolSessionEvent { profile, state },
    );
}

/// What a page load tells a pending session check: reaching the portal home
/// means the session works. Landing on its sign-in page means it expired; with
/// saved credentials an automatic sign-in is already running, so the check
/// waits for that to finish (success arrives as a completed login, failure as
/// a relay). Without one, the session has failed.
fn session_outcome(
    raw_url: &str,
    profile: SchoolAuthProfile,
    signing_in: bool,
) -> Option<&'static str> {
    if login_succeeded(raw_url, profile) {
        return Some("active");
    }
    let host = Url::parse(raw_url).ok()?.host_str()?.to_string();
    (SchoolAuthProfile::for_auth_host(&host) == Some(profile) && !signing_in).then_some("failed")
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RestoreRecord {
    site: SchoolSite,
    url: String,
}

struct ControllerState {
    webviews: HashMap<SchoolSite, Webview>,
    active: Option<SchoolSite>,
    current_site: SchoolSite,
    urls: HashMap<SchoolSite, String>,
    loaded_urls: HashMap<SchoolSite, String>,
    restore: bool,
    bounds: WebBounds,
    customizations: HashMap<SchoolSite, (String, String)>,
    login: Option<ActiveLogin>,
    /// Profiles whose last attempt failed; saved credentials are not replayed
    /// automatically until the person starts a new sign-in.
    auto_blocked: HashSet<SchoolAuthProfile>,
    /// A launch-time session check waiting for its portal page to load.
    heartbeat: Option<SchoolAuthProfile>,
    /// Authoritative result of the latest completed session check/login.
    sessions: HashMap<SchoolAuthProfile, &'static str>,
    session_changed: Arc<Notify>,
    creating: HashSet<SchoolSite>,
    ingestion_relays: HashMap<SchoolSite, String>,
    pending_ingestion: HashSet<SchoolSite>,
    /// Limits the next hidden LMS extraction to one course.
    pending_lms_course: Option<u64>,
    /// The next LMS extraction was explicitly requested as a full background pull.
    /// Ordinary page loads derive their narrow scope from the loaded Moodle page.
    pending_lms_full: bool,
    /// Pulls waiting for their site's next extractor result.
    ingest_waiters: HashMap<SchoolSite, Vec<mpsc::Sender<IngestOutcome>>>,
    /// School pulls are serialized because sign-in state and hidden navigation
    /// are shared even though each service has its own resident WebView.
    pulling: Option<SchoolSite>,
    /// The single owner of school navigation/authentication. Every request,
    /// including scheduled work, enters this queue before touching a WebView.
    active_job: Option<ActiveSchoolJob>,
    jobs: VecDeque<SchoolJob>,
    /// Pulls waiting for a page whose URL starts with the prefix to finish loading.
    load_waiters: Vec<(SchoolSite, String, mpsc::Sender<String>)>,
    /// Registration queue shown on 教務Web pages while set.
    guide: Vec<crate::kyomu_guide::GuideItem>,
    /// Pages the person visited in each view, for the app's own Back.
    history: HashMap<SchoolSite, PageHistory>,
    /// Sites whose next load comes from Back and must not be recorded again.
    suppress_history: HashSet<SchoolSite>,
    /// Same-service popups shown in place, and the page each was opened from.
    popups: HashMap<SchoolSite, PopupReturn>,
    /// Downloads WebKit is writing: original request URL and destination
    /// (macOS reports no path when a download finishes).
    downloads: Vec<(String, PathBuf)>,
    /// Files saved from school pages this session; only these can be revealed.
    saved_files: HashSet<PathBuf>,
    /// The PDF save shim per view (`WKWebView.UIDelegate` is weak).
    #[cfg(target_os = "macos")]
    ui_delegates: HashMap<SchoolSite, pdf_save::DelegateHandle>,
}

impl Default for ControllerState {
    fn default() -> Self {
        Self {
            webviews: HashMap::new(),
            active: None,
            current_site: SchoolSite::SciencePortal,
            urls: HashMap::new(),
            loaded_urls: HashMap::new(),
            restore: false,
            bounds: WebBounds::default(),
            customizations: HashMap::new(),
            login: None,
            auto_blocked: HashSet::new(),
            heartbeat: None,
            sessions: HashMap::new(),
            session_changed: Arc::new(Notify::new()),
            creating: HashSet::new(),
            ingestion_relays: HashMap::new(),
            pending_ingestion: HashSet::new(),
            pending_lms_course: None,
            pending_lms_full: false,
            ingest_waiters: HashMap::new(),
            pulling: None,
            active_job: None,
            jobs: VecDeque::new(),
            load_waiters: Vec::new(),
            guide: Vec::new(),
            history: HashMap::new(),
            suppress_history: HashSet::new(),
            popups: HashMap::new(),
            downloads: Vec::new(),
            saved_files: HashSet::new(),
            #[cfg(target_os = "macos")]
            ui_delegates: HashMap::new(),
        }
    }
}

#[derive(Clone)]
pub struct SchoolWebController {
    state: Arc<Mutex<ControllerState>>,
}

fn apply_bounds(webview: &Webview, bounds: WebBounds) -> Result<(), String> {
    let bounds = bounds.sanitized();
    webview
        .set_position(LogicalPosition::new(bounds.x, bounds.y))
        .and_then(|_| webview.set_size(LogicalSize::new(bounds.width, bounds.height)))
        .map_err(|_| "School view bounds could not be updated.".to_string())
}

fn site_for_url(raw: &str) -> Option<SchoolSite> {
    SchoolSite::for_url(raw)
}

/// Where a link that asks for a new window (`target="_blank"`, `window.open`)
/// goes. Embedded views have no tabs, and WebKit silently drops such a click
/// unless the view answers it, so every request is routed and none opens a
/// real popup.
#[derive(Debug, PartialEq)]
enum NewWindowTarget {
    /// An official school page, shown by that service's resident view.
    School(SchoolSite),
    /// Any other plain web page, handed to the system browser.
    External(String),
    Ignored,
}

fn new_window_target(raw: &str) -> NewWindowTarget {
    if sanitize_school_url(raw).is_some() {
        if let Some(site) = site_for_url(raw) {
            return NewWindowTarget::School(site);
        }
    }
    let Ok(url) = Url::parse(raw) else {
        return NewWindowTarget::Ignored;
    };
    let plain = matches!(url.scheme(), "https" | "http")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none();
    if plain {
        NewWindowTarget::External(url.to_string())
    } else {
        NewWindowTarget::Ignored
    }
}

#[derive(Clone, Debug, Serialize)]
struct SchoolWebOpen {
    site: SchoolSite,
    url: String,
}

const HISTORY_LIMIT: usize = 50;

/// Back stack of one view. Back always loads an earlier page as a fresh GET:
/// `history.back()` would resubmit 教務Web's WebForms postbacks (ViewState,
/// "不正な画面遷移" guards, a second 申告).
#[derive(Debug, Default)]
struct PageHistory {
    pages: Vec<String>,
}

impl PageHistory {
    /// Records a loaded page; a postback reloading the same URL is one entry.
    fn push(&mut self, url: String) {
        if self.pages.last() == Some(&url) {
            return;
        }
        self.pages.push(url);
        if self.pages.len() > HISTORY_LIMIT {
            self.pages.remove(0);
        }
    }

    /// The newest page that is not the current one. Entries after it (the
    /// current page) are dropped; the target stays as the new current page.
    fn back_target(&mut self, current: Option<&str>) -> Option<String> {
        let index = self
            .pages
            .iter()
            .rposition(|page| Some(page.as_str()) != current)?;
        self.pages.truncate(index + 1);
        self.pages.last().cloned()
    }

    /// Forgets everything after the newest visit to `url`.
    fn rewind_to(&mut self, url: &str) {
        if let Some(index) = self.pages.iter().rposition(|page| page == url) {
            self.pages.truncate(index + 1);
        }
    }
}

/// Pages worth returning to: the view's own service, never a sign-in step
/// or SAML hand-off (an IdP host only counts once its portal is reached).
fn history_entry(site: SchoolSite, raw_url: &str) -> Option<String> {
    let safe = sanitize_school_url(raw_url)?;
    if site_for_url(&safe) != Some(site) {
        return None;
    }
    let host = Url::parse(&safe).ok()?.host_str()?.to_string();
    match SchoolAuthProfile::for_auth_host(&host) {
        Some(profile) if !login_succeeded(&safe, profile) => None,
        _ => Some(safe),
    }
}

/// A same-service popup shown in place of the page that opened it.
#[derive(Clone, Debug, PartialEq)]
struct PopupReturn {
    opener: String,
    popup: String,
}

#[derive(Debug, PartialEq)]
enum PopupStep {
    /// Still on the popup (its own postbacks included).
    Stay,
    /// Back on the opener, e.g. after 閉じる.
    Returned,
    /// Anywhere else: the popup is over.
    Left,
}

/// Same document, ignoring the query (postbacks and sanitizing change it).
fn same_page(left: &str, right: &str) -> bool {
    match (Url::parse(left), Url::parse(right)) {
        (Ok(left), Ok(right)) => {
            left.scheme() == right.scheme()
                && left.host_str() == right.host_str()
                && left.path() == right.path()
        }
        _ => false,
    }
}

impl PopupReturn {
    fn step(&self, loaded: &str) -> PopupStep {
        if same_page(loaded, &self.popup) {
            PopupStep::Stay
        } else if same_page(loaded, &self.opener) {
            PopupStep::Returned
        } else {
            PopupStep::Left
        }
    }
}

/// WebKit ignores `window.close()` on a page no script opened, so a popup's
/// 閉じる button would do nothing in place. It returns to the opener with a
/// fresh GET when known, else steps back. Sign-in pages are left alone.
fn close_override_script(raw_url: &str, return_to: Option<&str>) -> Option<String> {
    sanitize_school_url(raw_url)?;
    let host = Url::parse(raw_url).ok()?.host_str()?.to_string();
    if SchoolAuthProfile::for_auth_host(&host).is_some() {
        return None;
    }
    let action = match return_to.filter(|url| sanitize_school_url(url).is_some()) {
        Some(url) => format!(
            "location.replace({})",
            serde_json::to_string(url)
                .ok()?
                .replace('<', "\\u003c")
                .replace('>', "\\u003e")
        ),
        None => "history.back()".into(),
    };
    Some(format!("window.close=()=>{action};"))
}

/// Updates the Back stack and popup state for a finished load, and returns
/// the `window.close` override the page needs.
fn track_page(state: &mut ControllerState, site: SchoolSite, raw_url: &str) -> Option<String> {
    let suppressed = state.suppress_history.remove(&site);
    let mut return_to = None;
    if let (Some(popup), Some(safe)) = (
        state.popups.get(&site).cloned(),
        sanitize_school_url(raw_url),
    ) {
        match popup.step(&safe) {
            PopupStep::Stay => return_to = Some(popup.opener),
            PopupStep::Returned => {
                state.popups.remove(&site);
                // Closing the popup drops it from Back, like a closed window.
                if let Some(history) = state.history.get_mut(&site) {
                    history.rewind_to(&popup.opener);
                }
            }
            PopupStep::Left => {
                state.popups.remove(&site);
            }
        }
    }
    // Hidden loads (pulls) are not the person's browsing.
    if !suppressed && state.active == Some(site) {
        if let Some(entry) = history_entry(site, raw_url) {
            state.history.entry(site).or_default().push(entry);
        }
    }
    close_override_script(raw_url, return_to.as_deref())
}

fn ingestion_script(raw: &str) -> Option<&'static str> {
    let url = Url::parse(raw).ok()?;
    // Files (`pluginfile.php`) shown in the view are documents, not LMS pages.
    if url.host_str() == Some("lms.s.isct.ac.jp")
        && url.path().starts_with("/2025/")
        && !url.path().contains("/pluginfile.php/")
    {
        return Some(T2SCHOLA_EXTRACT_SCRIPT);
    }
    (url.host_str() == Some("kyomu0.gakumu.titech.ac.jp")
        && matches!(
            url.path(),
            "/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_0.aspx"
                | "/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_1.aspx"
                | "/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_0.aspx"
                | "/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_6.aspx"
                | "/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_7.aspx"
        ))
    .then_some(KYOMU_EXTRACT_SCRIPT)
}

const SCHOOL_LOGIN_SCRIPT: &str = include_str!("../scripts/school-login.js");
/// Shows 教務Web's popups in an overlay frame inside the opener page, with a
/// working `window.opener` and `window.close()`. Gated to 教務Web by the
/// script itself; runs at document start in every frame.
const KYOMU_POPUP_SCRIPT: &str = include_str!("../scripts/kyomu-popup.js");
/// Sends cross-origin `target="_blank"` links through `window.open`, so they
/// reach `on_new_window` instead of the navigation policy.
const SCHOOL_LINKS_SCRIPT: &str = include_str!("../scripts/school-links.js");

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginScriptConfig<'a> {
    profile: &'static str,
    run: &'a str,
    username: &'a str,
    password: &'a str,
    otp: Vec<OtpWindow>,
    matrix: &'a str,
    auth_relay: &'a str,
    debug_relay: Option<&'a str>,
}

fn login_script(
    raw_url: &str,
    login: &ActiveLogin,
    auth_relay: &str,
    debug_relay: Option<&str>,
) -> Option<String> {
    let url = Url::parse(raw_url).ok()?;
    if url.scheme() != "https"
        || SchoolAuthProfile::for_auth_host(url.host_str()?) != Some(login.profile)
        || !is_login_step(&url)
    {
        return None;
    }
    let credentials = &login.credentials;
    // The code field can appear a while after this page loads; the script
    // picks whichever window is live when it fills.
    let otp = if credentials.otp_secret.is_empty() {
        Vec::new()
    } else {
        upcoming_otps(&credentials.otp_secret, 3).ok()?
    };
    let profile = match login.profile {
        SchoolAuthProfile::Science => "science",
        SchoolAuthProfile::TokyoTech => "tokyo-tech",
    };
    let config = serde_json::to_string(&LoginScriptConfig {
        profile,
        run: &login.run,
        username: &credentials.username,
        password: &credentials.password,
        otp,
        matrix: &credentials.matrix,
        auth_relay,
        debug_relay,
    })
    .ok()?
    .replace('<', "\\u003c")
    .replace('>', "\\u003e");
    Some(SCHOOL_LOGIN_SCRIPT.replace("__ISCT_COMPANION_LOGIN_CONFIG__", &config))
}

/// Pages that ask for something. SAML hand-offs on the same origin
/// auto-submit a form on load and must be left alone.
fn is_login_step(url: &Url) -> bool {
    let path = url.path().trim_end_matches('/');
    match url.host_str() {
        Some("isct.ex-tic.com") => {
            matches!(
                path,
                "/auth" | "/auth/session" | "/auth/session/second_factor"
            )
        }
        Some("portal.nap.gsic.titech.ac.jp" | "portal.titech.ac.jp") => {
            matches!(path, "" | "/portal.pl" | "/GetAccess/Login")
        }
        _ => false,
    }
}

/// Handles a login report the page wrote into its title.
fn handle_login_relay(state: &Arc<Mutex<ControllerState>>, app: &AppHandle, encoded: &str) {
    if encoded.len() > 4 * 1024 {
        return;
    }
    let Some(relay) = URL_SAFE_NO_PAD
        .decode(encoded)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<SchoolLoginRelay>(&bytes).ok())
    else {
        return;
    };
    let Some(relay_result) = relay_outcome(&relay.state, &relay.message) else {
        return;
    };
    let mut failed = false;
    let matches_active = state.lock().ok().is_some_and(|mut state| {
        let matches = state
            .login
            .as_ref()
            .is_some_and(|active| active.profile == relay.profile && active.run == relay.run);
        if matches && relay_result == RelayOutcome::Failed {
            state.login = None;
            state.auto_blocked.insert(relay.profile);
            if state.heartbeat == Some(relay.profile) {
                state.heartbeat = None;
            }
            failed = true;
        }
        matches
    });
    if matches_active {
        eprintln!(
            "[school-login] {:?} {} {}",
            relay.profile, relay.state, relay.message
        );
        emit_login_status(app, relay.profile, &relay.state, &relay.message);
    }
    if failed {
        emit_session(state, app, relay.profile, "failed");
        let interrupted = state.lock().ok().and_then(|mut state| {
            let site = state.pulling?;
            let waiters = state.ingest_waiters.remove(&site).unwrap_or_default();
            state.pending_ingestion.remove(&site);
            Some((site, waiters))
        });
        if let Some((site, waiters)) = interrupted {
            let stopped = outcome(
                site,
                if site == SchoolSite::ScienceLms {
                    "lms"
                } else {
                    "session"
                },
                Err("reconnect-required".into()),
            );
            for waiter in waiters {
                let _ = waiter.send(stopped.clone());
            }
            let _ = app.emit_to(MAIN_WINDOW, "school-ingest", stopped);
        }
        SchoolWebController {
            state: state.clone(),
        }
        .complete_session_job(app, relay.profile);
    }
}

fn handle_pull_progress(state: &Arc<Mutex<ControllerState>>, app: &AppHandle, encoded: &str) {
    if encoded.len() > 4 * 1024 {
        return;
    }
    let Some(progress) = URL_SAFE_NO_PAD
        .decode(encoded)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<SchoolPullProgressRelay>(&bytes).ok())
    else {
        return;
    };
    if !matches!(
        progress.phase.as_str(),
        "catalog" | "course" | "material" | "assignment"
    ) || progress.total > 10_000
        || progress.completed > progress.total
        || progress.course_title.len() > 512
        || progress.item_title.len() > 512
        || progress.term.len() > 16
    {
        return;
    }
    let active = state
        .lock()
        .ok()
        .is_some_and(|state| state.pulling == Some(SchoolSite::ScienceLms));
    if !active {
        return;
    }
    let _ = app.emit_to(
        MAIN_WINDOW,
        "school-pull-progress",
        SchoolPullProgress {
            site: SchoolSite::ScienceLms,
            phase: progress.phase,
            completed: progress.completed,
            total: progress.total,
            course_id: progress.course_id,
            course_title: progress.course_title,
            term: progress.term,
            item_title: progress.item_title,
        },
    );
}

fn login_succeeded(raw_url: &str, profile: SchoolAuthProfile) -> bool {
    let Ok(url) = Url::parse(raw_url) else {
        return false;
    };
    let host = url.host_str().unwrap_or_default();
    match profile {
        // Either the IdP's own portal or a service it signed us into.
        SchoolAuthProfile::Science => {
            (host == "isct.ex-tic.com" && url.path().starts_with("/idm/user/portal"))
                || (host == "lms.s.isct.ac.jp"
                    && !url.path().contains("/login")
                    && !url.path().contains("saml"))
        }
        SchoolAuthProfile::TokyoTech => {
            (host == "portal.nap.gsic.titech.ac.jp" && url.path().contains("ResourceList"))
                || host == "kyomu0.gakumu.titech.ac.jp"
                // TSUBAME after its GetAccess SSO (not its own mail-code login page).
                || (host == "portal.t4.gsic.titech.ac.jp"
                    && url.path().starts_with("/ptl/")
                    && !matches!(url.path(), "/ptl/" | "/ptl/requestLogin")
                    && !url.path().starts_with("/ptl/ga/"))
        }
    }
}

/// A file a school page handed over: a download, or a save from the PDF viewer.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SchoolDownloadEvent {
    site: SchoolSite,
    /// `started`, `done` or `failed`.
    state: &'static str,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
}

fn emit_download(
    app: &AppHandle,
    site: SchoolSite,
    state: &'static str,
    name: String,
    url: Option<String>,
    path: Option<&Path>,
) {
    eprintln!("[school-download] {site:?} {state} {name}");
    let _ = app.emit_to(
        MAIN_WINDOW,
        "school-download",
        SchoolDownloadEvent {
            site,
            state,
            name,
            url,
            path: path.and_then(Path::to_str).map(str::to_string),
        },
    );
}

/// Only official school pages, and only in the view the person is looking at
/// (hidden pulls never download).
fn download_allowed(raw_url: &str, site: SchoolSite, active: Option<SchoolSite>) -> bool {
    active == Some(site) && sanitize_school_url(raw_url).is_some()
}

fn safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name.len() <= 255
        && !name
            .chars()
            .any(|character| matches!(character, '/' | '\\') || character.is_control())
}

/// WebKit picks `~/Downloads/<suggested name>`; anything else is refused.
fn download_destination_ok(destination: &Path, download_dir: &Path) -> bool {
    destination.parent() == Some(download_dir)
        && destination
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(safe_file_name)
}

/// UTF-8 bytes: macOS allows 255 per name, which leaves room for ` (n)`.
const DOWNLOAD_NAME_LIMIT: usize = 200;

/// `name.ext` → (`name`, `.ext`); short trailing extensions only.
fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(index) if index > 0 && name.len() - index <= 16 => name.split_at(index),
        _ => (name, ""),
    }
}

/// A plain file name for data the PDF viewer hands over: no separators,
/// control characters or leading dots, bounded, never empty.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn download_file_name(suggested: &str, mime_type: &str) -> String {
    let cleaned = suggested
        .chars()
        .filter(|character| !character.is_control())
        .map(|character| {
            if matches!(character, '/' | '\\' | ':') {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    let cleaned = cleaned.trim().trim_start_matches('.').trim_start();
    let pdf = mime_type.eq_ignore_ascii_case("application/pdf");
    let mut name = match cleaned {
        "" if pdf => "download.pdf".to_string(),
        "" => "download".to_string(),
        cleaned => cleaned.to_string(),
    };
    if pdf && !name.to_ascii_lowercase().ends_with(".pdf") {
        name.push_str(".pdf");
    }
    if name.len() <= DOWNLOAD_NAME_LIMIT {
        return name;
    }
    let (stem, extension) = split_extension(&name);
    let mut keep = DOWNLOAD_NAME_LIMIT.saturating_sub(extension.len());
    while !stem.is_char_boundary(keep) {
        keep -= 1;
    }
    format!("{}{extension}", stem[..keep].trim_end())
}

/// `name.ext`, then `name (1).ext`, `name (2).ext`, … like WebKit downloads.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn download_candidates(name: &str) -> impl Iterator<Item = String> + '_ {
    let (stem, extension) = split_extension(name);
    std::iter::once(name.to_string())
        .chain((1..1000).map(move |number| format!("{stem} ({number}){extension}")))
}

/// Writes `bytes` under the first free candidate name; never overwrites.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn write_unique(directory: &Path, name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    for candidate in download_candidates(name) {
        let path = directory.join(candidate);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
                    let _ = fs::remove_file(&path);
                    return Err(error);
                }
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free file name",
    ))
}

/// Records a finished file and tells the Web page.
fn finish_download(
    app: &AppHandle,
    state: &Arc<Mutex<ControllerState>>,
    site: SchoolSite,
    name: String,
    saved: Option<PathBuf>,
) {
    match saved {
        Some(path) => {
            if let Ok(mut state) = state.lock() {
                state.saved_files.insert(path.clone());
            }
            emit_download(app, site, "done", name, None, Some(&path));
        }
        None => emit_download(app, site, "failed", name, None, None),
    }
}

/// The PDF viewer's ↓ button (macOS): writes the bytes it hands over to
/// `~/Downloads` off the main thread.
#[cfg(target_os = "macos")]
fn save_viewer_data(
    app: &AppHandle,
    state: &Arc<Mutex<ControllerState>>,
    site: SchoolSite,
    saved: pdf_save::SavedData,
) {
    let active = state.lock().ok().and_then(|state| state.active);
    let allowed = saved
        .origin
        .as_deref()
        .is_some_and(|origin| download_allowed(origin, site, active));
    if !allowed {
        eprintln!("[school-download] {site:?} PDF save refused");
        return;
    }
    let name = download_file_name(&saved.suggested_name, &saved.mime_type);
    let Ok(directory) = app.path().download_dir() else {
        emit_download(app, site, "failed", name, None, None);
        return;
    };
    let app = app.clone();
    let state = state.clone();
    std::thread::spawn(move || {
        let written = write_unique(&directory, &name, &saved.bytes);
        if let Err(error) = &written {
            eprintln!("[school-download] {site:?} PDF save failed: {error}");
        }
        let name = written
            .as_ref()
            .ok()
            .and_then(|path| path.file_name())
            .map(|file| file.to_string_lossy().into_owned())
            .unwrap_or(name);
        finish_download(&app, &state, site, name, written.ok());
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn syncs_from_lms_pages_but_not_from_files_shown_in_the_view() {
        assert!(ingestion_script("https://lms.s.isct.ac.jp/2025/my/").is_some());
        assert!(ingestion_script("https://lms.s.isct.ac.jp/2025/pluginfile.php/223879/mod_label/intro/oop2026-schedule.png").is_none());
    }

    use super::*;

    #[test]
    fn new_window_links_are_routed_instead_of_dropped() {
        assert_eq!(
            new_window_target("https://syllabus.s.isct.ac.jp/courses?jwc=202512345"),
            NewWindowTarget::School(SchoolSite::Syllabus)
        );
        assert_eq!(
            new_window_target("https://lms.s.isct.ac.jp/2025/my/"),
            NewWindowTarget::School(SchoolSite::ScienceLms)
        );
        assert_eq!(
            new_window_target("https://www.isct.ac.jp/ja/timetable.pdf"),
            NewWindowTarget::External("https://www.isct.ac.jp/ja/timetable.pdf".into())
        );
        // Plain http school pages are not embeddable but still open outside.
        assert_eq!(
            new_window_target("http://kyomu0.gakumu.titech.ac.jp/Titech/"),
            NewWindowTarget::External("http://kyomu0.gakumu.titech.ac.jp/Titech/".into())
        );
        for ignored in [
            "about:blank",
            "javascript:void(0)",
            "file:///etc/passwd",
            "https://user:secret@example.com/",
            "not a url",
        ] {
            assert_eq!(
                new_window_target(ignored),
                NewWindowTarget::Ignored,
                "{ignored}"
            );
        }
    }

    const KYOMU_OPENER: &str =
        "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_0.aspx";
    const KYOMU_POPUP: &str =
        "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_3_1.aspx";
    const KYOMU_TOP: &str = "https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx";

    #[test]
    fn history_skips_postbacks_and_keeps_the_newest_pages() {
        let mut history = PageHistory::default();
        history.push("a".into());
        history.push("a".into());
        history.push("b".into());
        history.push("a".into());
        assert_eq!(history.pages, ["a", "b", "a"]);
        let mut long = PageHistory::default();
        for index in 0..HISTORY_LIMIT + 5 {
            long.push(index.to_string());
        }
        assert_eq!(long.pages.len(), HISTORY_LIMIT);
        assert_eq!(long.pages.first().map(String::as_str), Some("5"));
    }

    #[test]
    fn back_targets_the_newest_other_page() {
        let mut history = PageHistory {
            pages: vec!["a".into(), "b".into(), "c".into()],
        };
        assert_eq!(history.back_target(Some("c")).as_deref(), Some("b"));
        assert_eq!(history.pages, ["a", "b"]);
        // A page the stack never saw (a hidden pull): back to the last one seen.
        assert_eq!(history.back_target(Some("pulled")).as_deref(), Some("b"));
        assert_eq!(history.back_target(Some("b")).as_deref(), Some("a"));
        assert_eq!(history.back_target(Some("a")), None);
        assert_eq!(history.pages, ["a"]);
        assert_eq!(PageHistory::default().back_target(None), None);
    }

    #[test]
    fn history_records_only_the_views_own_signed_in_pages() {
        assert_eq!(
            history_entry(SchoolSite::Kyomu, &format!("{KYOMU_POPUP}?token=x#top")).as_deref(),
            Some(KYOMU_POPUP)
        );
        assert_eq!(
            history_entry(
                SchoolSite::SciencePortal,
                "https://isct.ex-tic.com/idm/user/portal/"
            )
            .as_deref(),
            Some("https://isct.ex-tic.com/idm/user/portal/")
        );
        for (site, url) in [
            (
                SchoolSite::Kyomu,
                "https://portal.nap.gsic.titech.ac.jp/GetAccess/Login",
            ),
            (SchoolSite::Kyomu, "https://lms.s.isct.ac.jp/2025/my/"),
            (
                SchoolSite::SciencePortal,
                "https://isct.ex-tic.com/auth/session",
            ),
            (
                SchoolSite::Kyomu,
                "http://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx",
            ),
        ] {
            assert_eq!(history_entry(site, url), None, "{url}");
        }
    }

    fn visible_kyomu() -> ControllerState {
        ControllerState {
            active: Some(SchoolSite::Kyomu),
            ..ControllerState::default()
        }
    }

    #[test]
    fn popup_close_returns_to_the_opener_until_the_popup_is_left() {
        let site = SchoolSite::Kyomu;
        let mut state = visible_kyomu();
        track_page(&mut state, site, KYOMU_TOP);
        track_page(&mut state, site, KYOMU_OPENER);
        state.popups.insert(
            site,
            PopupReturn {
                opener: KYOMU_OPENER.into(),
                popup: KYOMU_POPUP.into(),
            },
        );
        let close = track_page(&mut state, site, KYOMU_POPUP).unwrap();
        assert!(close.contains(&format!("location.replace(\"{KYOMU_OPENER}\")")));
        // The popup's own postbacks keep the way back.
        let close = track_page(&mut state, site, &format!("{KYOMU_POPUP}?page=2")).unwrap();
        assert!(close.contains("location.replace("));
        // 閉じる: back on the opener, the popup is gone from Back too.
        let close = track_page(&mut state, site, KYOMU_OPENER).unwrap();
        assert!(close.contains("history.back()"));
        assert!(state.popups.is_empty());
        assert_eq!(state.history[&site].pages, [KYOMU_TOP, KYOMU_OPENER]);

        state.popups.insert(
            site,
            PopupReturn {
                opener: KYOMU_OPENER.into(),
                popup: KYOMU_POPUP.into(),
            },
        );
        track_page(&mut state, site, KYOMU_POPUP);
        track_page(&mut state, site, KYOMU_TOP);
        assert!(state.popups.is_empty());
    }

    #[test]
    fn back_loads_are_not_recorded_twice_and_hidden_loads_not_at_all() {
        let site = SchoolSite::Kyomu;
        let mut state = ControllerState::default();
        track_page(&mut state, site, KYOMU_TOP);
        assert!(!state.history.contains_key(&site));
        state.active = Some(site);
        track_page(&mut state, site, KYOMU_TOP);
        state.suppress_history.insert(site);
        track_page(&mut state, site, KYOMU_OPENER);
        assert!(state.suppress_history.is_empty());
        assert_eq!(state.history[&site].pages, [KYOMU_TOP]);
    }

    #[test]
    fn popup_overlay_script_only_runs_on_kyomu() {
        // Every school view runs it in every frame, sign-in pages included.
        assert!(KYOMU_POPUP_SCRIPT.contains(r#"const HOST = "kyomu0.gakumu.titech.ac.jp";"#));
        assert!(KYOMU_POPUP_SCRIPT.contains(
            r#"if (location.protocol !== "https:" || location.hostname !== HOST) return;"#
        ));
    }

    #[test]
    fn close_override_is_escaped_and_skips_sign_in_pages() {
        assert_eq!(
            close_override_script(KYOMU_POPUP, None).as_deref(),
            Some("window.close=()=>history.back();")
        );
        let script = close_override_script(
            KYOMU_POPUP,
            Some("https://kyomu0.gakumu.titech.ac.jp/Titech/a\"</script>.aspx"),
        )
        .unwrap();
        assert!(!script.contains("</script>"));
        assert!(!script.contains("a\"<"));
        // Only official https pages are valid return targets.
        assert_eq!(
            close_override_script(KYOMU_POPUP, Some("https://example.com/")).as_deref(),
            Some("window.close=()=>history.back();")
        );
        for url in [
            "https://isct.ex-tic.com/auth/session",
            "https://portal.nap.gsic.titech.ac.jp/GetAccess/Login",
            "https://example.com/",
        ] {
            assert_eq!(
                close_override_script(url, Some(KYOMU_OPENER)),
                None,
                "{url}"
            );
        }
    }

    #[test]
    fn maps_every_embedded_destination_to_the_single_controller() {
        assert_eq!(
            site_for_url("https://isct.ex-tic.com/auth/session"),
            Some(SchoolSite::SciencePortal)
        );
        assert_eq!(
            site_for_url("https://lms.s.isct.ac.jp/2025/"),
            Some(SchoolSite::ScienceLms)
        );
        assert_eq!(
            site_for_url("https://syllabus.s.isct.ac.jp/"),
            Some(SchoolSite::Syllabus)
        );
        assert_eq!(site_for_url("https://example.com/"), None);
        assert_eq!(
            site_for_url("https://mailv4.m.titech.ac.jp/cgi-bin/loginweb"),
            Some(SchoolSite::TokyoMail)
        );
        assert_eq!(
            site_for_url("https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx"),
            Some(SchoolSite::Kyomu)
        );
        assert_eq!(
            site_for_url("https://request.libra.titech.ac.jp/portal/user.php"),
            Some(SchoolSite::Library)
        );
        assert_eq!(
            site_for_url("https://portal.t4.gsic.titech.ac.jp/ptl/ga/sso/"),
            Some(SchoolSite::Tsubame)
        );
    }

    fn active(profile: SchoolAuthProfile, credentials: SchoolCredentials) -> ActiveLogin {
        ActiveLogin {
            profile,
            credentials,
            run: "run".into(),
        }
    }

    #[test]
    fn login_scripts_are_restricted_to_the_matching_auth_origin() {
        let science = active(
            SchoolAuthProfile::Science,
            SchoolCredentials {
                username: "user".into(),
                password: "password".into(),
                otp_secret: "JBSWY3DPEHPK3PXP".into(),
                matrix: String::new(),
            },
        );
        assert!(login_script(
            "https://isct.ex-tic.com/auth/session",
            &science,
            "isct-companion-auth://test/",
            None,
        )
        .is_some());
        for url in [
            "https://portal.nap.gsic.titech.ac.jp/GetAccess/Login",
            "http://isct.ex-tic.com/auth/session",
            "https://lms.s.isct.ac.jp/2025/",
        ] {
            assert!(login_script(url, &science, "isct-companion-auth://test/", None).is_none());
        }
    }

    #[test]
    fn login_script_config_is_embedded_once_and_escaped() {
        let tokyo = active(
            SchoolAuthProfile::TokyoTech,
            SchoolCredentials {
                username: "user</script>".into(),
                password: "password".into(),
                otp_secret: String::new(),
                matrix: "A".repeat(70),
            },
        );
        let script = login_script(
            "https://portal.nap.gsic.titech.ac.jp/portal.pl",
            &tokyo,
            "isct-companion-auth://test/",
            None,
        )
        .unwrap();
        assert!(!script.contains("__ISCT_COMPANION_LOGIN_CONFIG__"));
        assert!(!script.contains("</script>"));
        assert!(script.contains("\"run\":\"run\""));
    }

    #[test]
    fn recognizes_only_each_profiles_authenticated_destination() {
        assert!(login_succeeded(
            "https://isct.ex-tic.com/idm/user/portal/",
            SchoolAuthProfile::Science
        ));
        assert!(login_succeeded(
            "https://lms.s.isct.ac.jp/2025/my/",
            SchoolAuthProfile::Science
        ));
        assert!(login_succeeded(
            "https://portal.nap.gsic.titech.ac.jp/GetAccess/ResourceList?lang=ja",
            SchoolAuthProfile::TokyoTech
        ));
        for url in [
            "https://isct.ex-tic.com/auth/session",
            "https://lms.s.isct.ac.jp/2025/login/index.php",
            "https://lms.s.isct.ac.jp/2025/auth/saml2/sp/saml2-acs.php/lms",
        ] {
            assert!(!login_succeeded(url, SchoolAuthProfile::Science), "{url}");
        }
        assert!(!login_succeeded(
            "https://portal.nap.gsic.titech.ac.jp/GetAccess/Login",
            SchoolAuthProfile::TokyoTech
        ));
    }

    #[test]
    fn relays_only_accept_known_states_and_steps() {
        assert_eq!(
            relay_outcome("progress", "password"),
            Some(RelayOutcome::Progress)
        );
        assert_eq!(relay_outcome("progress", "anything"), None);
        assert_eq!(
            relay_outcome("needs-user", "x"),
            Some(RelayOutcome::NeedsUser)
        );
        assert_eq!(
            relay_outcome("invalid-credentials", "x"),
            Some(RelayOutcome::Failed)
        );
        assert_eq!(relay_outcome("connected", "x"), None);
    }

    #[test]
    fn login_scripts_skip_saml_hand_off_pages() {
        let step = |raw: &str| is_login_step(&Url::parse(raw).unwrap());
        assert!(step("https://isct.ex-tic.com/auth/session"));
        assert!(step("https://isct.ex-tic.com/auth/session/second_factor"));
        assert!(step(
            "https://portal.nap.gsic.titech.ac.jp/GetAccess/Login?Template=x"
        ));
        assert!(step(
            "https://portal.nap.gsic.titech.ac.jp/portal.pl?GASF=x"
        ));
        assert!(!step(
            "https://isct.ex-tic.com/auth/saml2/abc/assertions/def"
        ));
        assert!(!step(
            "https://isct.ex-tic.com/idm/user/saml2/authenticate/user-isct"
        ));
    }

    #[test]
    fn session_state_comes_from_the_landing_page() {
        let science = SchoolAuthProfile::Science;
        assert_eq!(
            session_outcome("https://isct.ex-tic.com/idm/user/portal/", science, false),
            Some("active")
        );
        // Signing in with saved credentials: wait for its result.
        assert_eq!(
            session_outcome("https://isct.ex-tic.com/auth/session", science, true),
            None
        );
        assert_eq!(
            session_outcome("https://isct.ex-tic.com/auth/session", science, false),
            Some("failed")
        );
        assert_eq!(
            session_outcome(
                "https://lms.s.isct.ac.jp/2025/auth/saml2/sp/saml2-acs.php/lms",
                science,
                false
            ),
            None
        );
        assert_eq!(
            session_outcome(
                "https://portal.nap.gsic.titech.ac.jp/portal.pl?GASF=x",
                SchoolAuthProfile::TokyoTech,
                false
            ),
            Some("failed")
        );
    }

    #[test]
    fn auth_hosts_map_to_their_profile() {
        assert_eq!(
            SchoolAuthProfile::for_auth_host("isct.ex-tic.com"),
            Some(SchoolAuthProfile::Science)
        );
        assert_eq!(
            SchoolAuthProfile::for_auth_host("portal.nap.gsic.titech.ac.jp"),
            Some(SchoolAuthProfile::TokyoTech)
        );
        assert_eq!(SchoolAuthProfile::for_auth_host("lms.s.isct.ac.jp"), None);
    }

    #[test]
    fn downloads_are_limited_to_the_visible_school_view() {
        let lms = SchoolSite::ScienceLms;
        let file = "https://lms.s.isct.ac.jp/2025/pluginfile.php/224823/mod_resource/content/3/LAIIAssignment1.pdf";
        assert!(download_allowed(file, lms, Some(lms)));
        // Hidden views (pulls) and other views never download.
        assert!(!download_allowed(file, lms, None));
        assert!(!download_allowed(file, lms, Some(SchoolSite::Kyomu)));
        for url in [
            "https://example.com/file.pdf",
            "http://lms.s.isct.ac.jp/2025/pluginfile.php/1/file.pdf",
            "https://user:secret@lms.s.isct.ac.jp/file.pdf",
            "blob:https://lms.s.isct.ac.jp/uuid",
            "data:application/pdf;base64,AAAA",
        ] {
            assert!(!download_allowed(url, lms, Some(lms)), "{url}");
        }
    }

    #[test]
    fn download_destinations_stay_directly_in_the_download_folder() {
        let downloads = Path::new("/Users/student/Downloads");
        assert!(download_destination_ok(
            &downloads.join("Lecture 1 (2).pptx"),
            downloads
        ));
        for bad in [
            "/Users/student/Downloads/../.zshrc",
            "/Users/student/Downloads/nested/file.docx",
            "/Users/student/Desktop/file.docx",
            "/Users/student/Downloads/line\nbreak.pdf",
            "/Users/student/Downloads/back\\slash.pdf",
            "/Users/student/Downloads",
        ] {
            assert!(
                !download_destination_ok(Path::new(bad), downloads),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn viewer_file_names_are_plain_and_bounded() {
        let pdf = "application/pdf";
        assert_eq!(
            download_file_name("LAIIAssignment1.pdf", pdf),
            "LAIIAssignment1.pdf"
        );
        assert_eq!(
            download_file_name("../../etc/passwd", pdf),
            "_.._etc_passwd.pdf"
        );
        assert_eq!(download_file_name("..hidden.pdf", pdf), "hidden.pdf");
        assert_eq!(download_file_name("a\u{0}b\nc.pdf", pdf), "abc.pdf");
        assert_eq!(download_file_name("C:\\x\\講義.pdf", pdf), "C__x_講義.pdf");
        assert_eq!(download_file_name("Slides", pdf), "Slides.pdf");
        assert_eq!(download_file_name("Slides.PDF", pdf), "Slides.PDF");
        assert_eq!(download_file_name(" .. ", pdf), "download.pdf");
        assert_eq!(download_file_name("", "application/zip"), "download");
        assert_eq!(download_file_name("notes.txt", "text/plain"), "notes.txt");
        let long = download_file_name(&format!("{}.pdf", "講".repeat(400)), pdf);
        assert!(long.len() <= DOWNLOAD_NAME_LIMIT, "{}", long.len());
        assert!(long.ends_with("講.pdf"));
        assert!(safe_file_name(&long));
    }

    #[test]
    fn unique_names_number_before_the_extension() {
        let names = download_candidates("Report.final.pdf")
            .take(3)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "Report.final.pdf",
                "Report.final (1).pdf",
                "Report.final (2).pdf"
            ]
        );
        assert_eq!(
            download_candidates("README").nth(1).as_deref(),
            Some("README (1)")
        );
    }

    #[test]
    fn viewer_saves_never_overwrite_existing_files() {
        let directory =
            std::env::temp_dir().join(format!("isct-companion-download-test-{}", new_run_id()));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("a.pdf"), b"old").unwrap();
        let first = write_unique(&directory, "a.pdf", b"new").unwrap();
        let second = write_unique(&directory, "a.pdf", b"newer").unwrap();
        assert_eq!(first, directory.join("a (1).pdf"));
        assert_eq!(second, directory.join("a (2).pdf"));
        assert_eq!(fs::read(directory.join("a.pdf")).unwrap(), b"old");
        assert_eq!(fs::read(&second).unwrap(), b"newer");
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn portal_error_text_is_normalized_and_bounded() {
        assert_eq!(
            clean_login_message("  Invalid\n\tpassword  "),
            "Invalid password"
        );
        assert_eq!(clean_login_message(&"x".repeat(500)).len(), 300);
    }
}

impl SchoolWebController {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(ControllerState::default())),
        }
    }

    fn advance_jobs(&self, app: &AppHandle) {
        let job = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            if state.active_job.is_some() || state.active.is_some() {
                return;
            }
            let Some(job) = state.jobs.pop_front() else {
                return;
            };
            state.active_job = Some(match &job {
                SchoolJob::Session(profile) => ActiveSchoolJob::Session(*profile),
                SchoolJob::Login { profile, .. } => ActiveSchoolJob::Login(*profile),
                SchoolJob::Pull {
                    site,
                    lms_course_id,
                    ..
                } => ActiveSchoolJob::Pull {
                    site: *site,
                    lms_course_id: *lms_course_id,
                },
            });
            eprintln!(
                "[school-queue] starting {:?} · {} queued",
                state.active_job,
                state.jobs.len()
            );
            job
        };
        match job {
            SchoolJob::Session(profile) => {
                if let Err(error) = self.ensure_session_now(app, profile) {
                    emit_session(&self.state, app, profile, "failed");
                    eprintln!("[school-queue] session failed to start: {error}");
                    self.complete_session_job(app, profile);
                }
            }
            SchoolJob::Login {
                profile,
                credentials,
                started,
            } => {
                let result = self.login_now(app, profile, credentials);
                let failed = result.is_err();
                let _ = started.send(result);
                if failed {
                    self.complete_session_job(app, profile);
                }
            }
            SchoolJob::Pull {
                site,
                lms_course_id,
                finished,
            } => {
                let controller = self.clone();
                let handle = app.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let result = controller.pull_now(&handle, site, lms_course_id);
                    let _ = finished.send(result);
                    controller.complete_pull_job(&handle, site, lms_course_id);
                });
            }
        }
    }

    fn complete_session_job(&self, app: &AppHandle, profile: SchoolAuthProfile) {
        let completed = self.state.lock().ok().is_some_and(|mut state| {
            if matches!(
                state.active_job,
                Some(ActiveSchoolJob::Session(active) | ActiveSchoolJob::Login(active))
                    if active == profile
            ) {
                state.active_job = None;
                true
            } else {
                false
            }
        });
        if completed {
            eprintln!("[school-queue] session {profile:?} finished");
            self.advance_jobs(app);
        }
    }

    fn complete_pull_job(&self, app: &AppHandle, site: SchoolSite, lms_course_id: Option<u64>) {
        let completed = self.state.lock().ok().is_some_and(|mut state| {
            if state.active_job
                == Some(ActiveSchoolJob::Pull {
                    site,
                    lms_course_id,
                })
            {
                state.active_job = None;
                true
            } else {
                false
            }
        });
        if completed {
            eprintln!("[school-queue] pull {site:?} finished");
            self.advance_jobs(app);
        }
    }

    pub fn request_session(
        &self,
        app: &AppHandle,
        profile: SchoolAuthProfile,
    ) -> Result<bool, String> {
        let accepted = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "School web state is unavailable.".to_string())?;
            let duplicate = matches!(
                state.active_job,
                Some(ActiveSchoolJob::Session(active) | ActiveSchoolJob::Login(active))
                    if active == profile
            ) || state
                .jobs
                .iter()
                .any(|job| matches!(job, SchoolJob::Session(queued) if *queued == profile));
            if duplicate {
                eprintln!("[school-queue] duplicate {profile:?} session discarded");
                false
            } else {
                state.sessions.remove(&profile);
                state.jobs.push_back(SchoolJob::Session(profile));
                eprintln!("[school-queue] queued {profile:?} session");
                true
            }
        };
        if accepted {
            self.advance_jobs(app);
        }
        Ok(accepted)
    }

    /// Waits for the Rust-owned school queue to prove that a profile is
    /// signed in. Downloaders call this instead of inferring authentication
    /// from a public LMS URL such as robots.txt.
    pub async fn wait_for_session(
        &self,
        app: &AppHandle,
        profile: SchoolAuthProfile,
    ) -> Result<(), String> {
        if self
            .state
            .lock()
            .ok()
            .and_then(|state| state.sessions.get(&profile).copied())
            == Some("active")
        {
            return Ok(());
        }
        self.request_session(app, profile)?;
        let wait = async {
            loop {
                let (status, changed) = {
                    let state = self
                        .state
                        .lock()
                        .map_err(|_| "School web state is unavailable.".to_string())?;
                    (
                        state.sessions.get(&profile).copied(),
                        state.session_changed.clone(),
                    )
                };
                match status {
                    Some("active") => return Ok(()),
                    Some("failed") => return Err("School sign-in failed.".to_string()),
                    _ => changed.notified().await,
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(180), wait)
            .await
            .map_err(|_| "School sign-in did not finish in time.".to_string())?
    }

    pub fn request_login(
        &self,
        app: &AppHandle,
        profile: SchoolAuthProfile,
        credentials: SchoolCredentials,
    ) -> Result<SchoolWebResult, String> {
        let (sender, receiver) = mpsc::channel();
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "School web state is unavailable.".to_string())?;
            state.sessions.remove(&profile);
            state.jobs.push_back(SchoolJob::Login {
                profile,
                credentials,
                started: sender,
            });
        }
        self.advance_jobs(app);
        receiver
            .recv_timeout(Duration::from_secs(180))
            .map_err(|_| "The queued school sign-in did not start in time.".to_string())?
    }

    pub fn request_pull(
        &self,
        app: &AppHandle,
        site: SchoolSite,
        lms_course_id: Option<u64>,
    ) -> Result<Vec<IngestOutcome>, String> {
        let (sender, receiver) = mpsc::channel();
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "School web state is unavailable.".to_string())?;
            // A full LMS pull is authoritative. Later refresh clicks and
            // scheduled refreshes are discarded rather than replayed.
            if matches!(
                state.active_job,
                Some(ActiveSchoolJob::Pull {
                    site: SchoolSite::ScienceLms,
                    lms_course_id: None
                })
            ) {
                eprintln!("[school-queue] refresh discarded during full LMS pull");
                return Ok(Vec::new());
            }
            state.jobs.push_back(SchoolJob::Pull {
                site,
                lms_course_id,
                finished: sender,
            });
            eprintln!("[school-queue] queued {site:?} pull");
        }
        self.advance_jobs(app);
        receiver
            .recv_timeout(Duration::from_secs(600))
            .map_err(|_| "The queued school pull did not finish in time.".to_string())?
    }

    pub fn prepare_login(
        &self,
        profile: SchoolAuthProfile,
        credentials: SchoolCredentials,
    ) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "School web state is unavailable.")?;
        state.auto_blocked.remove(&profile);
        state.login = Some(ActiveLogin {
            profile,
            credentials,
            run: new_run_id(),
        });
        Ok(())
    }

    pub fn configure(&self, app: &AppHandle, policy: SchoolWebPolicy) -> Result<bool, String> {
        let _ = policy.idle_minutes;
        let mut state = self
            .state
            .lock()
            .map_err(|_| "School web state is unavailable.")?;
        state.restore = policy.restore_across_restarts;
        if !state.restore {
            let _ = fs::remove_file(Self::restore_path(app)?);
        } else if let Some(site) = state.active {
            if let Some(url) = state.urls.get(&site) {
                Self::write_restore(app, site, url)?;
            }
        }
        Ok(true)
    }

    pub fn set_bounds(&self, app: &AppHandle, bounds: WebBounds) -> Result<bool, String> {
        let (webview, bounds) = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "School web state is unavailable.")?;
            state.bounds = bounds.sanitized();
            (
                state
                    .active
                    .and_then(|site| state.webviews.get(&site).cloned()),
                state.bounds,
            )
        };
        if let Some(webview) = webview {
            apply_bounds(&webview, bounds)?;
        }
        let _ = app;
        Ok(true)
    }

    /// Hide resident school webviews. Their pages and shared session stay warm.
    pub fn hide_all(&self, app: &AppHandle) -> Result<bool, String> {
        let webviews = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "School web state is unavailable.")?;
            state.active = None;
            state.webviews.values().cloned().collect::<Vec<_>>()
        };
        for webview in webviews {
            webview
                .hide()
                .map_err(|error| format!("School view could not be hidden: {error}"))?;
        }
        self.advance_jobs(app);
        Ok(true)
    }

    pub fn reload(&self, app: &AppHandle, site: SchoolSite) -> Result<bool, String> {
        let webview = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "School web state is unavailable.")?;
            // A full pull owns hidden navigation and ingestion. An update click
            // during it is intentionally discarded instead of being replayed
            // against whatever page the pull happens to reach next.
            if state.active_job.is_some() || state.pulling.is_some() {
                return Ok(false);
            }
            if matches!(site, SchoolSite::ScienceLms | SchoolSite::Kyomu) {
                state.pending_ingestion.insert(site);
            }
            if site == SchoolSite::ScienceLms {
                state.pending_lms_course = None;
                state.pending_lms_full = false;
            }
            state.webviews.get(&site).cloned()
        };
        let _ = app;
        match webview {
            Some(webview) => webview
                .reload()
                .map(|_| true)
                .map_err(|_| "School view could not be reloaded.".into()),
            None => Ok(false),
        }
    }

    pub fn navigate(&self, app: &AppHandle, site: SchoolSite, raw: &str) -> Result<bool, String> {
        let target = sanitize_school_url(raw)
            .ok_or_else(|| "School URL is outside the allowed official origins.".to_string())?;
        if site_for_url(&target) != Some(site) {
            return Err("School URL belongs to a different service.".into());
        }
        let webview = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "School web state is unavailable.")?;
            state.urls.insert(site, target.clone());
            state.webviews.get(&site).cloned()
        };
        let Some(webview) = webview else {
            return Ok(false);
        };
        let parsed = Url::parse(&target).map_err(|_| "School URL is invalid.")?;
        webview
            .navigate(parsed)
            .map_err(|_| "School view could not navigate.".to_string())?;
        if self.state.lock().ok().is_some_and(|state| state.restore) {
            Self::write_restore(app, site, &target)?;
        }
        Ok(true)
    }

    /// Loads the previous page of the site's view as a fresh GET. False when
    /// there is no earlier page (or no view).
    pub fn back(&self, app: &AppHandle, site: SchoolSite) -> Result<bool, String> {
        let target = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "School web state is unavailable.")?;
            if !state.webviews.contains_key(&site) {
                return Ok(false);
            }
            let current = state.urls.get(&site).cloned();
            let target = state
                .history
                .get_mut(&site)
                .and_then(|history| history.back_target(current.as_deref()));
            if target.is_some() {
                state.suppress_history.insert(site);
            }
            target
        };
        let Some(target) = target else {
            return Ok(false);
        };
        eprintln!("[school-web] {site:?} back");
        let navigated = self.navigate(app, site, &target);
        if !matches!(navigated, Ok(true)) {
            if let Ok(mut state) = self.state.lock() {
                state.suppress_history.remove(&site);
            }
        }
        navigated
    }

    /// Shows a file saved from a school page this session in Finder. Any
    /// other path is refused (false).
    pub fn reveal_download(&self, app: &AppHandle, path: &str) -> Result<bool, String> {
        let path = PathBuf::from(path);
        let known = self
            .state
            .lock()
            .map_err(|_| "School web state is unavailable.")?
            .saved_files
            .contains(&path);
        if !known {
            return Ok(false);
        }
        app.opener()
            .reveal_item_in_dir(&path)
            .map(|_| true)
            .map_err(|_| "The file could not be shown. It may have been moved.".into())
    }

    pub fn set_zoom(&self, site: SchoolSite, scale: f64) -> Result<bool, String> {
        if !scale.is_finite() || !(0.5..=2.0).contains(&scale) {
            return Err("School view zoom must be between 50% and 200%.".into());
        }
        let webview = self
            .state
            .lock()
            .map_err(|_| "School web state is unavailable.")?
            .webviews
            .get(&site)
            .cloned();
        let Some(webview) = webview else {
            return Ok(false);
        };
        webview
            .set_zoom(scale)
            .map_err(|_| "School view zoom could not be updated.".to_string())?;
        Ok(true)
    }

    fn restore_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
        app.path()
            .app_data_dir()
            .map(|path| path.join("school-web-restore.json"))
            .map_err(|_| "School web data path is unavailable.".into())
    }

    fn write_restore(app: &AppHandle, site: SchoolSite, url: &str) -> Result<(), String> {
        let path = Self::restore_path(app)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|_| "School restore directory could not be created.")?;
        }
        let safe = sanitize_school_url(url).unwrap_or_else(|| site.start_url().to_string());
        let bytes = serde_json::to_vec(&RestoreRecord { site, url: safe })
            .map_err(|_| "School restore serialization failed.")?;
        let temporary = path.with_extension("json.tmp");
        let mut options = OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|_| "School restore temporary file could not be opened.")?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "School restore write failed.")?;
        fs::rename(temporary, path).map_err(|_| "School restore publication failed.".into())
    }

    fn restored_target(&self, app: &AppHandle, requested: SchoolSite) -> String {
        let restore = self.state.lock().ok().is_some_and(|state| state.restore);
        if !restore {
            return requested.start_url().to_string();
        }
        let record = Self::restore_path(app)
            .ok()
            .and_then(|path| fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<RestoreRecord>(&bytes).ok());
        match record {
            Some(record) if record.site == requested => sanitize_school_url(&record.url)
                .unwrap_or_else(|| requested.start_url().to_string()),
            _ => requested.start_url().to_string(),
        }
    }

    pub fn open(
        &self,
        app: &AppHandle,
        site: SchoolSite,
        css: String,
        javascript: String,
        bounds: Option<WebBounds>,
        target_url: Option<String>,
    ) -> Result<SchoolWebResult, String> {
        self.open_internal(app, site, css, javascript, bounds, target_url, true)
    }

    /// Opens the profile's portal home in the hidden school view: a working
    /// session reports `active`; an expired one lands on the sign-in page,
    /// where saved credentials sign in again (the same hook that covers any
    /// school page redirected to sign-in). Used once at launch; afterwards
    /// sessions are renewed lazily when a page hits the sign-in redirect.
    /// Skipped while the person is using the school view or a sign-in runs.
    fn ensure_session_now(
        &self,
        app: &AppHandle,
        profile: SchoolAuthProfile,
    ) -> Result<bool, String> {
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "School web state is unavailable.")?;
            let busy = if state.active.is_some() {
                Some("school view is open")
            } else if state.login.is_some() {
                Some("a sign-in is running")
            } else if state.pulling.is_some() {
                Some("a school pull is running")
            } else if !state.creating.is_empty() {
                Some("a school view is being created")
            } else {
                None
            };
            if let Some(reason) = busy {
                eprintln!("[school-session] {profile:?} check deferred: {reason}");
                return Ok(false);
            }
            state.heartbeat = Some(profile);
        }
        let site = profile.start_site();
        self.open_internal(
            app,
            site,
            String::new(),
            String::new(),
            None,
            Some(site.start_url().to_string()),
            false,
        )
        .map(|_| true)
    }

    fn login_now(
        &self,
        app: &AppHandle,
        profile: SchoolAuthProfile,
        credentials: SchoolCredentials,
    ) -> Result<SchoolWebResult, String> {
        self.prepare_login(profile, credentials)?;
        let site = profile.start_site();
        self.open_internal(
            app,
            site,
            String::new(),
            String::new(),
            None,
            Some(site.start_url().to_string()),
            false,
        )
    }

    /// Sets (or clears, when empty) the 教務Web registration queue. Applies to
    /// the next 教務Web page load.
    pub fn set_guide(&self, items: Vec<crate::kyomu_guide::GuideItem>) -> Result<bool, String> {
        let items = crate::kyomu_guide::validate(items)?;
        self.state
            .lock()
            .map_err(|_| "School web state is unavailable.")?
            .guide = items;
        Ok(true)
    }

    /// Loads `url` in the site's resident view without showing it. Creates
    /// the view hidden when it does not exist yet.
    fn load_hidden(&self, app: &AppHandle, site: SchoolSite, url: &str) -> Result<(), String> {
        let (webview, active) = {
            let state = self
                .state
                .lock()
                .map_err(|_| "School web state is unavailable.")?;
            (state.webviews.get(&site).cloned(), state.active)
        };
        // Navigating the visible view would cancel whatever the person just
        // clicked (a TTL refresh can start while they browse).
        if active == Some(site) {
            return Err("Leave the Web page, then pull again.".into());
        }
        if let Some(webview) = webview {
            let parsed = Url::parse(url).map_err(|_| "School URL is invalid.")?;
            return webview
                .navigate(parsed)
                .map_err(|_| "School view could not navigate.".to_string());
        }
        if active.is_some() {
            return Err("Leave the Web page, then pull again.".into());
        }
        self.open_internal(
            app,
            site,
            String::new(),
            String::new(),
            None,
            Some(url.to_string()),
            false,
        )
        .map(|_| ())
    }

    fn expect_ingest(&self, site: SchoolSite) -> Result<mpsc::Receiver<IngestOutcome>, String> {
        let (sender, receiver) = mpsc::channel();
        let mut state = self
            .state
            .lock()
            .map_err(|_| "School web state is unavailable.")?;
        state.pending_ingestion.insert(site);
        state.ingest_waiters.entry(site).or_default().push(sender);
        Ok(receiver)
    }

    /// Runs the site's read-only extractor and waits for its result. Expired
    /// sessions are renewed by the usual saved-credential sign-in on the way.
    /// Blocking: call from a worker thread.
    fn pull_now(
        &self,
        app: &AppHandle,
        site: SchoolSite,
        lms_course_id: Option<u64>,
    ) -> Result<Vec<IngestOutcome>, String> {
        let wait = |receiver: mpsc::Receiver<IngestOutcome>, seconds: u64| {
            receiver
                .recv_timeout(Duration::from_secs(seconds))
                .map_err(|_| {
                    "The school page did not answer in time. Check the account in Settings."
                        .to_string()
                })
        };
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "School web state is unavailable.".to_string())?;
            if state.login.is_some() {
                return Err(
                    "Wait for the current school sign-in to finish, then pull again.".into(),
                );
            }
            if state.pulling.is_some() {
                return Err("Another school pull is already running.".into());
            }
            state.pulling = Some(site);
        }
        let result = (|| match site {
            SchoolSite::ScienceLms => {
                if lms_course_id == Some(0) {
                    return Err("LMS course ID must be positive.".into());
                }
                {
                    let mut state = self
                        .state
                        .lock()
                        .map_err(|_| "School web state is unavailable.".to_string())?;
                    state.pending_lms_course = lms_course_id;
                    state.pending_lms_full = lms_course_id.is_none();
                }
                let receiver = self.expect_ingest(site)?;
                self.load_hidden(app, site, "https://lms.s.isct.ac.jp/2025/my/")?;
                Ok(vec![wait(receiver, 150)?])
            }
            SchoolSite::Kyomu => {
                // Sub-pages bounce to the portal until Default.aspx opens the session.
                let (sender, loaded) = mpsc::channel();
                self.state
                    .lock()
                    .map_err(|_| "School web state is unavailable.")?
                    .load_waiters
                    .push((site, format!("{KYOMU_ROOT}Student/"), sender));
                self.load_hidden(app, site, &format!("{KYOMU_ROOT}Default.aspx"))?;
                loaded.recv_timeout(Duration::from_secs(150)).map_err(|_| {
                    "教務Web did not open. Check the Tokyo Tech account in Settings.".to_string()
                })?;
                let mut results = Vec::new();
                for page in KYOMU_PULL_PAGES {
                    let receiver = self.expect_ingest(site)?;
                    self.load_hidden(app, site, &format!("{KYOMU_ROOT}{page}"))?;
                    let result = wait(receiver, 60)?;
                    let stop = result.state == "reconnect-required";
                    results.push(result);
                    if stop {
                        break;
                    }
                }
                Ok(results)
            }
            _ => Err("Only LMS and 教務Web can be pulled.".into()),
        })();
        if let Ok(mut state) = self.state.lock() {
            if state.pulling == Some(site) {
                state.pulling = None;
            }
            if result.is_err() {
                state.pending_ingestion.remove(&site);
                state.ingest_waiters.remove(&site);
                state
                    .load_waiters
                    .retain(|(waiting_site, _, _)| *waiting_site != site);
                if site == SchoolSite::ScienceLms {
                    state.pending_lms_course = None;
                    state.pending_lms_full = false;
                }
            }
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn open_internal(
        &self,
        app: &AppHandle,
        site: SchoolSite,
        css: String,
        javascript: String,
        bounds: Option<WebBounds>,
        target_url: Option<String>,
        visible: bool,
    ) -> Result<SchoolWebResult, String> {
        if css.len() > 256 * 1024 || javascript.len() > 256 * 1024 {
            return Err("School customization exceeds the safe size.".into());
        }
        let requested_target = target_url.as_deref().and_then(sanitize_school_url);
        if target_url.is_some() && requested_target.is_none() {
            return Err("School URL is outside the allowed official origins.".into());
        }
        let restored = self.restored_target(app, site);
        let (existing, other_views, target, view_bounds, should_navigate, existing_relay) = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "School web state is unavailable.")?;
            if visible && state.active_job.is_some() {
                return Err("Wait for the school pull to finish before opening this page.".into());
            }
            if let Some(bounds) = bounds {
                state.bounds = bounds.sanitized();
            }
            let target = requested_target
                .clone()
                .or_else(|| state.urls.get(&site).cloned())
                .unwrap_or(restored);
            let should_navigate = requested_target.is_some();
            let other_views = if visible {
                state
                    .webviews
                    .iter()
                    .filter_map(|(key, view)| (*key != site).then_some(view.clone()))
                    .collect()
            } else {
                Vec::new()
            };
            state.active = visible.then_some(site);
            state.current_site = site;
            state.urls.insert(site, target.clone());
            state.customizations.insert(site, (css, javascript));
            if visible && matches!(site, SchoolSite::ScienceLms | SchoolSite::Kyomu) {
                state.pending_ingestion.insert(site);
            }
            if visible && site == SchoolSite::ScienceLms {
                state.pending_lms_course = None;
                state.pending_lms_full = false;
            }
            (
                state
                    .webviews
                    .get(&site)
                    .cloned()
                    .or_else(|| app.get_webview(site.webview_label())),
                other_views,
                target,
                state.bounds,
                should_navigate,
                state.ingestion_relays.get(&site).cloned(),
            )
        };

        if let Some(webview) = existing {
            if visible {
                for other in other_views {
                    let _ = other.hide();
                }
                apply_bounds(&webview, view_bounds)?;
            }
            if should_navigate {
                let parsed = Url::parse(&target).map_err(|_| "School URL is invalid.")?;
                webview
                    .navigate(parsed)
                    .map_err(|_| "School view could not navigate.".to_string())?;
            }
            if visible {
                webview
                    .show()
                    .map_err(|_| "School view could not be shown.".to_string())?;
                let _ = webview.set_focus();
                if !should_navigate {
                    if let (Some(script), Some(relay)) =
                        (ingestion_script(&target), existing_relay.as_deref())
                    {
                        if let Ok(mut state) = self.state.lock() {
                            state.pending_ingestion.remove(&site);
                        }
                        let extraction = format!("delete globalThis.__ISCT_COMPANION_LMS_COURSE_ID__;globalThis.__ISCT_COMPANION_LMS_FULL__=false;Promise.resolve({script}).then(v=>location.href='{relay}'+btoa(unescape(encodeURIComponent(JSON.stringify(v)))).replace(/\\+/g,'-').replace(/\\//g,'_').replace(/=+$/,''))");
                        let _ = webview.eval(&extraction);
                    }
                }
            } else {
                webview
                    .hide()
                    .map_err(|_| "School view could not be hidden.".to_string())?;
            }
            if self.state.lock().ok().is_some_and(|state| state.restore) {
                Self::write_restore(app, site, &target)?;
            }
            return Ok(SchoolWebResult {
                site,
                url: target,
                phase: "pageLoaded",
            });
        }

        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "School web state is unavailable.")?;
            if state.creating.contains(&site) {
                return Ok(SchoolWebResult {
                    site,
                    url: target,
                    phase: "sessionRestored",
                });
            }
            state.creating.insert(site);
        }

        let nonce = {
            let mut bytes = [0u8; 24];
            rand::rng().fill_bytes(&mut bytes);
            URL_SAFE_NO_PAD.encode(bytes)
        };
        let relay_prefix = format!("isct-companion-extract://{nonce}/");
        if let Ok(mut state) = self.state.lock() {
            state.ingestion_relays.insert(site, relay_prefix.clone());
        }
        // Login reports arrive through the page title: unlike a relay
        // navigation, a title change can never cancel the page's own form
        // POST or redirect.
        let auth_relay_prefix = format!("isct-companion-auth:{nonce}:");
        let progress_relay_prefix = format!("isct-companion-progress:{nonce}:");
        let debug_relay_prefix = format!("isct-companion-login-debug://{nonce}/");
        let relay_for_navigation = relay_prefix.clone();
        let relay_for_load = relay_prefix;
        let auth_relay_for_title = auth_relay_prefix.clone();
        let auth_relay_for_load = auth_relay_prefix;
        let progress_relay_for_title = progress_relay_prefix.clone();
        let progress_relay_for_load = progress_relay_prefix;
        let debug_relay_for_navigation = debug_relay_prefix.clone();
        let debug_relay_for_load = debug_relay_prefix;
        let state_for_title = self.state.clone();
        let app_for_title = app.clone();
        let state_for_load = self.state.clone();
        let app_for_load = app.clone();
        let app_for_navigation = app.clone();
        let state_for_navigation = self.state.clone();
        let app_for_new_window = app.clone();
        let state_for_new_window = self.state.clone();
        let app_for_download = app.clone();
        let state_for_download = self.state.clone();
        let main = app
            .get_window(MAIN_WINDOW)
            .ok_or_else(|| "Main window is unavailable.".to_string())?;
        let start = Url::parse(&target).map_err(|_| "School URL is invalid.")?;
        let builder = WebviewBuilder::new(site.webview_label(), WebviewUrl::External(start));
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        let builder = builder
            .data_store_identifier(SCHOOL_DATA_STORE)
            .background_throttling(BackgroundThrottlingPolicy::Disabled);
        #[cfg(target_os = "windows")]
        let builder = builder.data_directory(
            app.path()
                .app_data_dir()
                .map_err(|_| "School web data path is unavailable.")?
                .join("shared-webview-data"),
        );
        #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "windows")))]
        let builder = builder;
        let builder = builder
            // Tauri's handler claims OS file drops, so the page never sees
            // them; school pages need the drop for assignment uploads.
            .disable_drag_drop_handler()
            .initialization_script_for_all_frames(KYOMU_POPUP_SCRIPT)
            .initialization_script_for_all_frames(SCHOOL_LINKS_SCRIPT)
            .on_page_load(move |webview, payload| {
                // Started fires in the outgoing document; only act once the
                // new page exists, or every script would run twice.
                if payload.event() != PageLoadEvent::Finished {
                    return;
                }
                let raw_url = payload.url().as_str();
                if let Ok(mut state) = state_for_load.lock() {
                    state.loaded_urls.insert(site, raw_url.to_string());
                }
                if let Ok(url) = Url::parse(raw_url) {
                    eprintln!(
                        "[school-web] {} loaded {}{}",
                        webview.label(),
                        url.host_str().unwrap_or_default(),
                        url.path()
                    );
                }
                let auth_profile = Url::parse(raw_url)
                    .ok()
                    .filter(|url| url.scheme() == "https")
                    .and_then(|url| url.host_str().and_then(SchoolAuthProfile::for_auth_host));
                // Landing on a sign-in page with saved credentials starts an
                // automatic sign-in, unless that profile's last attempt failed.
                let should_arm = auth_profile.is_some_and(|profile| {
                    state_for_load.lock().ok().is_some_and(|state| {
                        state.login.is_none() && !state.auto_blocked.contains(&profile)
                    })
                });
                let armed = if should_arm {
                    auth_profile.and_then(|profile| {
                        let service = app_for_load
                            .try_state::<SchoolAuthService<OsSecretVault>>()?;
                        let credentials = service.load(profile).ok().flatten()?;
                        Some(ActiveLogin {
                            profile,
                            credentials,
                            run: new_run_id(),
                        })
                    })
                } else {
                    None
                };
                let (site, css, javascript, restore, login, completed, heartbeat) = match state_for_load.lock() {
                    Ok(mut state) => {
                        if let Some(safe_url) = sanitize_school_url(raw_url) {
                            state.urls.insert(site, safe_url);
                        }
                        if state.login.is_none() {
                            state.login = armed;
                        }
                        let login = state.login.clone();
                        let (login, completed) = match login {
                            Some(active) if login_succeeded(raw_url, active.profile) => {
                                state.login = None;
                                state.auto_blocked.remove(&active.profile);
                                (None, Some(active.profile))
                            }
                            login => (login, None),
                        };
                        // Only the profile's own view can answer its check.
                        let heartbeat = state.heartbeat.filter(|profile| profile.start_site() == site).and_then(|profile| {
                            let signing_in = state
                                .login
                                .as_ref()
                                .is_some_and(|active| active.profile == profile);
                            let outcome = session_outcome(raw_url, profile, signing_in)?;
                            state.heartbeat = None;
                            Some((profile, outcome))
                        });
                        (
                            site,
                            state
                                .customizations
                                .get(&site)
                                .cloned()
                                .unwrap_or_default()
                                .0,
                            state
                                .customizations
                                .get(&site)
                                .cloned()
                                .unwrap_or_default()
                                .1,
                            state.restore,
                            login,
                            completed,
                            heartbeat,
                        )
                    }
                    Err(_) => return,
                };
                if restore {
                    let _ = Self::write_restore(&app_for_load, site, raw_url);
                }
                if let Some(safe_url) = sanitize_school_url(raw_url) {
                    let _ = app_for_load.emit_to(
                        MAIN_WINDOW,
                        "school-web-url",
                        serde_json::json!({ "site": site, "url": safe_url }),
                    );
                }
                if let Some(profile) = completed {
                    emit_login_status(&app_for_load, profile, "connected", "Connected");
                }
                let settled_profile = match (heartbeat, completed) {
                    (Some((profile, outcome)), _) => {
                        emit_session(&state_for_load, &app_for_load, profile, outcome);
                        Some(profile)
                    }
                    (None, Some(profile)) => {
                        emit_session(&state_for_load, &app_for_load, profile, "active");
                        Some(profile)
                    }
                    _ => None,
                };
                if let Some(profile) = settled_profile {
                    SchoolWebController {
                        state: state_for_load.clone(),
                    }
                    .complete_session_job(&app_for_load, profile);
                }
                if let Some(active) = login {
                    // Page capture is opt-in: ISCT_COMPANION_LOGIN_DEBUG=1 yarn desktop:dev
                    let debug_relay = (cfg!(debug_assertions)
                        && std::env::var_os("ISCT_COMPANION_LOGIN_DEBUG").is_some())
                    .then_some(debug_relay_for_load.as_str());
                    if let Some(script) =
                        login_script(raw_url, &active, &auth_relay_for_load, debug_relay)
                    {
                        let _ = webview.eval(&script);
                    }
                }
                let close_override = state_for_load
                    .lock()
                    .ok()
                    .and_then(|mut state| track_page(&mut state, site, raw_url));
                if let Some(script) = close_override {
                    let _ = webview.eval(&script);
                }
                if !css.trim().is_empty() {
                    if let Ok(css_json) = serde_json::to_string(&css) {
                        let _ = webview.eval(&format!("(()=>{{const s=document.createElement('style');s.textContent={css_json};document.documentElement.appendChild(s)}})()"));
                    }
                }
                if !javascript.trim().is_empty() {
                    let _ = webview.eval(&javascript);
                }
                let guide = state_for_load
                    .lock()
                    .ok()
                    .and_then(|state| crate::kyomu_guide::script(&state.guide));
                if let Some(guide) = guide.filter(|_| site_for_url(raw_url) == Some(SchoolSite::Kyomu)) {
                    let _ = webview.eval(&guide);
                }
                if let Ok(mut state) = state_for_load.lock() {
                    let waiters = std::mem::take(&mut state.load_waiters);
                    for (waiting_site, prefix, sender) in waiters {
                        if waiting_site == site && raw_url.starts_with(&prefix) {
                            let _ = sender.send(raw_url.to_string());
                        } else {
                            state.load_waiters.push((waiting_site, prefix, sender));
                        }
                    }
                }
                let (should_ingest, lms_course_id, lms_full) = state_for_load
                    .lock()
                    .ok()
                    .map(|mut state| {
                        let should = ingestion_script(raw_url).is_some()
                            && state.pending_ingestion.remove(&site);
                        let course = (should && site == SchoolSite::ScienceLms)
                            .then(|| state.pending_lms_course.take())
                            .flatten();
                        let full = should
                            && site == SchoolSite::ScienceLms
                            && std::mem::take(&mut state.pending_lms_full);
                        (should, course, full)
                    })
                    .unwrap_or((false, None, false));
                if let Some(script) = should_ingest.then(|| ingestion_script(raw_url)).flatten() {
                    let course_config = lms_course_id
                        .map(|id| format!("globalThis.__ISCT_COMPANION_LMS_COURSE_ID__={id};"))
                        .unwrap_or_else(|| "delete globalThis.__ISCT_COMPANION_LMS_COURSE_ID__;".into());
                    let extraction = format!("{course_config}globalThis.__ISCT_COMPANION_LMS_FULL__={lms_full};globalThis.__ISCT_COMPANION_LMS_PROGRESS__=v=>document.title='{progress_relay_for_load}'+btoa(unescape(encodeURIComponent(JSON.stringify(v)))).replace(/\\+/g,'-').replace(/\\//g,'_').replace(/=+$/,'');Promise.resolve({script}).then(v=>location.href='{}'+btoa(unescape(encodeURIComponent(JSON.stringify(v)))).replace(/\\+/g,'-').replace(/\\//g,'_').replace(/=+$/,''))", relay_for_load);
                    let _ = webview.eval(&extraction);
                }
            })
            .on_document_title_changed(move |_webview, title| {
                if let Some(encoded) = title.strip_prefix(&auth_relay_for_title) {
                    handle_login_relay(&state_for_title, &app_for_title, encoded);
                } else if let Some(encoded) = title.strip_prefix(&progress_relay_for_title) {
                    handle_pull_progress(&state_for_title, &app_for_title, encoded);
                }
            })
            .on_navigation(move |url| {
                if let Some(encoded) = url.as_str().strip_prefix(&debug_relay_for_navigation) {
                    if cfg!(debug_assertions) && encoded.len() <= 4 * 1024 * 1024 {
                        if let Ok(bytes) = URL_SAFE_NO_PAD.decode(encoded) {
                            if let Ok(capture) = serde_json::from_slice::<LoginCaptureRelay>(&bytes) {
                                if sanitize_school_url(&capture.url).is_some()
                                    && capture.html.len() <= 2 * 1024 * 1024
                                {
                                    let directory = std::env::temp_dir().join("isct-companion-login-debug");
                                    if fs::create_dir_all(&directory).is_ok() {
                                        let timestamp = SystemTime::now()
                                            .duration_since(UNIX_EPOCH)
                                            .map(|duration| duration.as_millis())
                                            .unwrap_or_default();
                                        let page = Url::parse(&capture.url)
                                            .ok()
                                            .map(|url| {
                                                format!(
                                                    "{}{}",
                                                    url.host_str().unwrap_or("school"),
                                                    url.path()
                                                )
                                            })
                                            .unwrap_or_else(|| "school".into())
                                            .chars()
                                            .map(|character| {
                                                if character.is_ascii_alphanumeric() {
                                                    character
                                                } else {
                                                    '-'
                                                }
                                            })
                                            .collect::<String>();
                                        let path = directory.join(format!("{timestamp}-{page}.html"));
                                        let _ = fs::write(path, capture.html);
                                    }
                                }
                            }
                        }
                    }
                    return false;
                }
                if url.as_str().starts_with(&relay_for_navigation) {
                    if let Some(encoded) = url.as_str().strip_prefix(&relay_for_navigation) {
                        if encoded.len() <= 11 * 1024 * 1024 {
                            if let Ok(bytes) = URL_SAFE_NO_PAD.decode(encoded) {
                                if let Ok(raw) = String::from_utf8(bytes) {
                                    let store = app_for_navigation.try_state::<crate::data::DataStore>();
                                    let apply = |apply: &dyn Fn(&mut rusqlite::Connection) -> Result<usize, String>| {
                                        store
                                            .as_ref()
                                            .ok_or_else(|| "Data store is unavailable.".to_string())
                                            .and_then(|store| {
                                                rusqlite::Connection::open(store.path())
                                                    .map_err(|error| error.to_string())
                                            })
                                            .and_then(|mut connection| apply(&mut connection))
                                    };
                                    let from_lms = serde_json::from_str::<serde_json::Value>(&raw)
                                        .ok()
                                        .and_then(|value| value.get("sourceUrl")?.as_str().map(str::to_string))
                                        .is_some_and(|source| site_for_url(&source) == Some(SchoolSite::ScienceLms));
                                    let outcome = if from_lms {
                                        match parse_lms_payload(&raw) {
                                            Ok((LmsStage::Ready, Some(payload))) => {
                                                outcome(site, "lms", apply(&|c| apply_lms_payload(c, &payload)))
                                            }
                                            Ok(_) => outcome(site, "lms", Err("reconnect-required".into())),
                                            Err(error) => {
                                                // The extractor's own reason (e.g. a Moodle errorcode) is the useful part.
                                                let reason = serde_json::from_str::<serde_json::Value>(&raw)
                                                    .ok()
                                                    .and_then(|value| value.get("error")?.as_str().map(str::to_string))
                                                    .map(|reason| format!("{error}: {reason}"))
                                                    .unwrap_or(error);
                                                outcome(site, "lms", Err(reason))
                                            }
                                        }
                                    } else if let Ok((stage, payload)) = parse_lms_payload(&raw) {
                                        match (stage, payload) {
                                            (LmsStage::Ready, Some(payload)) => {
                                                outcome(site, "lms", apply(&|c| apply_lms_payload(c, &payload)))
                                            }
                                            _ => outcome(site, "lms", Err("reconnect-required".into())),
                                        }
                                    } else if let Ok((stage, payload)) = parse_kyomu_payload(&raw) {
                                        let kind = payload.as_ref().map(|p| p.kind.clone()).unwrap_or_default();
                                        match (stage, payload) {
                                            (KyomuStage::Ready, Some(payload)) => {
                                                outcome(site, &kind, apply(&|c| apply_kyomu_payload(c, &payload)))
                                            }
                                            _ => outcome(site, &kind, Err("reconnect-required".into())),
                                        }
                                    } else {
                                        outcome(site, "unknown", Err("The page returned an unexpected result.".into()))
                                    };
                                    eprintln!(
                                        "[school-ingest] {:?} {} {} {}",
                                        outcome.site, outcome.kind, outcome.state, outcome.message
                                    );
                                    let waiters = state_for_navigation
                                        .lock()
                                        .ok()
                                        .and_then(|mut state| state.ingest_waiters.remove(&site))
                                        .unwrap_or_default();
                                    for waiter in waiters {
                                        let _ = waiter.send(outcome.clone());
                                    }
                                    let _ = app_for_navigation.emit_to(MAIN_WINDOW, "school-ingest", outcome);
                                }
                            }
                        }
                    }
                    return false;
                }
                let allowed = sanitize_school_url(url.as_str()).is_some();
                if !allowed && cfg!(debug_assertions) {
                    eprintln!(
                        "[school-web] blocked {}://{}{}",
                        url.scheme(),
                        url.host_str().unwrap_or_default(),
                        url.path()
                    );
                }
                allowed
            })
            // Files WebKit cannot show (docx, zip, `download` links). Runs on
            // the main thread: bookkeeping and events only.
            .on_download(move |_webview, event| match event {
                DownloadEvent::Requested { url, destination } => {
                    let download_dir = app_for_download.path().download_dir().ok();
                    let accepted = download_dir
                        .is_some_and(|directory| download_destination_ok(destination, &directory))
                        && state_for_download.lock().ok().is_some_and(|mut state| {
                            if !download_allowed(url.as_str(), site, state.active) {
                                return false;
                            }
                            state.downloads.push((url.to_string(), destination.clone()));
                            true
                        });
                    if !accepted {
                        eprintln!("[school-download] {site:?} download refused");
                        return false;
                    }
                    let name = destination
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    emit_download(
                        &app_for_download,
                        site,
                        "started",
                        name,
                        sanitize_school_url(url.as_str()),
                        None,
                    );
                    true
                }
                DownloadEvent::Finished { url, success, .. } => {
                    let destination = state_for_download.lock().ok().and_then(|mut state| {
                        let index = state
                            .downloads
                            .iter()
                            .position(|(pending, _)| pending == url.as_str())?;
                        Some(state.downloads.remove(index).1)
                    });
                    // Refused downloads also finish (unsuccessfully); they were never announced.
                    if let Some(destination) = destination {
                        let name = destination
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let saved = success.then_some(destination);
                        finish_download(&app_for_download, &state_for_download, site, name, saved);
                    }
                    true
                }
                _ => true,
            })
            .on_new_window(move |url, _features| {
                let raw = url.as_str();
                match new_window_target(raw) {
                    // Same service: open in place, like a same-window link.
                    // Deferred so the load starts after WebKit's delegate call returns.
                    NewWindowTarget::School(target) if target == site => {
                        // Remember the opener so the popup's 閉じる can return to it.
                        if let (Ok(mut state), Some(popup)) =
                            (state_for_new_window.lock(), sanitize_school_url(raw))
                        {
                            match state.urls.get(&site).cloned() {
                                Some(opener) if !same_page(&opener, &popup) => {
                                    state.popups.insert(site, PopupReturn { opener, popup });
                                }
                                _ => {
                                    state.popups.remove(&site);
                                }
                            }
                        }
                        if let Some(view) = app_for_new_window.get_webview(site.webview_label()) {
                            std::thread::spawn(move || {
                                let _ = view.navigate(url);
                            });
                        }
                    }
                    // Another service: the Web page switches to it, as with the address bar.
                    NewWindowTarget::School(target) => {
                        if let Some(url) = sanitize_school_url(raw) {
                            let _ = app_for_new_window.emit_to(
                                MAIN_WINDOW,
                                "school-web-open",
                                SchoolWebOpen { site: target, url },
                            );
                        }
                    }
                    NewWindowTarget::External(url) => {
                        let _ = app_for_new_window.opener().open_url(url, None::<&str>);
                    }
                    NewWindowTarget::Ignored => {
                        if cfg!(debug_assertions) {
                            eprintln!("[school-web] ignored new window {}:", url.scheme());
                        }
                    }
                }
                NewWindowResponse::Deny
            });

        let initial_position = if visible {
            LogicalPosition::new(view_bounds.x, view_bounds.y)
        } else {
            LogicalPosition::new(0.0, 0.0)
        };
        let initial_size = if visible {
            LogicalSize::new(view_bounds.width, view_bounds.height)
        } else {
            LogicalSize::new(1.0, 1.0)
        };
        let webview = match main.add_child(builder, initial_position, initial_size) {
            Ok(webview) => webview,
            Err(error) => {
                if let Ok(mut state) = self.state.lock() {
                    state.creating.remove(&site);
                }
                return Err(format!("School view could not be created: {error}"));
            }
        };

        #[cfg(target_os = "macos")]
        {
            let app_for_save = app.clone();
            let state_for_save = self.state.clone();
            let installed = webview.with_webview(move |platform| {
                let state_for_callback = state_for_save.clone();
                let on_save = Box::new(move |saved: pdf_save::SavedData| {
                    save_viewer_data(&app_for_save, &state_for_callback, site, saved);
                });
                // SAFETY: `inner()` is this view's live WKWebView, and
                // `with_webview` runs the closure on the main thread.
                if let Some(handle) = unsafe { pdf_save::install(platform.inner(), on_save) } {
                    if let Ok(mut state) = state_for_save.lock() {
                        state.ui_delegates.insert(site, handle);
                    }
                }
            });
            if let Err(error) = installed {
                eprintln!("[school-download] {site:?} PDF save hook unavailable: {error}");
            }
        }

        let (is_active, bounds) = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "School web state is unavailable.")?;
            state.creating.remove(&site);
            state.webviews.insert(site, webview.clone());
            (state.active == Some(site), state.bounds)
        };
        if is_active {
            for other in other_views {
                let _ = other.hide();
            }
            apply_bounds(&webview, bounds)?;
            webview
                .show()
                .map_err(|_| "School view could not be shown.".to_string())?;
            let _ = webview.set_focus();
        } else {
            webview
                .hide()
                .map_err(|error| format!("School view could not be hidden: {error}"))?;
        }
        if self.state.lock().ok().is_some_and(|state| state.restore) {
            Self::write_restore(app, site, &target)?;
        }
        Ok(SchoolWebResult {
            site,
            url: target,
            phase: "sessionRestored",
        })
    }
}
