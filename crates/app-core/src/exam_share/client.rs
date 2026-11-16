use super::ExamError;
use crate::net::{client_builder, read_response_limited, Host};
use reqwest::{Method, RequestBuilder, Response, StatusCode};
use serde::{de::DeserializeOwned, Deserialize};
use std::time::Duration;
use tokio::{sync::Mutex, time::Instant};
use url::Url;

const MAX_JSON: usize = 16 * 1024 * 1024;
const API_KEY: &str = "AIzaSyBKTbvs7OBhZB__eISYaznVp_R-QMLMFEM";
pub(super) const FIREBASE_PROJECT: &str = "gen-lang-client-0463137884";

pub(super) struct Client {
    pub http: reqwest::Client,
    version: String,
    last_start: Mutex<Option<Instant>>,
}

impl Client {
    pub fn new(version: String) -> Result<Self, ExamError> {
        Ok(Self {
            http: client_builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(45))
                .build()
                .map_err(|_| ExamError::Unavailable)?,
            version,
            last_start: Mutex::new(None),
        })
    }

    pub fn request(
        &self,
        host: Host,
        method: Method,
        path: &[&str],
    ) -> Result<RequestBuilder, ExamError> {
        let mut url =
            Url::parse(&format!("https://{}/", host.name())).map_err(|_| ExamError::Unavailable)?;
        url.path_segments_mut()
            .map_err(|_| ExamError::Unavailable)?
            .pop_if_empty()
            .extend(path.iter().copied());
        Ok(self
            .http
            .request(method, url)
            .header("X-Client", format!("isct-companion/{}", self.version)))
    }

    pub async fn service<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
    ) -> Result<T, ExamError> {
        {
            let mut last = self.last_start.lock().await;
            if let Some(last) = *last {
                tokio::time::sleep_until(last + Host::ScienceTokyoExam.budget().1).await;
            }
            *last = Some(Instant::now());
        }
        let response = request.send().await.map_err(network)?;
        let path = response.url().path().to_owned();
        if response.headers().contains_key("X-Vercel-Mitigated") {
            log::warn!("[exam-share] {path}: challenged by the Vercel firewall");
            return Err(ExamError::Unavailable);
        }
        let status = response.status();
        if status.is_success() {
            return read(response).await;
        }
        let body: Result<ServiceError, _> = read(response).await;
        let code = body.ok().and_then(|body| body.code);
        log::warn!("[exam-share] {path}: {status} {code:?}");
        match (status, code.as_deref()) {
            (StatusCode::UNAUTHORIZED, _) => Err(ExamError::Expired),
            (_, Some("auth.not_allowed")) => Err(ExamError::Denied),
            (_, Some("auth.not_registered")) => Err(ExamError::Consent),
            (_, Some("server.not_found")) => Err(ExamError::Unsupported),
            (_, Some("exam.report_duplicate")) => Err(ExamError::AlreadyReported),
            (_, Some("request.daily_limit")) => Err(ExamError::Quota),
            (StatusCode::NOT_FOUND, _) => Err(ExamError::NotFound),
            (StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS, None) => {
                Err(ExamError::Unavailable)
            }
            (_, Some("write_freeze" | "server.write_freeze" | "server.not_ready")) => {
                Err(ExamError::Unavailable)
            }
            (_, _) if status.is_server_error() => Err(ExamError::Unavailable),
            _ => Err(ExamError::Rejected),
        }
    }

    pub async fn firebase<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
    ) -> Result<T, ExamError> {
        let response = request
            .query(&[("key", API_KEY)])
            .send()
            .await
            .map_err(network)?;
        let status = response.status();
        if status.is_success() {
            return read(response).await;
        }
        let body: Result<FirebaseError, _> = read(response).await;
        let message = body.ok().map(|error| error.error.message);
        log::warn!("[exam-share] identity: {status} {message:?}");
        match message.as_deref() {
            Some(
                "TOKEN_EXPIRED" | "USER_DISABLED" | "USER_NOT_FOUND" | "INVALID_REFRESH_TOKEN",
            ) => Err(ExamError::Expired),
            _ if status.is_server_error() => Err(ExamError::Unavailable),
            _ => Err(ExamError::Rejected),
        }
    }
}

#[derive(Deserialize)]
struct ServiceError {
    code: Option<String>,
}

#[derive(Deserialize)]
struct FirebaseError {
    error: FirebaseMessage,
}

#[derive(Deserialize)]
struct FirebaseMessage {
    message: String,
}

pub(super) fn network(error: reqwest::Error) -> ExamError {
    if error.is_timeout() {
        ExamError::Timeout
    } else {
        ExamError::Network
    }
}

pub(super) async fn read<T: DeserializeOwned>(response: Response) -> Result<T, ExamError> {
    let bytes = read_response_limited(response, MAX_JSON)
        .await
        .map_err(|_| ExamError::Network)?;
    serde_json::from_slice(&bytes).map_err(|_| ExamError::Response)
}
