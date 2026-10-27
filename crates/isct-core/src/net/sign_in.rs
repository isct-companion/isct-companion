//! Background sign-in for Rust transports. The WebView still runs the SSO
//! (password, then Matrix or OTP); transports only ask for it and wait.

use super::{BoxFuture, NetError};
use crate::school_auth::SchoolAuthProfile;
use std::sync::Arc;

pub trait SignIn: Send + Sync {
    /// Run (or join) the WebView SSO for this profile using saved credentials.
    fn sign_in<'a>(&'a self, profile: SchoolAuthProfile) -> BoxFuture<'a, Result<(), NetError>>;
}

/// What one read found: its value, or a school session that is gone.
pub(crate) enum Session<T> {
    Live(T),
    Gone,
}

/// The transports' way into the school browser's session jobs. Joining a
/// sign-in already under way, and keeping it running when every caller has
/// stopped waiting, belong to the engine's job for the profile, so this holds
/// no flights of its own.
#[derive(Clone)]
pub struct SignInGate(Arc<dyn SignIn>);

impl SignInGate {
    pub fn new(inner: Arc<dyn SignIn>) -> Self {
        Self(inner)
    }

    /// Signs in, or joins the sign-in already running for `profile`.
    pub async fn ensure(&self, profile: SchoolAuthProfile) -> Result<(), NetError> {
        self.0.sign_in(profile).await
    }

    /// Runs `read`; when the session is gone, signs in once (or joins the
    /// sign-in already running) and reads again. `None` when the session is
    /// still gone or no saved sign-in can run.
    pub(crate) async fn signed_in<T, E, Fut>(
        &self,
        profile: SchoolAuthProfile,
        mut read: impl FnMut() -> Fut,
    ) -> Result<Option<T>, E>
    where
        E: From<NetError>,
        Fut: std::future::Future<Output = Result<Session<T>, E>>,
    {
        if let Session::Live(value) = read().await? {
            return Ok(Some(value));
        }
        match self.ensure(profile).await {
            Ok(()) => {}
            Err(NetError::SignInRequired) => return Ok(None),
            Err(error) => return Err(error.into()),
        }
        Ok(match read().await? {
            Session::Live(value) => Some(value),
            Session::Gone => None,
        })
    }
}
