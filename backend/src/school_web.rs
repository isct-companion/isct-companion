use crate::lifecycle::SchoolSite;
use crate::school_auth::{
    upcoming_otps, OtpWindow, SchoolAuthProfile, SchoolAuthService, SchoolCredentials,
};
use crate::school_extract::{
    merge_school_payload, publish_school_cache, sanitize_school_url, stage_school_payload,
    ExtractedPayload, T2SCHOLA_EXTRACT_SCRIPT,
};
use crate::secrets::OsSecretVault;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::utils::config::BackgroundThrottlingPolicy;
use tauri::webview::{PageLoadEvent, WebviewBuilder};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Webview, WebviewUrl};
use url::Url;

const MAIN_WINDOW: &str = "main";
// Stable UUID shared by every embedded school view. On Apple platforms this
// selects one WKWebsiteDataStore, so cookies/session storage survive view
// switches and are shared by the resident pool.
const SCHOOL_DATA_STORE: [u8; 16] = [
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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolLoginEvent {
    pub profile: SchoolAuthProfile,
    pub state: String,
    pub message: String,
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

fn emit_session(app: &AppHandle, profile: SchoolAuthProfile, state: &'static str) {
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
    restore: bool,
    bounds: WebBounds,
    customizations: HashMap<SchoolSite, (String, String)>,
    login: Option<ActiveLogin>,
    /// Profiles whose last attempt failed; saved credentials are not replayed
    /// automatically until the person starts a new sign-in.
    auto_blocked: HashSet<SchoolAuthProfile>,
    /// A launch-time session check waiting for its portal page to load.
    heartbeat: Option<SchoolAuthProfile>,
    creating: HashSet<SchoolSite>,
}

impl Default for ControllerState {
    fn default() -> Self {
        Self {
            webviews: HashMap::new(),
            active: None,
            current_site: SchoolSite::SciencePortal,
            urls: HashMap::new(),
            restore: false,
            bounds: WebBounds::default(),
            customizations: HashMap::new(),
            login: None,
            auto_blocked: HashSet::new(),
            heartbeat: None,
            creating: HashSet::new(),
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
    match Url::parse(raw).ok()?.host_str()? {
        "lms.s.isct.ac.jp" | "t2schola.titech.ac.jp" => Some(SchoolSite::ScienceLms),
        "syllabus.s.isct.ac.jp" => Some(SchoolSite::Syllabus),
        "isct.ex-tic.com" | "portal.isct.ac.jp" => Some(SchoolSite::SciencePortal),
        "portal.titech.ac.jp" | "portal.nap.gsic.titech.ac.jp" | "rp.nap.gsic.titech.ac.jp" => {
            Some(SchoolSite::TokyoPortal)
        }
        "mailv4.m.titech.ac.jp" => Some(SchoolSite::TokyoMail),
        "kyomu0.gakumu.titech.ac.jp" => Some(SchoolSite::Kyomu),
        "request.libra.titech.ac.jp" => Some(SchoolSite::Library),
        _ => None,
    }
}

fn is_lms_url(raw: &str) -> bool {
    matches!(site_for_url(raw), Some(SchoolSite::ScienceLms))
}

const SCHOOL_LOGIN_SCRIPT: &str = include_str!("../scripts/school-login.js");

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
    Some(SCHOOL_LOGIN_SCRIPT.replace("__STWS_LOGIN_CONFIG__", &config))
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
    let Some(outcome) = relay_outcome(&relay.state, &relay.message) else {
        return;
    };
    let mut failed = false;
    let matches_active = state.lock().ok().is_some_and(|mut state| {
        let matches = state
            .login
            .as_ref()
            .is_some_and(|active| active.profile == relay.profile && active.run == relay.run);
        if matches && outcome == RelayOutcome::Failed {
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
        emit_session(app, relay.profile, "failed");
    }
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            "stws-auth://test/",
            None,
        )
        .is_some());
        for url in [
            "https://portal.nap.gsic.titech.ac.jp/GetAccess/Login",
            "http://isct.ex-tic.com/auth/session",
            "https://lms.s.isct.ac.jp/2025/",
        ] {
            assert!(login_script(url, &science, "stws-auth://test/", None).is_none());
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
            "stws-auth://test/",
            None,
        )
        .unwrap();
        assert!(!script.contains("__STWS_LOGIN_CONFIG__"));
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
        let _ = app;
        Ok(true)
    }

    pub fn reload(&self, app: &AppHandle, site: SchoolSite) -> Result<bool, String> {
        let webview = {
            let state = self
                .state
                .lock()
                .map_err(|_| "School web state is unavailable.")?;
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
    pub fn ensure_session(
        &self,
        app: &AppHandle,
        profile: SchoolAuthProfile,
    ) -> Result<bool, String> {
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "School web state is unavailable.")?;
            if state.active.is_some() || state.login.is_some() || !state.creating.is_empty() {
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

    pub fn login_in_background(
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
        let (existing, other_views, target, view_bounds, should_navigate) = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "School web state is unavailable.")?;
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
        let cache_path = app
            .path()
            .app_cache_dir()
            .map_err(|_| "School cache path is unavailable.")?
            .join("school-cache.json");
        let relay_prefix = format!("stws-extract://{nonce}/");
        // Login reports arrive through the page title: unlike a relay
        // navigation, a title change can never cancel the page's own form
        // POST or redirect.
        let auth_relay_prefix = format!("stws-auth:{nonce}:");
        let debug_relay_prefix = format!("stws-login-debug://{nonce}/");
        let relay_for_navigation = relay_prefix.clone();
        let relay_for_load = relay_prefix;
        let auth_relay_for_title = auth_relay_prefix.clone();
        let auth_relay_for_load = auth_relay_prefix;
        let debug_relay_for_navigation = debug_relay_prefix.clone();
        let debug_relay_for_load = debug_relay_prefix;
        let state_for_title = self.state.clone();
        let app_for_title = app.clone();
        let state_for_load = self.state.clone();
        let app_for_load = app.clone();
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
            .on_page_load(move |webview, payload| {
                // Started fires in the outgoing document; only act once the
                // new page exists, or every script would run twice.
                if payload.event() != PageLoadEvent::Finished {
                    return;
                }
                let raw_url = payload.url().as_str();
                if let Ok(url) = Url::parse(raw_url) {
                    eprintln!(
                        "[school-web] loaded {}{}",
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
                        let heartbeat = state.heartbeat.and_then(|profile| {
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
                    let _ = app_for_load.emit_to(MAIN_WINDOW, "school-web-url", safe_url);
                }
                if let Some(profile) = completed {
                    emit_login_status(&app_for_load, profile, "connected", "Connected");
                }
                match (heartbeat, completed) {
                    (Some((profile, outcome)), _) => emit_session(&app_for_load, profile, outcome),
                    (None, Some(profile)) => emit_session(&app_for_load, profile, "active"),
                    _ => {}
                }
                if let Some(active) = login {
                    // Page capture is opt-in: STWS_LOGIN_DEBUG=1 yarn desktop:dev
                    let debug_relay = (cfg!(debug_assertions)
                        && std::env::var_os("STWS_LOGIN_DEBUG").is_some())
                    .then_some(debug_relay_for_load.as_str());
                    if let Some(script) =
                        login_script(raw_url, &active, &auth_relay_for_load, debug_relay)
                    {
                        let _ = webview.eval(&script);
                    }
                }
                if !css.trim().is_empty() {
                    if let Ok(css_json) = serde_json::to_string(&css) {
                        let _ = webview.eval(&format!("(()=>{{const s=document.createElement('style');s.textContent={css_json};document.documentElement.appendChild(s)}})()"));
                    }
                }
                if !javascript.trim().is_empty() {
                    let _ = webview.eval(&javascript);
                }
                if is_lms_url(raw_url) {
                    let extraction = format!("Promise.resolve({T2SCHOLA_EXTRACT_SCRIPT}).then(v=>location.href='{}'+btoa(unescape(encodeURIComponent(JSON.stringify(v)))).replace(/\\+/g,'-').replace(/\\//g,'_').replace(/=+$/,''))", relay_for_load);
                    let _ = webview.eval(&extraction);
                }
            })
            .on_document_title_changed(move |_webview, title| {
                if let Some(encoded) = title.strip_prefix(&auth_relay_for_title) {
                    handle_login_relay(&state_for_title, &app_for_title, encoded);
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
                                    let directory = std::env::temp_dir().join("campus-life-login-debug");
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
                                    let cached = fs::read(&cache_path).ok().and_then(|bytes| {
                                        serde_json::from_slice::<ExtractedPayload>(&bytes).ok()
                                    });
                                    let staged = stage_school_payload(&raw, cached.clone());
                                    if let Some(payload) = staged.payload {
                                        if staged.state == crate::school_extract::StageState::Fresh {
                                            let merged = merge_school_payload(cached, payload);
                                            let _ = publish_school_cache(&cache_path, &merged);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    return false;
                }
                sanitize_school_url(url.as_str()).is_some()
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
