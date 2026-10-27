//! `Http`: one long-lived client per host with that host's budget, timeouts,
//! byte cap and circuit breaker.

use super::{
    breaker::{Admission, Breaker, Outcome},
    redact::redact_text,
    tls, Body, Host, NetError, NetRequest, Response, USER_AGENT,
};
use std::{collections::HashMap, error::Error as _, sync::Arc, time::Duration};
use tokio::{
    sync::{Mutex, Semaphore},
    time::Instant,
};

/// No bytes (headers or body) for this long fails the request as stalled.
const STALL: Duration = Duration::from_secs(20);
const CONNECT: Duration = Duration::from_secs(5);

struct Lane {
    client: reqwest::Client,
    permits: Semaphore,
    spacing: Duration,
    next_start: Mutex<Option<Instant>>,
}

impl Lane {
    /// Waits for this request's turn: a free slot, then the fixed spacing
    /// after the previous start.
    async fn turn(&self) -> Result<tokio::sync::SemaphorePermit<'_>, NetError> {
        let permit = self
            .permits
            .acquire()
            .await
            .map_err(|_| NetError::Cancelled)?;
        if !self.spacing.is_zero() {
            let start = {
                let mut next = self.next_start.lock().await;
                let now = Instant::now();
                let start = next.map_or(now, |next| next.max(now));
                *next = Some(start + self.spacing);
                start
            };
            tokio::time::sleep_until(start).await;
        }
        Ok(permit)
    }
}

struct Inner {
    lanes: HashMap<Host, Lane>,
    breaker: Breaker,
}

#[derive(Clone)]
pub struct Http(Arc<Inner>);

fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

impl Http {
    pub fn new() -> Result<Self, NetError> {
        let mut lanes = HashMap::new();
        for host in Host::ALL {
            let builder = super::client_builder()
                .user_agent(USER_AGENT)
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(CONNECT)
                .pool_idle_timeout(Duration::from_secs(90));
            let builder = if host == Host::Kyomu {
                builder.tls_backend_preconfigured(tls::kyomu_config().map_err(NetError::Tls)?)
            } else {
                builder
            };
            let client = builder
                .build()
                .map_err(|error| NetError::Invalid(format!("HTTP client: {error}")))?;
            let (concurrent, spacing) = host.budget();
            lanes.insert(
                host,
                Lane {
                    client,
                    permits: Semaphore::new(concurrent),
                    spacing,
                    next_start: Mutex::new(None),
                },
            );
        }
        Ok(Self(Arc::new(Inner {
            lanes,
            breaker: Breaker::default(),
        })))
    }

    /// Never follows redirects. Applies the host's concurrency/spacing, the
    /// 20 s stall timeout and `limits.deadline`, the byte cap, and the host
    /// circuit breaker (returns HostPaused without sending while paused).
    /// Always sends the app User-Agent "Mozilla/5.0 (compatible; IsctCompanion/0.1)".
    ///
    /// Every status comes back as `Ok`; the caller decides what a 302, 404 or
    /// 503 means. 5xx statuses still count as host failures. The deadline
    /// starts when the request leaves the host queue.
    pub async fn send(&self, request: NetRequest) -> Result<Response, NetError> {
        if Host::of(&request.url) != Some(request.host) {
            return Err(NetError::Invalid(format!(
                "URL does not belong to {}",
                request.host.name()
            )));
        }
        let lane = self
            .0
            .lanes
            .get(&request.host)
            .ok_or_else(|| NetError::Invalid("no client for host".into()))?;
        let host = request.host;
        // Checked before queueing so paused hosts fail fast, and again at the
        // turn so a pause that began meanwhile still holds the request.
        self.paused(host)?;
        let _permit = lane.turn().await?;
        let admission = self
            .0
            .breaker
            .admit(host, now_unix())
            .map_err(|until_unix| NetError::HostPaused { host, until_unix })?;
        let mut ticket = Ticket {
            breaker: &self.0.breaker,
            host,
            admission,
            settled: false,
        };
        let result = exchange(&lane.client, request).await;
        let outcome = match &result {
            Ok(response) if response.status >= 500 => Outcome::Failure,
            Ok(_) => Outcome::Success,
            Err(NetError::Connect(_) | NetError::Dns | NetError::Stalled | NetError::Deadline) => {
                Outcome::Failure
            }
            Err(_) => Outcome::Neutral,
        };
        ticket.settle(outcome);
        result
    }

    /// The caller saw a maintenance page: pause the host now.
    pub fn report_maintenance(&self, host: Host) {
        self.0.breaker.trip(host, now_unix());
    }

    fn paused(&self, host: Host) -> Result<(), NetError> {
        match self.0.breaker.paused_until(host, now_unix()) {
            Some(until_unix) => Err(NetError::HostPaused { host, until_unix }),
            None => Ok(()),
        }
    }
}

/// Releases a probe that never finished (its future was dropped), so the
/// host is not stuck waiting for a result that will not come.
struct Ticket<'a> {
    breaker: &'a Breaker,
    host: Host,
    admission: Admission,
    settled: bool,
}

impl Ticket<'_> {
    fn settle(&mut self, outcome: Outcome) {
        self.settled = true;
        self.breaker
            .record(self.host, self.admission, outcome, now_unix());
    }
}

impl Drop for Ticket<'_> {
    fn drop(&mut self) {
        if !self.settled {
            self.breaker
                .record(self.host, self.admission, Outcome::Neutral, now_unix());
        }
    }
}

/// Runs `step` within both the stall window and the overall deadline.
async fn bounded<T>(
    deadline: Instant,
    step: impl std::future::Future<Output = T>,
) -> Result<T, NetError> {
    match tokio::time::timeout_at(deadline, tokio::time::timeout(STALL, step)).await {
        Err(_) => Err(NetError::Deadline),
        Ok(Err(_)) => Err(NetError::Stalled),
        Ok(Ok(value)) => Ok(value),
    }
}

async fn exchange(client: &reqwest::Client, request: NetRequest) -> Result<Response, NetError> {
    let deadline = Instant::now() + request.limits.deadline;
    let max_bytes = request.limits.max_bytes;
    let mut builder = client.request(request.method, request.url);
    for (name, value) in request.headers {
        builder = builder.header(name, value);
    }
    builder = match request.body {
        Body::None => builder,
        Body::Form(fields) => builder.form(&fields),
        Body::Json(value) => builder.json(&value),
    };
    let mut response = bounded(deadline, builder.send()).await?.map_err(classify)?;
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(NetError::TooLarge);
    }
    let status = response.status().as_u16();
    let url = response.url().clone();
    let headers = response.headers().clone();
    let mut body = bytes::BytesMut::new();
    while let Some(chunk) = bounded(deadline, response.chunk())
        .await?
        .map_err(classify)?
    {
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(NetError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Response {
        status,
        url,
        headers,
        body: body.freeze(),
    })
}

/// A typed error without the request URL (it may carry a sesskey or token).
fn classify(error: reqwest::Error) -> NetError {
    let error = error.without_url();
    if error.is_timeout() {
        return NetError::Stalled;
    }
    if error.is_dns() {
        return NetError::Dns;
    }
    let mut source = error.source();
    while let Some(cause) = source {
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            if matches!(
                io.kind(),
                std::io::ErrorKind::NetworkUnreachable | std::io::ErrorKind::NetworkDown
            ) {
                return NetError::Offline;
            }
            // hyper-rustls wraps the handshake error in an io::Error, whose
            // `source()` skips the wrapped error itself.
            if let Some(tls) = io
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<rustls::Error>())
            {
                return NetError::Tls(redact_text(&tls.to_string()));
            }
        }
        if let Some(tls) = cause.downcast_ref::<rustls::Error>() {
            return NetError::Tls(redact_text(&tls.to_string()));
        }
        source = cause.source();
    }
    NetError::Connect(redact_text(&chain(&error)))
}

/// The error and its causes on one line; reqwest's Display stops at the top.
fn chain(error: &reqwest::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}
