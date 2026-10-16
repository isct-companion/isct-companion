use super::SyllabusError;
use reqwest::header::{ACCEPT, CONTENT_TYPE, LOCATION};
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::{Mutex, Semaphore};
use tokio::time::Instant;
use url::Url;

/// Every request to the public syllabus shares one client (its connection
/// pool and keep-alive) and one limiter: at most `CONCURRENT` requests in
/// flight, and request starts at least `SPACING` apart, however many
/// commands are asking at once.
const CONCURRENT: usize = 6;
const SPACING: Duration = Duration::from_millis(60);
static PERMITS: Semaphore = Semaphore::const_new(CONCURRENT);
static NEXT_START: Mutex<Option<Instant>> = Mutex::const_new(None);
static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn request_error(error: reqwest::Error) -> SyllabusError {
    SyllabusError::Network(error.to_string())
}

fn client() -> Result<&'static reqwest::Client, SyllabusError> {
    if let Some(client) = CLIENT.get() {
        return Ok(client);
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .pool_idle_timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| SyllabusError::network("Syllabus client is unavailable."))?;
    Ok(CLIENT.get_or_init(|| client))
}

/// Waits for this request's turn to start.
async fn pace() {
    let mut next = NEXT_START.lock().await;
    let now = Instant::now();
    let start = next.map_or(now, |slot| slot.max(now));
    *next = Some(start + SPACING);
    drop(next);
    tokio::time::sleep_until(start).await;
}

pub(crate) const ORIGIN: &str = "https://syllabus.s.isct.ac.jp";
pub(crate) const MAX_BODY: usize = 3 * 1024 * 1024;
const MAX_REDIRECTS: usize = 3;

pub(crate) struct Page {
    pub url: Url,
    pub html: String,
}

fn validate_url(url: &Url, allowed_path_prefix: &str) -> Result<(), SyllabusError> {
    if url.scheme() != "https"
        || url.host_str() != Some("syllabus.s.isct.ac.jp")
        || !url.username().is_empty()
        || url.password().is_some()
        || !url.path().starts_with(allowed_path_prefix)
    {
        return Err(SyllabusError::invalid_input(
            "Syllabus request is outside the public syllabus origin.",
        ));
    }
    Ok(())
}

pub(crate) async fn get(mut url: Url) -> Result<Page, SyllabusError> {
    let allowed_path_prefix = if url.path() == "/courses" {
        "/courses".to_string()
    } else {
        url.path().to_string()
    };
    validate_url(&url, &allowed_path_prefix)?;
    let client = client()?;
    // One permit covers the request and its redirects.
    let _permit = PERMITS
        .acquire()
        .await
        .map_err(|_| SyllabusError::network("Syllabus client is unavailable."))?;

    for redirect_count in 0..=MAX_REDIRECTS {
        pace().await;
        let mut response = client
            .get(url.clone())
            .header(ACCEPT, "text/html")
            .send()
            .await
            .map_err(request_error)?;

        if response.status().is_redirection() {
            if redirect_count == MAX_REDIRECTS {
                return Err(SyllabusError::network(
                    "Syllabus returned too many redirects.",
                ));
            }
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or("Syllabus redirect was invalid.")?;
            let next = url
                .join(location)
                .map_err(|_| SyllabusError::network("Syllabus redirect was invalid."))?;
            validate_url(&next, &allowed_path_prefix)?;
            url = next;
            continue;
        }
        if !response.status().is_success() {
            return Err(SyllabusError::network(format!(
                "Official syllabus returned HTTP {}.",
                response.status().as_u16()
            )));
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !content_type.starts_with("text/html") {
            return Err("Official syllabus returned an unexpected content type.".into());
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_BODY as u64)
        {
            return Err("Syllabus response is too large.".into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| SyllabusError::network("Syllabus response could not be read."))?
        {
            if bytes.len() + chunk.len() > MAX_BODY {
                return Err("Syllabus response is too large.".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let html =
            String::from_utf8(bytes).map_err(|_| "Syllabus response was not valid UTF-8 text.")?;
        return Ok(Page { url, html });
    }
    Err(SyllabusError::network(
        "Syllabus returned too many redirects.",
    ))
}

#[cfg(test)]
#[path = "../tests/syllabus__transport.rs"]
mod tests;
