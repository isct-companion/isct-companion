//! Native OAuth ports. The platform opens the browser; Core owns the grant.

pub mod code_exchange;
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "android"))]
pub mod loopback;

#[cfg(any(
    target_os = "ios",
    target_os = "macos",
    target_os = "windows",
    target_os = "android"
))]
use crate::net::BoxFuture;
#[cfg(any(
    target_os = "ios",
    target_os = "macos",
    target_os = "windows",
    target_os = "android"
))]
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
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "android"))]
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
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "android"))]
pub trait NativeBrowserAuth: Send + Sync {
    fn authenticate<'a>(
        &'a self,
        url: &'a Url,
        callback: BrowserCallback,
    ) -> BoxFuture<'a, Result<(), OAuthError>>;
}
