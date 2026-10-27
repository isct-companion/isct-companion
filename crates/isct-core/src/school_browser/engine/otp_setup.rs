//! A user-owned, visible enrollment session. No stored OTP is required to
//! begin; the user completes the school's available sign-in method first.
use super::*;
use crate::school_auth::{OtpSetupError, SchoolOtpSetup};

pub(super) struct WaitGuard<'a>(pub(super) &'a SchoolBrowserEngine, pub(super) u64);

impl Drop for WaitGuard<'_> {
    fn drop(&mut self) {
        self.0.cancel_school_otp_setup(self.1);
    }
}

impl SchoolBrowserEngine {
    pub fn begin_school_otp_setup(&self) -> Result<u64, OtpSetupError> {
        if self.0.tasks.is_closed() {
            return Err(OtpSetupError::Cancelled);
        }
        let id = self.0.next_action.fetch_add(1, Ordering::Relaxed);
        let mut state = self.0.state.guard();
        state.otp_setup = Some(id);
        state.otp_enrollment = None;
        if state
            .login
            .as_ref()
            .is_some_and(|login| login.profile == SchoolAuthProfile::Science)
        {
            state.login = None;
        }
        if state
            .heartbeat
            .is_some_and(|heartbeat| heartbeat.profile == SchoolAuthProfile::Science)
        {
            state.heartbeat = None;
        }
        drop(state);
        let _ = self.0.traces.send(Trace::Browsing(None));
        Ok(id)
    }

    pub fn cancel_school_otp_setup(&self, id: u64) {
        let mut state = self.0.state.guard();
        if state.otp_setup == Some(id) {
            state.otp_setup = None;
            state.otp_enrollment = None;
        }
        drop(state);
        let _ = self.0.traces.send(Trace::Browsing(None));
    }

    /// Subscribe before checking the loaded page, recovering from missed
    /// notifications through the browser snapshot. Leaving the visible view,
    /// explicit cancellation, shutdown or ten minutes ends the operation.
    pub async fn school_otp_setup(&self, id: u64) -> Result<SchoolOtpSetup, OtpSetupError> {
        let _ownership = WaitGuard(self, id);
        let mut traces = self.0.traces.subscribe();
        let wait = async {
            let mut checked = None;
            loop {
                let page = {
                    let state = self.0.state.guard();
                    if state.otp_setup != Some(id) {
                        return Err(OtpSetupError::Cancelled);
                    }
                    state
                        .views
                        .get(&SchoolSite::SciencePortal)
                        .filter(|view| {
                            state.browsing == Some(SchoolSite::SciencePortal)
                                && view.url.trim_end_matches('/')
                                    == crate::school_auth::otp_setup::SETTINGS_URL
                                        .trim_end_matches('/')
                        })
                        .map(|view| (view.id, view.last_page))
                };
                if let Some(page) = page.filter(|page| checked.as_ref() != Some(page)) {
                    checked = Some(page);
                    let result = self
                        .transport()
                        .map_err(|_| OtpSetupError::Unavailable)?
                        .otp_setup
                        .read()
                        .await;
                    if self.0.state.guard().otp_setup != Some(id) {
                        return Err(OtpSetupError::Cancelled);
                    }
                    match result {
                        Err(OtpSetupError::SignInRequired) => {
                            self.navigate(
                                SchoolSite::SciencePortal,
                                crate::school_auth::otp_setup::SETTINGS_URL,
                            )
                            .map_err(|_| OtpSetupError::Unavailable)?;
                        }
                        result => return result,
                    }
                }
                tokio::select! {
                    _ = self.0.tasks.closed() => return Err(OtpSetupError::Cancelled),
                    trace = traces.recv() => match trace {
                        Ok(Trace::Closed(browser)) if browser.site == SchoolSite::SciencePortal => return Err(OtpSetupError::Cancelled),
                        Ok(Trace::Stopped) | Err(broadcast::error::RecvError::Closed) => return Err(OtpSetupError::Cancelled),
                        _ => {},
                    },
                }
            }
        };
        let result = tokio::time::timeout(Duration::from_secs(600), wait)
            .await
            .unwrap_or(Err(OtpSetupError::TimedOut));
        self.cancel_school_otp_setup(id);
        result
    }
}
