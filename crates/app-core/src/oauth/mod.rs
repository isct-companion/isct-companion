//! Native OAuth ports. The platform opens the browser; Core owns the grant.

pub mod code_exchange;
#[cfg(not(target_os = "ios"))]
pub mod loopback;

use crate::net::BoxFuture;
use code_exchange::{Options, PendingAuthorization};
use tokio_util::sync::CancellationToken;
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    #[error("OAuth configuration is invalid.")]
    Configuration,
    #[error("Secure random bytes are unavailable.")]
    Random,
    #[error("OAuth callback is invalid.")]
    Callback,
    #[error("OAuth state did not match.")]
    State,
    #[error("OAuth authorization was declined.")]
    Cancelled,
    #[error("OAuth authorization timed out.")]
    Timeout,
    #[error("An authorization is already running.")]
    Busy,
    #[error("The system browser could not be opened.")]
    Browser,
    #[error("OAuth loopback listener failed.")]
    Loopback,
    #[error("OAuth token request failed.")]
    Request,
    #[error("OAuth token response was invalid.")]
    Response,
    #[error("OAuth token request was rejected (HTTP {status}{detail}).")]
    Rejected { status: u16, detail: &'static str },
}

#[cfg(target_os = "ios")]
pub enum BrowserCallback<'a> {
    Scheme(&'a str),
}
#[cfg(not(target_os = "ios"))]
pub enum BrowserCallback {
    Loopback,
}

#[cfg(target_os = "ios")]
pub trait NativeBrowserAuth: Send + Sync {
    fn authenticate<'a>(
        &'a self,
        url: &'a Url,
        callback: BrowserCallback<'a>,
    ) -> BoxFuture<'a, Result<String, OAuthError>>;
    fn cancel(&self) -> BoxFuture<'_, ()>;
}
#[cfg(not(target_os = "ios"))]
pub trait NativeBrowserAuth: Send + Sync {
    fn authenticate<'a>(
        &'a self,
        url: &'a Url,
        callback: BrowserCallback,
    ) -> BoxFuture<'a, Result<(), OAuthError>>;
}

/// How long a person has to finish one authorization in the browser.
const AUTHORIZATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

/// One interactive authorization on iOS: the system session returns through
/// the client's own URL scheme. Cancelling or timing out closes the sheet.
#[cfg(target_os = "ios")]
pub async fn authorize(
    client: &code_exchange::OAuthClient,
    options: Options<'_>,
    browser: &dyn NativeBrowserAuth,
    cancel: &CancellationToken,
) -> Result<(PendingAuthorization, Url), OAuthError> {
    let redirect = client.ios_redirect()?;
    let pending = code_exchange::begin(&client.id, redirect.clone(), options)?;
    let authorize = browser.authenticate(&pending.url, BrowserCallback::Scheme(redirect.scheme()));
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(OAuthError::Cancelled),
        result = tokio::time::timeout(AUTHORIZATION_TIMEOUT, authorize) => {
            result.unwrap_or(Err(OAuthError::Timeout))
        }
    };
    if matches!(result, Err(OAuthError::Cancelled | OAuthError::Timeout)) {
        browser.cancel().await;
    }
    let callback = Url::parse(&result?).map_err(|_| OAuthError::Callback)?;
    Ok((pending, callback))
}

/// One interactive authorization elsewhere: the browser returns to a
/// loopback listener on `port` (0 for an ephemeral one).
#[cfg(not(target_os = "ios"))]
pub async fn authorize(
    client_id: &str,
    port: u16,
    options: Options<'_>,
    browser: &dyn NativeBrowserAuth,
    cancel: &CancellationToken,
) -> Result<(PendingAuthorization, Url), OAuthError> {
    let listener = loopback::Loopback::bind(port).await?;
    let pending = code_exchange::begin(client_id, listener.redirect().clone(), options)?;
    let callback = listener
        .authenticate(browser, &pending, cancel, AUTHORIZATION_TIMEOUT)
        .await?;
    Ok((pending, callback))
}
