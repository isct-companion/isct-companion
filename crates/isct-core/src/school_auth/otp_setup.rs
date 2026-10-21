//! Read-only enrollment discovery. Registration and credential rotation stay
//! on the school's own page; this service never submits an enrollment form.
use crate::net::{Host, Http, NetError, NetRequest, SessionCookies};
use scraper::{Html, Selector};
use serde::Serialize;
use std::sync::Arc;

pub const SETTINGS_URL: &str = "https://isct.ex-tic.com/idm/user/mfa_setting/";
const REGISTER_URL: &str = "https://isct.ex-tic.com/idm/user/totp_setting/add";

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SchoolOtpSetup {
    pub registered: bool,
    /// Registered accounts go to settings only after reset confirmation.
    pub url: String,
}

#[derive(Debug, thiserror::Error, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OtpSetupError {
    #[error("Sign in to the school account settings, then try Help me set up again.")]
    SignInRequired,
    #[error("The school OTP settings page could not be recognized.")]
    UnexpectedPage,
    #[error("Could not check school OTP settings. Try again.")]
    Unavailable,
}

impl From<NetError> for OtpSetupError {
    fn from(error: NetError) -> Self {
        match error {
            NetError::SignInRequired => Self::SignInRequired,
            _ => Self::Unavailable,
        }
    }
}

pub(crate) struct OtpSetupClient {
    http: Http,
    cookies: Arc<dyn SessionCookies>,
}

impl OtpSetupClient {
    pub(crate) fn new(http: Http, cookies: Arc<dyn SessionCookies>) -> Self {
        Self { http, cookies }
    }

    pub(crate) async fn read(&self) -> Result<SchoolOtpSetup, OtpSetupError> {
        let url = url::Url::parse(SETTINGS_URL).map_err(|_| OtpSetupError::Unavailable)?;
        let mut request = NetRequest::get(Host::ScienceAccount, url.clone());
        request.headers.push((
            reqwest::header::COOKIE,
            self.cookies.cookie_header(&url).await?,
        ));
        let response = self.http.send(request).await?;
        self.cookies
            .store_set_cookies(&url, &response.set_cookies())
            .await?;
        // Never follow a redirect with account cookies or interpret a login
        // page as an account whose OTP registration is absent.
        if (300..400).contains(&response.status) || matches!(response.status, 401 | 403) {
            return Err(OtpSetupError::SignInRequired);
        }
        if response.status != 200 {
            return Err(OtpSetupError::Unavailable);
        }
        let html = response.text()?;
        let registered = registration_state(&html)?;
        Ok(SchoolOtpSetup {
            registered,
            url: if registered {
                SETTINGS_URL
            } else {
                REGISTER_URL
            }
            .into(),
        })
    }
}

pub(super) fn registration_state(html: &str) -> Result<bool, OtpSetupError> {
    let selector = |raw| Selector::parse(raw).map_err(|_| OtpSetupError::UnexpectedPage);
    let document = Html::parse_document(html);
    let row_selector = selector(".form-group")?;
    let label = selector("label[for='appAuth']")?;
    let setup = selector("button[onclick*='totp_setting/add']")?;
    let remove = selector("button.btn-danger")?;
    let mut rows = document
        .select(&row_selector)
        .filter(|row| row.select(&label).next().is_some());
    let row = rows.next().ok_or(OtpSetupError::UnexpectedPage)?;
    if rows.next().is_some() {
        return Err(OtpSetupError::UnexpectedPage);
    }
    match (
        row.select(&setup).next().is_some(),
        row.select(&remove).next().is_some(),
    ) {
        (true, false) => Ok(false),
        (false, true) => Ok(true),
        _ => Err(OtpSetupError::UnexpectedPage),
    }
}
