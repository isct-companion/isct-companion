//! Background sign-in for Rust transports. The WebView still runs the SSO
//! (password, then Matrix or OTP); transports only ask for it and wait.

use super::{BoxFuture, NetError};
use crate::school_auth::SchoolAuthProfile;
use futures_util::future::{FutureExt, Shared};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use tokio::sync::Mutex;

pub trait SignIn: Send + Sync {
    /// Run (or join) the WebView SSO for this profile using saved credentials.
    fn sign_in<'a>(&'a self, profile: SchoolAuthProfile) -> BoxFuture<'a, Result<(), NetError>>;
}

/// What one read found: its value, or a school session that is gone.
pub(crate) enum Session<T> {
    Live(T),
    Gone,
}

type Flight = Shared<BoxFuture<'static, Result<(), NetError>>>;

struct Gate {
    inner: Arc<dyn SignIn>,
    flights: Mutex<HashMap<SchoolAuthProfile, (u64, Flight)>>,
    next_id: AtomicU64,
}

/// Single-flight per profile: concurrent callers join one sign-in.
#[derive(Clone)]
pub struct SignInGate(Arc<Gate>);

impl SignInGate {
    pub fn new(inner: Arc<dyn SignIn>) -> Self {
        Self(Arc::new(Gate {
            inner,
            flights: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(0),
        }))
    }

    /// Signs in, or joins the sign-in already running for `profile`. A
    /// sign-in that finished, either way, is not reused: the next caller
    /// starts a fresh one. If every waiter is dropped, the sign-in pauses with
    /// them and the next caller resumes it.
    pub async fn ensure(&self, profile: SchoolAuthProfile) -> Result<(), NetError> {
        let (id, flight) = {
            let mut flights = self.0.flights.lock().await;
            match flights.get(&profile) {
                Some((id, flight)) => (*id, flight.clone()),
                None => {
                    let id = self.0.next_id.fetch_add(1, Ordering::Relaxed);
                    let inner = self.0.inner.clone();
                    let flight: BoxFuture<'static, Result<(), NetError>> =
                        Box::pin(async move { inner.sign_in(profile).await });
                    let flight = flight.shared();
                    flights.insert(profile, (id, flight.clone()));
                    (id, flight)
                }
            }
        };
        let result = flight.await;
        let mut flights = self.0.flights.lock().await;
        if flights
            .get(&profile)
            .is_some_and(|(current, _)| *current == id)
        {
            flights.remove(&profile);
        }
        result
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
