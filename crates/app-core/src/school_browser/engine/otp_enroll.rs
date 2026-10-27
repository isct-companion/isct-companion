use super::*;
use crate::school_auth::OtpSetupError;

const REGISTER: &str = "https://isct.ex-tic.com/idm/user/totp_setting/add";
const SCRIPT: &str = adpt_isct::scripts::OTP_ENROLLMENT;

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Inspect,
    Removing,
    Reading,
    Submitting,
}

pub(super) struct Enrollment {
    operation: u64,
    reset: bool,
    phase: Phase,
    page: Option<PageId>,
    token: String,
    reply: Option<Reply>,
    owner: String,
    candidate: String,
}

#[derive(Deserialize)]
struct Reply {
    operation: u64,
    sequence: u64,
    token: String,
    kind: String,
    #[serde(default)]
    owner: String,
    #[serde(default)]
    secret: String,
    #[serde(default)]
    registered: bool,
}

impl Inner {
    fn enrollment_script(
        &self,
        page: PageId,
        action: &str,
        extra: serde_json::Value,
    ) -> Result<(), OtpSetupError> {
        let config = {
            let mut state = self.state.guard();
            let relay = state
                .views
                .get(&page.browser.site)
                .filter(|view| view.id == page.browser && view.last_page == page.sequence)
                .map(|view| format!("{}otp:", view.auth_relay))
                .ok_or(OtpSetupError::Cancelled)?;
            let enrollment = state
                .otp_enrollment
                .as_mut()
                .ok_or(OtpSetupError::Cancelled)?;
            enrollment.page = Some(page);
            enrollment.token = new_id();
            let mut config = serde_json::json!({"operation":enrollment.operation,"sequence":page.sequence,"token":enrollment.token,"relay":relay,"action":action});
            if let (Some(config), Some(extra)) = (config.as_object_mut(), extra.as_object()) {
                config.extend(extra.clone());
            }
            config
        };
        let config = serde_json::to_string(&config).map_err(|_| OtpSetupError::Unavailable)?;
        self.dispatch_eval(
            page.browser,
            SCRIPT.replace("__OTP_ENROLLMENT_CONFIG__", &config),
            true,
        )
        .map_err(|_| OtpSetupError::Unavailable)
    }

    pub(super) fn otp_enrollment_page(&self, event: &PageLoaded) {
        let eligible = {
            let state = self.state.guard();
            state.browsing == Some(SchoolSite::SciencePortal)
                && state
                    .otp_enrollment
                    .as_ref()
                    .is_some_and(|enrollment| enrollment.page != Some(event.page))
                && matches!(
                    event.url.path().trim_end_matches('/'),
                    "/idm/user/mfa_setting" | "/idm/user/totp_setting/add"
                )
                && event.url.host_str() == Some("isct.ex-tic.com")
        };
        if eligible {
            let _ = self.enrollment_script(event.page, "inspect", serde_json::json!({}));
        }
    }

    pub(super) fn otp_enrollment_relay(&self, page: PageId, encoded: &str) {
        if encoded.len() > 8192 {
            return;
        }
        let Some(reply) = URL_SAFE_NO_PAD
            .decode(encoded)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Reply>(&bytes).ok())
        else {
            return;
        };
        let mut state = self.state.guard();
        if state.browsing != Some(SchoolSite::SciencePortal)
            || !state
                .views
                .get(&page.browser.site)
                .is_some_and(|view| view.id == page.browser && view.last_page == page.sequence)
        {
            return;
        }
        let Some(enrollment) = state.otp_enrollment.as_mut() else {
            return;
        };
        if enrollment.operation != reply.operation
            || enrollment.page != Some(page)
            || reply.sequence != page.sequence
            || enrollment.token != reply.token
            || reply.owner.len() > 128
            || reply.secret.len() > 1024
        {
            return;
        }
        // A one-use token rejects duplicate and stale callbacks even on the
        // same native page. Re-injection issues a fresh token.
        enrollment.token.clear();
        enrollment.reply = Some(reply);
        drop(state);
        let _ = self.traces.send(Trace::Enrollment);
    }
}

impl SchoolBrowserEngine {
    /// Completes the visible school's forms, then promotes the staged vault
    /// candidate only after the school reports App Authentication registered.
    pub async fn complete_school_otp_setup(
        &self,
        id: u64,
        reset: bool,
    ) -> Result<(), OtpSetupError> {
        let _ownership = super::otp_setup::WaitGuard(self, id);
        let mut traces = self.0.traces.subscribe();
        {
            let mut state = self.0.state.guard();
            if state.otp_setup != Some(id) {
                return Err(OtpSetupError::Cancelled);
            }
            state.otp_enrollment = Some(Enrollment {
                operation: id,
                reset,
                phase: Phase::Inspect,
                page: None,
                token: String::new(),
                reply: None,
                owner: String::new(),
                candidate: new_id(),
            });
        }
        let work = async {
            loop {
                // Recover a page that loaded before this waiter attached.
                let current = {
                    let state = self.0.state.guard();
                    if state.otp_setup != Some(id) {
                        return Err(OtpSetupError::Cancelled);
                    }
                    state
                        .views
                        .get(&SchoolSite::SciencePortal)
                        .filter(|view| view.last_page > 0)
                        .and_then(|view| {
                            Url::parse(&view.url).ok().map(|url| PageLoaded {
                                page: PageId {
                                    browser: view.id,
                                    sequence: view.last_page,
                                },
                                cause: None,
                                url,
                            })
                        })
                };
                if let Some(page) = current {
                    self.0.otp_enrollment_page(&page);
                }
                let message = {
                    let mut state = self.0.state.guard();
                    let enrollment = state
                        .otp_enrollment
                        .as_mut()
                        .ok_or(OtpSetupError::Cancelled)?;
                    enrollment.reply.take().map(|reply| {
                        (
                            reply,
                            enrollment.phase,
                            enrollment.page,
                            enrollment.reset,
                            enrollment.candidate.clone(),
                            enrollment.owner.clone(),
                        )
                    })
                };
                if let Some((reply, phase, page, reset, candidate, owner)) = message {
                    let page = page.ok_or(OtpSetupError::UnexpectedPage)?;
                    match (reply.kind.as_str(), phase) {
                        ("settings", Phase::Inspect) if reply.registered => {
                            if !reset {
                                return Err(OtpSetupError::AlreadyRegistered);
                            }
                            let auth = self.0.auth.clone();
                            let account = reply.owner.clone();
                            tokio::task::spawn_blocking(move || {
                                auth.check_enrollment_owner(&account)
                            })
                            .await
                            .map_err(|_| OtpSetupError::Storage)?
                            .map_err(|_| OtpSetupError::Storage)?;
                            self.enrollment_phase(id, Phase::Removing)?;
                            self.0.enrollment_script(
                                page,
                                "remove",
                                serde_json::json!({"owner":reply.owner}),
                            )?;
                        }
                        ("settings", Phase::Inspect | Phase::Removing) if !reply.registered => {
                            self.enrollment_phase(id, Phase::Reading)?;
                            self.navigate(SchoolSite::SciencePortal, REGISTER)
                                .map_err(|_| OtpSetupError::Unavailable)?;
                        }
                        ("secret", Phase::Inspect | Phase::Reading) => {
                            let auth = self.0.auth.clone();
                            let setup = reply.secret.clone();
                            let account = reply.owner.clone();
                            let operation = candidate.clone();
                            tokio::task::spawn_blocking(move || {
                                auth.stage_enrollment_otp(&account, &setup, &operation)
                            })
                            .await
                            .map_err(|_| OtpSetupError::Storage)?
                            .map_err(|_| OtpSetupError::Storage)?;
                            self.enrollment_phase(id, Phase::Submitting)?;
                            {
                                let mut state = self.0.state.guard();
                                if let Some(enrollment) = state.otp_enrollment.as_mut() {
                                    enrollment.owner = reply.owner.clone();
                                }
                            }
                            let code = upcoming_otps(&reply.secret)
                                .map_err(|_| OtpSetupError::UnexpectedPage)?
                                .into_iter()
                                .next()
                                .ok_or(OtpSetupError::UnexpectedPage)?;
                            self.0.enrollment_script(page, "submit", serde_json::json!({"owner":reply.owner,"secret":reply.secret,"code":code.code,"validUntil":code.from + 30}))?;
                        }
                        ("settings", Phase::Submitting)
                            if reply.registered && reply.owner == owner =>
                        {
                            let auth = self.0.auth.clone();
                            tokio::task::spawn_blocking(move || {
                                auth.confirm_enrollment_otp(&candidate, &owner)
                            })
                            .await
                            .map_err(|_| OtpSetupError::Storage)?
                            .map_err(|_| OtpSetupError::Storage)?;
                            self.0.set_session_from_inner(
                                SchoolAuthProfile::Science,
                                SchoolSessionState::Active,
                            );
                            let _ = self.0.events.emit(CoreEvent::SchoolLogin(SchoolLoginEvent {
                                profile: SchoolAuthProfile::Science,
                                state: "connected".into(),
                                message: "OTP setup complete".into(),
                            }));
                            return Ok(());
                        }
                        _ => return Err(OtpSetupError::Rejected),
                    }
                }
                tokio::select! {
                    _ = self.0.tasks.closed() => return Err(OtpSetupError::Cancelled),
                    event = traces.recv() => match event {
                        Ok(Trace::Closed(browser)) if browser.site == SchoolSite::SciencePortal => return Err(OtpSetupError::Cancelled),
                        Ok(Trace::Stopped) | Err(broadcast::error::RecvError::Closed) => return Err(OtpSetupError::Cancelled),
                        _ => {},
                    }
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(180), work)
            .await
            .unwrap_or(Err(OtpSetupError::TimedOut))
    }

    fn enrollment_phase(&self, id: u64, phase: Phase) -> Result<(), OtpSetupError> {
        let mut state = self.0.state.guard();
        if state.otp_setup != Some(id) {
            return Err(OtpSetupError::Cancelled);
        }
        state
            .otp_enrollment
            .as_mut()
            .ok_or(OtpSetupError::Cancelled)?
            .phase = phase;
        Ok(())
    }
}
