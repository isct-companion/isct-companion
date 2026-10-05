use reqwest::header::{ACCEPT, CONTENT_TYPE, LOCATION};
use std::time::Duration;
use url::Url;

pub(crate) const ORIGIN: &str = "https://syllabus.s.isct.ac.jp";
pub(crate) const MAX_BODY: usize = 3 * 1024 * 1024;
const MAX_REDIRECTS: usize = 3;

pub(crate) struct Page {
    pub url: Url,
    pub html: String,
}

fn validate_url(url: &Url, allowed_path_prefix: &str) -> Result<(), String> {
    if url.scheme() != "https"
        || url.host_str() != Some("syllabus.s.isct.ac.jp")
        || !url.username().is_empty()
        || url.password().is_some()
        || !url.path().starts_with(allowed_path_prefix)
    {
        return Err("Syllabus request is outside the public syllabus origin.".into());
    }
    Ok(())
}

pub(crate) async fn get(mut url: Url) -> Result<Page, String> {
    let allowed_path_prefix = if url.path() == "/courses" {
        "/courses".to_string()
    } else {
        url.path().to_string()
    };
    validate_url(&url, &allowed_path_prefix)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|_| "Syllabus client is unavailable.")?;

    for redirect_count in 0..=MAX_REDIRECTS {
        let mut response = client
            .get(url.clone())
            .header(ACCEPT, "text/html")
            .send()
            .await
            .map_err(|_| "Official syllabus is temporarily unavailable.")?;

        if response.status().is_redirection() {
            if redirect_count == MAX_REDIRECTS {
                return Err("Syllabus returned too many redirects.".into());
            }
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or("Syllabus redirect was invalid.")?;
            let next = url
                .join(location)
                .map_err(|_| "Syllabus redirect was invalid.")?;
            validate_url(&next, &allowed_path_prefix)?;
            url = next;
            continue;
        }
        if !response.status().is_success() {
            return Err(format!(
                "Official syllabus returned HTTP {}.",
                response.status().as_u16()
            ));
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
            .map_err(|_| "Syllabus response could not be read.")?
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
    unreachable!()
}
