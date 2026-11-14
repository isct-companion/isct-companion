//! User-authorized enrollment on the visible Extic portal.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::{school_auth::EnrollmentError, school_session::fresh_otp};
use serde_json::{json, Value};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const SITE: SchoolSite = SchoolSite::SciencePortal;
const PROFILE: SchoolAuthProfile = SchoolAuthProfile::Science;
const ORIGIN: &str = "https://isct.ex-tic.com";
const SETTINGS: &str = "/idm/user/mfa_setting";
const ADD: &str = "/idm/user/totp_setting/add";
pub(super) const ENTRY: &str = "https://isct.ex-tic.com/idm/user/mfa_setting/";
const DEADLINE: Duration = Duration::from_secs(600);

#[derive(Clone, Copy, Eq, PartialEq)]
enum Phase {
    Inspect,
    CheckOwner,
    Confirm(PageId),
    Removing,
    ReadSecret,
    StageSecret,
    Submitting,
    Saving,
    End,
}

pub(super) struct Enrollment {
    pub(super) view: SchoolOtpEnrollment,
    cancel: CancellationToken,
    browser: BrowserId,
    account: u64,
    username: String,
    base: Option<String>,
    phase: Phase,
    request: Option<(PageId, ActionId)>,
    candidate: String,
    mutation: bool,
    reset_removed: bool,
    redirected: bool,
    /// The first load must be the navigation that opens the setup visit.
    first_page: u64,
    authorizing: bool,
}

impl Enrollment {
    pub(super) fn active(&self) -> bool {
        self.phase != Phase::End
    }

    fn prevents_restart(&self) -> bool {
        matches!(self.view.stage, SchoolOtpStage::Saving | SchoolOtpStage::SaveFailed | SchoolOtpStage::Unknown)
            || (self.active() && self.mutation)
    }

    pub(super) fn stop(&mut self) {
        if self.active() && self.phase != Phase::Saving {
            self.end(if self.mutation {
                SchoolOtpStage::Unknown
            } else {
                SchoolOtpStage::Incomplete
            });
        }
    }

    fn end(&mut self, stage: SchoolOtpStage) {
        self.phase = Phase::End;
        self.view.stage = stage;
        self.request = None;
        self.cancel.cancel();
    }

    fn failed(&mut self, failure: EnrollmentError) {
        self.end(match failure {
            _ if self.mutation => SchoolOtpStage::Unknown,
            EnrollmentError::Account => SchoolOtpStage::AccountMismatch,
            _ => SchoolOtpStage::Incomplete,
        });
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Reply {
    Settings { owner: String, registered: bool },
    Secret { owner: String, secret: String },
    Sent,
    Error,
}

fn otp_path(url: &Url) -> Option<&str> {
    if url.origin().ascii_serialization() != ORIGIN
        || url.query().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    match url.path().strip_suffix('/').unwrap_or(url.path()) {
        SETTINGS => Some(SETTINGS),
        ADD => Some(ADD),
        _ => None,
    }
}

impl State {
    fn otp_document(&self, page: PageId) -> bool {
        self.browsing == Some(SITE)
            && page.browser.site == SITE
            && self.views.get(&SITE).is_some_and(|view| {
                view.id == page.browser
                    && view.last_page == page.sequence
                    && view.navigation.is_none()
                    && !view.otp_loading
                    && !view.covered
                    && view.write_pending.is_none()
                    && view.popups.is_empty()
            })
    }
}

impl SchoolBrowserEngine {
    pub(super) fn start_otp(
        &self,
        browser: BrowserId,
        username: &str,
    ) -> Result<bool, SchoolBrowserError> {
        let session = self
            .0
            .sessions
            .permit(PROFILE)
            .ok_or(SchoolBrowserError::SignOutIncomplete)?;
        let account = session.generation();
        drop(session);
        let cancel = CancellationToken::new();
        let id = self.0.next_action.fetch_add(1, Ordering::Relaxed);
        let initialized = self.0.state.with(|state| {
            if state.otp.as_ref().is_some_and(Enrollment::prevents_restart) {
                return false;
            }
            let Some(view) = state.views.get(&SITE) else {
                return false;
            };
            if let Some(old) = state.otp.as_mut() {
                old.stop();
            }
            let first_page = view.last_page;
            state.otp = Some(Enrollment {
                view: SchoolOtpEnrollment {
                    id,
                    stage: SchoolOtpStage::SignIn,
                    owner: None,
                },
                cancel: cancel.clone(),
                browser,
                account,
                username: username.trim().into(),
                base: None,
                phase: Phase::Inspect,
                request: None,
                candidate: uuid::Uuid::new_v4().to_string(),
                mutation: false,
                reset_removed: false,
                redirected: false,
                first_page,
                authorizing: false,
            });
            true
        });
        if !initialized {
            return Ok(false);
        }
        let inner = self.0.clone();
        self.0.tasks.spawn_on(&self.0.runtime, async move {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = inner.tasks.closed() => {},
                _ = tokio::time::sleep(DEADLINE) => {},
            }
            inner.fail_otp(id, EnrollmentError::Stopped);
        });
        Ok(true)
    }

    pub fn school_otp_intent(
        &self,
        id: u64,
        intent: SchoolOtpIntent,
    ) -> Result<bool, SchoolBrowserError> {
        let inner = &self.0;
        if intent == SchoolOtpIntent::Stop {
            inner.stop_otp(id);
            return Ok(true);
        }
        let reset = inner.state.with(|state| {
            let otp = state.otp.as_ref().filter(|otp| otp.view.id == id)?;
            match (intent, otp.phase, otp.view.stage) {
                (SchoolOtpIntent::Reset, Phase::Confirm(page), _) if state.otp_document(page) => {
                    Some(Ok((page, otp.view.owner.clone()?)))
                }
                (SchoolOtpIntent::RetrySave, Phase::End, SchoolOtpStage::SaveFailed) => {
                    Some(Err(()))
                }
                _ => None,
            }
        });
        match reset {
            Some(Ok((page, owner))) => {
                let Some((generation, base)) = inner.state.with(|state| {
                    let otp = state.otp.as_mut().filter(|otp| {
                        otp.view.id == id && otp.phase == Phase::Confirm(page) && !otp.authorizing
                    })?;
                    otp.authorizing = true;
                    Some((otp.account, otp.base.clone()))
                }) else {
                    return Ok(false);
                };
                let inner = inner.clone();
                let work = inner.clone();
                inner.tasks.spawn_on(&inner.runtime, async move {
                    let inner = work;
                    let actual = owner.clone();
                    let checked = inner
                        .sessions
                        .otp_vault(generation, move |auth| {
                            auth.enrollment_account(&owner, &owner)
                        })
                        .await;
                    match checked {
                        Ok(current) if current == base => {
                            inner.otp_eval(
                                id,
                                page,
                                SETTINGS,
                                "remove",
                                json!({ "owner": actual }),
                                Some(Phase::Removing),
                            );
                        }
                        Ok(_) => inner.fail_otp(id, EnrollmentError::Account),
                        Err(error) => inner.fail_otp(id, error),
                    }
                });
                Ok(true)
            }
            Some(Err(())) => {
                inner.save_otp(id);
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

impl Inner {
    pub(super) fn leave_otp(&self) {
        self.state.with(|state| {
            if let Some(otp) = state.otp.as_mut() {
                otp.stop();
            }
        });
        self.publish_page(SITE);
    }

    pub(super) fn stop_otp(&self, id: u64) {
        self.state.with(|state| {
            if let Some(otp) = state.otp.as_mut().filter(|otp| otp.view.id == id) {
                if otp.mutation && otp.active() {
                    otp.stop();
                } else if otp.phase != Phase::Saving {
                    otp.cancel.cancel();
                    state.otp = None;
                }
            }
        });
        self.publish_page(SITE);
    }

    pub(super) fn otp_script_allowed(&self, page: PageId, action: ActionId, url: &Url) -> bool {
        if self.tasks.is_closed() || self.sessions.web_cleanup_pending() {
            return false;
        }
        self.state.with(|state| {
            otp_path(url).is_some()
                && state
                    .views
                    .get(&SITE)
                    .is_some_and(|view| view.url == url.as_str())
                && state.otp_document(page)
                && state.otp.as_ref().is_some_and(|otp| {
                    otp.request == Some((page, action))
                        && self.sessions.account_generation(PROFILE) == otp.account
                })
        })
    }

    fn otp_eval(
        self: &Arc<Self>,
        id: u64,
        page: PageId,
        path: &str,
        command: &str,
        mut config: Value,
        mutation: Option<Phase>,
    ) {
        let action = ActionId(self.next_action.fetch_add(1, Ordering::Relaxed));
        let scheduled = self.state.with(|state| {
            if !state.otp_document(page) {
                return false;
            }
            let Some(otp) = state
                .otp
                .as_mut()
                .filter(|otp| otp.view.id == id && otp.active())
            else {
                return false;
            };
            // A removal can only come from the explicit intent on the
            // inspected settings document; a submission needs a staged seed.
            let expected = match mutation {
                Some(Phase::Removing) => otp.phase == Phase::Confirm(page),
                Some(Phase::Submitting) => otp.phase == Phase::StageSecret,
                _ => true,
            };
            if !expected || otp.request.is_some() {
                return false;
            }
            config["action"] = json!(command);
            config["path"] = json!(path);
            config["document"] = json!(format!("{}:{}", otp.candidate, page.sequence));
            otp.request = Some((page, action));
            if let Some(phase) = mutation {
                otp.phase = phase;
                otp.authorizing = false;
                otp.mutation = true;
                otp.view.stage = SchoolOtpStage::SettingUp;
            }
            true
        });
        if !scheduled {
            if mutation.is_some() {
                self.fail_otp(id, EnrollmentError::Stopped);
            }
            return;
        }
        self.publish_page(SITE);
        let script = adpt_isct::scripts::OTP_ENROLLMENT
            .replace("__OTP_ENROLLMENT_CONFIG__", &config.to_string());
        if self
            .browser
            .dispatch(BrowserCommand {
                browser: page.browser,
                action,
                command: BrowserAction::EvalOtp {
                    page,
                    script: OtpScript(script),
                },
            })
            .is_err()
        {
            self.otp_result(page, action, None);
        }
    }

    pub(super) fn otp_page(self: &Arc<Self>, event: &PageLoaded) {
        let page = event.page;
        let path = otp_path(&event.url);
        let ready = self.state.with(|state| {
            if !state.otp_document(page) {
                return None;
            }
            let otp = state.otp.as_mut().filter(|otp| {
                otp.active() && otp.browser == page.browser && page.sequence > otp.first_page
            })?;
            if let Phase::Confirm(old) = otp.phase {
                if old != page {
                    otp.phase = Phase::Inspect;
                    otp.authorizing = false;
                    otp.view.stage = SchoolOtpStage::Checking;
                }
            }
            if otp.request.is_some_and(|(old, _)| old != page) {
                otp.request = None;
            }
            if matches!(
                otp.phase,
                Phase::CheckOwner | Phase::StageSecret | Phase::Saving | Phase::Confirm(_)
            ) {
                return None;
            }
            if is_identity_login(&event.url) {
                if otp.mutation {
                    otp.stop();
                } else {
                    otp.view.stage = SchoolOtpStage::SignIn;
                }
                return None;
            }
            if otp.phase == Phase::Inspect
                && !otp.redirected
                && event.url.origin().ascii_serialization() == ORIGIN
                && matches!(
                    event.url.path().trim_end_matches('/'),
                    "/idm/user" | "/idm/user/portal"
                )
            {
                otp.redirected = true;
                return Some((otp.view.id, None));
            }
            let expected = match otp.phase {
                Phase::Inspect | Phase::Removing | Phase::Submitting => path == Some(SETTINGS),
                Phase::ReadSecret => path == Some(ADD),
                _ => false,
            };
            if !expected {
                // A submitted form may return to its own page with an error.
                // It doesn't prove whether a destructive request took effect.
                otp.stop();
                return None;
            }
            if otp.phase == Phase::Inspect {
                otp.view.stage = SchoolOtpStage::Checking;
            }
            Some((otp.view.id, path))
        });
        self.publish_page(SITE);
        match ready {
            Some((id, Some(path))) => self.otp_eval(id, page, path, "inspect", json!({}), None),
            Some((id, None)) => self.otp_navigate(id, page, ENTRY),
            None => {}
        }
    }

    fn otp_navigate(&self, id: u64, page: PageId, target: &str) {
        let Ok(url) = Url::parse(target) else {
            self.fail_otp(id, EnrollmentError::Invalid);
            return;
        };
        let action = ActionId(self.next_action.fetch_add(1, Ordering::Relaxed));
        let command = self.state.with(|state| {
            if !state.otp_document(page)
                || !state
                    .otp
                    .as_ref()
                    .is_some_and(|otp| otp.view.id == id && otp.active())
            {
                return None;
            }
            let view = state.views.get_mut(&SITE)?;
            view.navigation = Some(action);
            view.actions.insert(action, true);
            Some(BrowserCommand {
                browser: page.browser,
                action,
                command: BrowserAction::Navigate(url),
            })
        });
        if let Some(command) = command {
            if self.browser.dispatch(command).is_err() {
                self.fail_otp(id, EnrollmentError::Stopped);
            }
        }
    }

    pub(super) fn otp_result(
        self: &Arc<Self>,
        page: PageId,
        action: ActionId,
        raw: Option<String>,
    ) {
        let response = raw
            .filter(|value| value.len() <= 8192)
            .and_then(|value| serde_json::from_str::<Reply>(&value).ok());
        let current = self.state.with(|state| {
            if !state.otp_document(page) {
                return None;
            }
            let otp = state
                .otp
                .as_mut()
                .filter(|otp| otp.request == Some((page, action)))?;
            otp.request = None;
            Some((
                otp.view.id,
                otp.phase,
                otp.account,
                otp.username.clone(),
                otp.view.owner.clone(),
                otp.base.clone(),
                otp.candidate.clone(),
            ))
        });
        let Some((id, phase, generation, username, owner, base, candidate)) = current else {
            return;
        };
        match (phase, response) {
            (Phase::Removing | Phase::Submitting, Some(Reply::Sent)) => {}
            (Phase::Removing | Phase::Submitting, Some(Reply::Error)) => {
                self.state.with(|state| {
                    if let Some(otp) = state
                        .otp
                        .as_mut()
                        .filter(|otp| otp.view.id == id && otp.phase == phase)
                    {
                        otp.end(if phase == Phase::Removing || !otp.reset_removed {
                            SchoolOtpStage::Incomplete
                        } else {
                            SchoolOtpStage::Unknown
                        });
                    }
                });
                self.publish_page(SITE);
            }
            (Phase::Inspect, Some(Reply::Settings { owner, registered })) => {
                if !self.otp_phase(id, page, Phase::CheckOwner, |otp| {
                    otp.view.owner = Some(owner.clone());
                }) {
                    return;
                }
                let inner = self.clone();
                self.tasks.spawn_on(&self.runtime, async move {
                    let checked = inner
                        .sessions
                        .otp_vault(generation, move |auth| {
                            auth.enrollment_account(&username, &owner)
                        })
                        .await;
                    match checked {
                        Ok(base) => {
                            let phase = if registered {
                                Phase::Confirm(page)
                            } else {
                                Phase::ReadSecret
                            };
                            if inner.otp_phase(id, page, phase, |otp| {
                                otp.base = base;
                                otp.view.stage = if registered {
                                    SchoolOtpStage::ConfirmReset
                                } else {
                                    SchoolOtpStage::SettingUp
                                };
                            }) && !registered
                            {
                                inner.otp_navigate(
                                    id,
                                    page,
                                    "https://isct.ex-tic.com/idm/user/totp_setting/add",
                                );
                            }
                        }
                        Err(error) => inner.fail_otp(id, error),
                    }
                });
            }
            (
                Phase::Removing,
                Some(Reply::Settings {
                    owner: actual,
                    registered: false,
                }),
            ) if owner.as_deref() == Some(&actual) => {
                if self.otp_phase(id, page, Phase::ReadSecret, |otp| otp.reset_removed = true) {
                    self.otp_navigate(
                        id,
                        page,
                        "https://isct.ex-tic.com/idm/user/totp_setting/add",
                    );
                }
            }
            (
                Phase::ReadSecret,
                Some(Reply::Secret {
                    owner: actual,
                    secret,
                }),
            ) if owner.as_deref() == Some(&actual) => {
                if !self.otp_phase(id, page, Phase::StageSecret, |_| {}) {
                    return;
                }
                let inner = self.clone();
                self.tasks.spawn_on(&self.runtime, async move {
                    let (prepared, setup, account) = (secret.clone(), secret, actual.clone());
                    let staged = inner.sessions.otp_vault(generation, move |auth| {
                        auth.stage_enrollment(&candidate, &account, base, &prepared)
                    }).await;
                    if let Err(error) = staged {
                        inner.fail_otp(id, error);
                        return;
                    }
                    let Ok((window, period)) = fresh_otp(&setup).await else {
                        inner.fail_otp(id, EnrollmentError::Invalid);
                        return;
                    };
                    inner.otp_eval(
                        id,
                        page,
                        ADD,
                        "submit",
                        json!({
                            "owner": actual,
                            "secret": setup,
                            "code": window.code,
                            "validFrom": window.from,
                            "validUntil": window.from + period,
                        }),
                        Some(Phase::Submitting),
                    );
                });
            }
            (
                Phase::Submitting,
                Some(Reply::Settings {
                    owner: actual,
                    registered: true,
                }),
            ) if owner.as_deref() == Some(&actual) => self.save_otp(id),
            _ => self.fail_otp(id, EnrollmentError::Invalid),
        }
    }

    /// Update only while this document belongs to the live operation.
    fn otp_phase(
        &self,
        id: u64,
        page: PageId,
        phase: Phase,
        update: impl FnOnce(&mut Enrollment),
    ) -> bool {
        let changed = self.state.with(|state| {
            if !state.otp_document(page) {
                return false;
            }
            let Some(otp) = state
                .otp
                .as_mut()
                .filter(|otp| otp.view.id == id && otp.active())
            else {
                return false;
            };
            otp.phase = phase;
            update(otp);
            true
        });
        if changed {
            self.publish_page(SITE);
        } else {
            self.fail_otp(id, EnrollmentError::Stopped);
        }
        changed
    }

    fn fail_otp(&self, id: u64, error: EnrollmentError) {
        self.state.with(|state| {
            if let Some(otp) = state
                .otp
                .as_mut()
                .filter(|otp| otp.view.id == id && otp.active())
            {
                otp.failed(error);
            }
        });
        self.publish_page(SITE);
    }

    fn save_otp(self: &Arc<Self>, id: u64) {
        let job = self.state.with(|state| {
            let otp = state.otp.as_mut().filter(|otp| otp.view.id == id)?;
            if otp.phase != Phase::Submitting && otp.view.stage != SchoolOtpStage::SaveFailed {
                return None;
            }
            otp.view.stage = SchoolOtpStage::Saving;
            otp.phase = Phase::Saving;
            otp.cancel.cancel();
            Some((otp.account, otp.candidate.clone()))
        });
        let Some((account, candidate)) = job else {
            return;
        };
        self.publish_page(SITE);
        let inner = self.clone();
        self.tasks.spawn_on(&self.runtime, async move {
            let saved = inner.sessions.confirm_otp(account, candidate).await;
            inner.state.with(|state| {
                if let Some(otp) = state.otp.as_mut().filter(|otp| otp.view.id == id) {
                    otp.end(match saved {
                        Ok(()) => SchoolOtpStage::Complete,
                        Err(EnrollmentError::Account | EnrollmentError::Stopped) => {
                            SchoolOtpStage::Unknown
                        }
                        Err(_) => SchoolOtpStage::SaveFailed,
                    });
                }
            });
            inner.publish_page(SITE);
        });
    }
}
