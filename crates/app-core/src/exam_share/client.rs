use super::{ExamError, ExamSitePages};
use crate::net::{client_builder, read_response_limited};
use reqwest::{Method, RequestBuilder, Response, StatusCode};
use serde::{de::DeserializeOwned, Deserialize};
use std::{path::Path, time::Duration};
use tokio::{io::AsyncWriteExt, sync::Mutex, time::Instant};
use url::Url;

const MAX_JSON: usize = 16 * 1024 * 1024;
/// The spacing between request starts the site asks of a client.
const SITE_SPACING: Duration = Duration::from_millis(500);
const API_KEY: &str = "AIzaSyBKTbvs7OBhZB__eISYaznVp_R-QMLMFEM";
pub(super) const FIREBASE_PROJECT: &str = "gen-lang-client-0463137884";

/// The services the client talks to. They are not campus hosts, so they stay
/// out of `net::Host`, whose list the school session, cookie and VPN rules
/// read as the school's own.
#[derive(Clone, Copy)]
pub(super) enum Endpoint {
    Site,
    IdentityToolkit,
    SecureToken,
}

/// A page of the ScienceTokyoExam website, for a shell's "open in browser".
pub(super) fn site_page(path: &str) -> String {
    format!("https://{}/{path}", Endpoint::Site.host())
}

/// The website's pages a shell links to whatever the account's state.
pub(super) fn site_pages() -> ExamSitePages {
    ExamSitePages {
        search: site_page("search"),
        requests: site_page("request"),
        library: site_page("library"),
        terms: site_page("terms"),
        privacy: site_page("privacy"),
    }
}

impl Endpoint {
    pub const fn host(self) -> &'static str {
        match self {
            Self::Site => "sciencetokyoexam.com",
            Self::IdentityToolkit => "identitytoolkit.googleapis.com",
            Self::SecureToken => "securetoken.googleapis.com",
        }
    }
}

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

    fn base(&self, endpoint: Endpoint) -> Result<Url, ExamError> {
        Url::parse(&format!("https://{}/", endpoint.host())).map_err(|_| ExamError::Unavailable)
    }

    pub fn request(
        &self,
        endpoint: Endpoint,
        method: Method,
        path: &[&str],
    ) -> Result<RequestBuilder, ExamError> {
        let mut url = self.base(endpoint)?;
        url.path_segments_mut()
            .map_err(|_| ExamError::Unavailable)?
            .pop_if_empty()
            .extend(path.iter().copied());
        Ok(self
            .http
            .request(method, url)
            .header("X-Client", format!("isct-companion/{}", self.version)))
    }

    /// Wait out the spacing the site asks between request starts.
    async fn space(&self) {
        let mut last = self.last_start.lock().await;
        if let Some(last) = *last {
            tokio::time::sleep_until(last + SITE_SPACING).await;
        }
        *last = Some(Instant::now());
    }

    /// Send a site request whose answer has no body Core reads: any 2xx,
    /// 204 included, is success.
    pub async fn fire(&self, request: RequestBuilder) -> Result<(), ExamError> {
        self.space().await;
        let response = request.send().await.map_err(network)?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(ExamError::Rejected)
        }
    }

    pub async fn service<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
    ) -> Result<T, ExamError> {
        self.space().await;
        let response = request.send().await.map_err(network)?;
        if response.headers().contains_key("X-Vercel-Mitigated") {
            return Err(ExamError::Unavailable);
        }
        let status = response.status();
        if status.is_success() {
            return read(response).await;
        }
        let body: Option<ServiceError> = read(response).await.ok();
        let code = body.as_ref().and_then(|body| body.code.as_deref());
        match (status, code) {
            (StatusCode::UNAUTHORIZED, _) => Err(ExamError::Expired),
            (_, Some("auth.not_allowed")) => Err(ExamError::Denied),
            (_, Some("auth.not_registered")) => Err(ExamError::Consent),
            (_, Some("auth.email_taken")) => Err(ExamError::EmailTaken),
            (_, Some("server.not_found")) => Err(ExamError::Unsupported),
            (_, Some("upload.sharing_blocked" | "request.sharing_blocked")) => {
                Err(ExamError::SharingBlocked)
            }
            (_, Some("exam.report_duplicate")) => Err(ExamError::AlreadyReported),
            (_, Some("request.daily_limit")) => Err(ExamError::Quota),
            (_, Some("upload.already_registered")) => Err(ExamError::UploadRegistered),
            (_, Some("upload.duplicate_file")) => Err(ExamError::UploadDuplicate),
            (_, Some("upload.quota_daily" | "upload.quota_total")) => Err(ExamError::UploadQuota),
            (_, Some("upload.rollback_in_use")) => Err(ExamError::UploadInUse),
            (_, Some("upload.too_large" | "upload.stored_too_large")) => Err(ExamError::UploadSize),
            (_, Some("upload.subject_required")) => Err(ExamError::UploadSubject),
            (_, Some("upload.teacher_invalid")) => Err(ExamError::UploadTeacher),
            (_, Some("upload.year_invalid")) => Err(ExamError::UploadYear),
            (_, Some("upload.occasion_invalid")) => Err(ExamError::UploadOccasion),
            (_, Some("upload.occasion_number_invalid")) => Err(ExamError::UploadOccasionNumber),
            (_, Some("upload.role_invalid")) => Err(ExamError::UploadRole),
            (_, Some("upload.subject_or_teacher_not_found")) => Err(ExamError::UploadNotFound),
            (_, Some("upload.note_too_long")) => Err(ExamError::UploadNoteLength),
            (
                _,
                Some(
                    "upload.type_not_allowed"
                    | "upload.format_mismatch"
                    | "upload.file_empty"
                    | "upload.file_name_missing"
                    | "upload.file_missing",
                ),
            ) => Err(ExamError::UploadFile),
            (
                StatusCode::BAD_REQUEST,
                Some(
                    "common.input_invalid"
                    | "exam.report_reason_required"
                    | "course.invalid"
                    | "course.class_invalid",
                ),
            ) => Err(ExamError::InvalidInput),
            (StatusCode::BAD_REQUEST, Some(code)) if code.starts_with("upload.") => {
                Err(ExamError::InvalidInput)
            }
            (_, Some("rate.too_many")) => Err(ExamError::Unavailable),
            (StatusCode::NOT_FOUND, Some("course.not_found")) => Err(ExamError::SubjectNotFound),
            (StatusCode::NOT_FOUND, _) => Err(ExamError::NotFound),
            (StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS, None) => {
                Err(ExamError::Unavailable)
            }
            (_, Some("write_freeze" | "server.write_freeze" | "server.not_ready")) => {
                Err(ExamError::Unavailable)
            }
            (_, _) if crate::net::host_failing(status.as_u16()) => Err(ExamError::Unavailable),
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
        match body.ok().map(|error| error.error.message).as_deref() {
            Some(
                "TOKEN_EXPIRED" | "USER_DISABLED" | "USER_NOT_FOUND" | "INVALID_REFRESH_TOKEN",
            ) => Err(ExamError::Expired),
            _ if crate::net::host_failing(status.as_u16()) => {
                Err(ExamError::Unavailable)
            }
            Some(message)
                if message.starts_with("TOO_MANY_ATTEMPTS_TRY_LATER")
                    || message.starts_with("QUOTA_EXCEEDED") =>
            {
                Err(ExamError::Unavailable)
            }
            // Google's sign-in refused it, not ScienceTokyoExam: never read
            // as the service refusing what was sent (`refused_by_service`).
            _ => Err(ExamError::Identity),
        }
    }

    /// Stream a signed file URL into `path`, refusing more than `limit`
    /// bytes, the service's own limit for one material.
    pub async fn fetch_file(&self, url: Url, path: &Path, limit: u64) -> Result<(), ExamError> {
        if !is_file_url(&url) {
            return Err(ExamError::Response);
        }
        let mut response = self.http.get(url).send().await.map_err(network)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Err(ExamError::NotFound);
        }
        if !response.status().is_success() {
            return Err(ExamError::Unavailable);
        }
        if response.content_length().is_some_and(|len| len > limit) {
            return Err(ExamError::Response);
        }
        let mut stream = tokio::fs::File::create(path)
            .await
            .map_err(|_| ExamError::Storage)?;
        let mut written = 0;
        while let Some(chunk) = response.chunk().await.map_err(network)? {
            written += chunk.len() as u64;
            if written > limit {
                return Err(ExamError::Response);
            }
            stream
                .write_all(&chunk)
                .await
                .map_err(|_| ExamError::Storage)?;
        }
        stream.flush().await.map_err(|_| ExamError::Storage)
    }
}

#[derive(Deserialize)]
struct ServiceError {
    code: Option<String>,
}

/// Whether `url` is the storage host the service signs file URLs for.
pub(super) fn is_file_url(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url
            .host_str()
            .is_some_and(|host| host.ends_with(".r2.cloudflarestorage.com"))
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
