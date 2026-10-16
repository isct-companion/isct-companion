//! LMS session readiness policy independent of native webview handles.

use crate::{downloads::FetchError, school_parser::lms::is_lms_origin};
use std::time::Duration;
use tokio::sync::Mutex;
use url::Url;

const POLL_INTERVAL: Duration = Duration::from_millis(200);
const SCHOOL_TIMEOUT: Duration = Duration::from_secs(180);
const LMS_ATTEMPTS: usize = 2;
const LMS_POLLS_PER_ATTEMPT: usize = 100;
const RELOAD_DELAY: Duration = Duration::from_secs(5);
pub const LMS_HOME: &str = "https://lms.s.isct.ac.jp/2025/my/";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionState {
    Pending,
    Active,
    Failed,
}

pub trait LmsSessionPort: Send + Sync {
    fn school_state(&self) -> Result<SessionState, FetchError>;
    fn request_school_session(&self) -> Result<(), FetchError>;
    fn ensure_lms_view(&self) -> Result<(), FetchError>;
    fn lms_ready(&self) -> Result<bool, FetchError>;
    fn navigate_lms_home(&self) -> Result<(), FetchError>;
    fn cookies(&self, url: &Url) -> Result<String, FetchError>;
}

#[derive(Default)]
pub struct LmsSession {
    gate: Mutex<()>,
}

impl LmsSession {
    pub async fn cookie_header(
        &self,
        port: &dyn LmsSessionPort,
        url: &Url,
    ) -> Result<String, FetchError> {
        if !is_lms_origin(url) {
            return Err(FetchError::Invalid);
        }
        let _gate = self.gate.lock().await;
        self.wait_for_school(port).await?;
        port.ensure_lms_view()?;

        for attempt in 0..LMS_ATTEMPTS {
            for _ in 0..LMS_POLLS_PER_ATTEMPT {
                if port.lms_ready()? {
                    let cookies = port.cookies(url)?;
                    return if cookies.trim().is_empty() {
                        Err(FetchError::Login)
                    } else {
                        Ok(cookies)
                    };
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
            if attempt == 0 {
                // The legacy recovery path treated this request as a hint; the
                // explicit navigation below remains the authoritative retry.
                let _ = port.request_school_session();
                tokio::time::sleep(RELOAD_DELAY).await;
                port.navigate_lms_home()?;
            }
        }
        Err(FetchError::Login)
    }

    async fn wait_for_school(&self, port: &dyn LmsSessionPort) -> Result<(), FetchError> {
        if port.school_state()? == SessionState::Active {
            return Ok(());
        }
        port.request_school_session()?;
        tokio::time::timeout(SCHOOL_TIMEOUT, async {
            loop {
                match port.school_state()? {
                    SessionState::Active => return Ok(()),
                    SessionState::Failed => return Err(FetchError::Login),
                    SessionState::Pending => tokio::time::sleep(POLL_INTERVAL).await,
                }
            }
        })
        .await
        .map_err(|_| FetchError::Login)?
    }
}

#[cfg(test)]
#[path = "tests/lms_session.rs"]
mod tests;
