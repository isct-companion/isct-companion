use crate::{
    data::DataStore,
    lms_files::{self, CourseKey, LmsResourceKind},
};
use serde_json::json;
use std::{fs, ops::RangeInclusive, path::Path, time::UNIX_EPOCH};
use tauri::{http, AppHandle, Manager};
use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reason {
    Login,
    Unresolved,
    TooLarge,
    Network,
    Invalid,
    Timeout,
}

impl Reason {
    fn name(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Unresolved => "unresolved",
            Self::TooLarge => "tooLarge",
            Self::Network => "network",
            Self::Invalid => "invalid",
            Self::Timeout => "timeout",
        }
    }
    fn status(self) -> http::StatusCode {
        match self {
            Self::Login => http::StatusCode::UNAUTHORIZED,
            Self::Unresolved => http::StatusCode::NOT_FOUND,
            Self::TooLarge => http::StatusCode::PAYLOAD_TOO_LARGE,
            Self::Invalid => http::StatusCode::BAD_REQUEST,
            Self::Timeout => http::StatusCode::GATEWAY_TIMEOUT,
            Self::Network => http::StatusCode::BAD_GATEWAY,
        }
    }
}

fn error(reason: Reason, bytes: Option<u64>) -> http::Response<Vec<u8>> {
    let mut body = json!({"reason":reason.name()});
    if let Some(bytes) = bytes {
        body["bytes"] = bytes.into();
    }
    response(reason.status(), "application/json; charset=utf-8")
        .header("X-Isct-Reason", reason.name())
        .body(body_bytes(&body))
        .unwrap_or_else(|_| http::Response::new(Vec::new()))
}

fn login_error(app: &AppHandle) -> http::Response<Vec<u8>> {
    // SchoolWebController is the sole owner of school-session ordering. A
    // burst of failed images/files is safe: its queue discards duplicate
    // Science session jobs and orders the accepted request behind active work.
    let controller = app.state::<crate::school_web::SchoolWebController>();
    let _ = controller.request_session(app, crate::school_auth::SchoolAuthProfile::Science);
    error(Reason::Login, None)
}

// Kept separate so the error builder above never accepts caller-provided headers.
fn body_bytes(value: &serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_default()
}

fn response(status: http::StatusCode, content_type: &str) -> http::response::Builder {
    http::Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, content_type)
        .header(
            http::header::CACHE_CONTROL,
            "private, max-age=0, must-revalidate",
        )
        .header(http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(http::header::ACCESS_CONTROL_ALLOW_METHODS, "GET, OPTIONS")
        .header(http::header::ACCESS_CONTROL_ALLOW_HEADERS, "Range")
}

fn preflight(request: &http::Request<Vec<u8>>) -> Option<http::Response<Vec<u8>>> {
    (request.method() == http::Method::OPTIONS).then(|| {
        response(http::StatusCode::NO_CONTENT, "text/plain; charset=utf-8")
            .body(Vec::new())
            .unwrap_or_else(|_| http::Response::new(Vec::new()))
    })
}

fn json_response(value: serde_json::Value) -> http::Response<Vec<u8>> {
    let body = body_bytes(&value);
    response(http::StatusCode::OK, "application/json; charset=utf-8")
        .header(http::header::CONTENT_LENGTH, body.len().to_string())
        .body(body)
        .unwrap_or_else(|_| http::Response::new(Vec::new()))
}

fn external_url(raw: &str) -> Option<Url> {
    Url::parse(raw).ok().filter(|url| {
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.port().is_none()
            && url.username().is_empty()
            && url.password().is_none()
    })
}

fn mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "txt" => "text/plain; charset=utf-8",
        "md" => "text/markdown; charset=utf-8",
        "html" | "htm" => "text/html; charset=utf-8",
        "json" => "application/json",
        "csv" => "text/csv; charset=utf-8",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

fn parse_range(
    value: Option<&http::HeaderValue>,
    size: u64,
) -> Result<Option<RangeInclusive<u64>>, ()> {
    let Some(value) = value.and_then(|value| value.to_str().ok()) else {
        return Ok(None);
    };
    let raw = value.strip_prefix("bytes=").ok_or(())?;
    if raw.contains(',') || size == 0 {
        return Err(());
    }
    let (start, end) = raw.split_once('-').ok_or(())?;
    let range = if start.is_empty() {
        let suffix = end.parse::<u64>().map_err(|_| ())?;
        if suffix == 0 {
            return Err(());
        }
        size.saturating_sub(suffix)..=size - 1
    } else {
        let start = start.parse::<u64>().map_err(|_| ())?;
        let end = if end.is_empty() {
            size - 1
        } else {
            end.parse::<u64>().map_err(|_| ())?.min(size - 1)
        };
        if start > end || start >= size {
            return Err(());
        }
        start..=end
    };
    Ok(Some(range))
}

fn serve(path: &Path, range: Option<&http::HeaderValue>, digest: &str) -> http::Response<Vec<u8>> {
    let Ok(metadata) = path.metadata() else {
        return error(Reason::Network, None);
    };
    let size = metadata.len();
    let Ok(requested) = parse_range(range, size) else {
        return http::Response::builder()
            .status(http::StatusCode::RANGE_NOT_SATISFIABLE)
            .header(http::header::CONTENT_RANGE, format!("bytes */{size}"))
            .body(Vec::new())
            .unwrap_or_else(|_| http::Response::new(Vec::new()));
    };
    let Ok(bytes) = fs::read(path) else {
        return error(Reason::Network, None);
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |value| value.as_secs());
    let etag = format!("\"{digest}-{size}-{modified}\"");
    let (status, body, content_range) = match requested {
        Some(range) => {
            let start = *range.start();
            let end = *range.end();
            (
                http::StatusCode::PARTIAL_CONTENT,
                bytes[start as usize..=end as usize].to_vec(),
                Some(format!("bytes {start}-{end}/{size}")),
            )
        }
        None => (http::StatusCode::OK, bytes, None),
    };
    let mut builder = response(status, mime(path))
        .header(http::header::CONTENT_LENGTH, body.len().to_string())
        .header(http::header::ACCEPT_RANGES, "bytes")
        .header(http::header::ETAG, etag);
    if let Some(value) = content_range {
        builder = builder.header(http::header::CONTENT_RANGE, value);
    }
    builder
        .body(body)
        .unwrap_or_else(|_| http::Response::new(Vec::new()))
}

fn app_url(
    request: &http::Request<Vec<u8>>,
    source: &str,
    course: &CourseKey,
    download: bool,
    foreground: bool,
) -> String {
    let http_origin = request.uri().scheme_str() == Some("http");
    let origin = if http_origin {
        "http://lmsfile.localhost"
    } else {
        "lmsfile://localhost"
    };
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query
        .append_pair("u", source)
        .append_pair("term", &course.term)
        .append_pair("course", &course.course);
    if download {
        query.append_pair("download", "1");
    }
    if foreground {
        query.append_pair("foreground", "1");
    }
    format!("{origin}/get?{}", query.finish())
}

fn request_input(
    request: &http::Request<Vec<u8>>,
) -> Result<(String, CourseKey, bool, bool), Reason> {
    if request.method() != http::Method::GET || request.uri().path() != "/get" {
        return Err(Reason::Invalid);
    }
    let query = request.uri().query().ok_or(Reason::Invalid)?;
    let values =
        url::form_urlencoded::parse(query.as_bytes()).collect::<std::collections::HashMap<_, _>>();
    let source = values
        .get("u")
        .map(|value| value.to_string())
        .ok_or(Reason::Invalid)?;
    let course = CourseKey {
        term: values
            .get("term")
            .map(|value| value.to_string())
            .ok_or(Reason::Invalid)?,
        course: values
            .get("course")
            .map(|value| value.to_string())
            .ok_or(Reason::Invalid)?,
    };
    lms_files::component(&course.term).map_err(|_| Reason::Invalid)?;
    lms_files::component(&course.course).map_err(|_| Reason::Invalid)?;
    Ok((
        source,
        course,
        values.get("download").is_some_and(|value| value == "1"),
        values.get("foreground").is_some_and(|value| value == "1"),
    ))
}

pub async fn handle(app: AppHandle, request: http::Request<Vec<u8>>) -> http::Response<Vec<u8>> {
    if let Some(response) = preflight(&request) {
        return response;
    }
    let Ok((source, course, explicit, foreground)) = request_input(&request) else {
        return error(Reason::Invalid, None);
    };
    let Ok((source_url, kind)) = lms_files::lms_resource_url(&source) else {
        return error(Reason::Invalid, None);
    };
    let store = app.state::<DataStore>();
    let stored = store.lms_material_for_url(&source).ok().flatten();
    let known = stored
        .as_ref()
        .and_then(|row| row.get("resourceUrl"))
        .and_then(serde_json::Value::as_str);
    let mut resolved = known.map(str::to_string);

    #[cfg(desktop)]
    if resolved.is_none() {
        if let LmsResourceKind::View { module, cmid } = &kind {
            let downloader = app.state::<crate::lms_downloader::Downloader>();
            match downloader.resolve(&app, &source_url, *cmid, module).await {
                Ok(crate::lms_downloader::ResolvedResource::File { url }) => {
                    if lms_files::lms_resource_url(&url).is_err() {
                        return error(Reason::Invalid, None);
                    }
                    let _ = store.update_lms_resource_url(*cmid, &source, &url);
                    resolved = Some(url);
                }
                Ok(crate::lms_downloader::ResolvedResource::External { url }) => {
                    return if external_url(&url).is_some() {
                        let _ = store.update_lms_resource_url(*cmid, &source, &url);
                        json_response(json!({"kind":"external","url":url}))
                    } else {
                        error(Reason::Invalid, None)
                    };
                }
                Ok(crate::lms_downloader::ResolvedResource::Folder { files }) => {
                    let files = files
                        .into_iter()
                        .filter_map(|file| {
                            lms_files::lms_resource_url(&file.url).ok().map(|_| {
                                json!({
                        "name":file.name,"url":app_url(&request,&file.url,&course,explicit,true)})
                            })
                        })
                        .collect::<Vec<_>>();
                    return if files.is_empty() {
                        error(Reason::Unresolved, None)
                    } else {
                        json_response(json!({"kind":"folder","files":files}))
                    };
                }
                Ok(crate::lms_downloader::ResolvedResource::Login)
                | Err(crate::lms_downloader::FetchError::Login) => return login_error(&app),
                Ok(crate::lms_downloader::ResolvedResource::Unresolved) => {
                    return error(Reason::Unresolved, None)
                }
                Err(crate::lms_downloader::FetchError::Invalid) => {
                    eprintln!("[lms-file] resolve rejected by allowlist");
                    return error(Reason::Invalid, None);
                }
                Err(crate::lms_downloader::FetchError::Timeout) => {
                    eprintln!("[lms-file] resolve timed out");
                    return error(Reason::Timeout, None);
                }
                Err(crate::lms_downloader::FetchError::Network) => {
                    eprintln!("[lms-file] resolve network failure");
                    return error(Reason::Network, None);
                }
            }
        }
    }
    if matches!(kind, LmsResourceKind::File) {
        resolved = Some(source.clone());
    }
    let Some(resolved) = resolved else {
        return error(Reason::Unresolved, None);
    };
    let Ok((url, LmsResourceKind::File)) = lms_files::lms_resource_url(&resolved) else {
        return external_url(&resolved).map_or_else(
            || error(Reason::Invalid, None),
            |url| json_response(json!({"kind":"external","url":url})),
        );
    };
    let Ok(path) = lms_files::material_path(&app, &course, &url) else {
        return error(Reason::Invalid, None);
    };
    if !path.is_file() {
        #[cfg(desktop)]
        {
            let limit = if explicit {
                None
            } else {
                store
                    .load_workspace()
                    .ok()
                    .and_then(|workspace| {
                        workspace
                            .pointer("/settings/autoDownloadLimitMb")
                            .and_then(serde_json::Value::as_u64)
                    })
                    .map(|mb| mb.clamp(1, 1024) * 1024 * 1024)
                    .or(Some(20 * 1024 * 1024))
            };
            match app
                .state::<crate::lms_downloader::Downloader>()
                .fetch(&app, &url, path.clone(), limit, foreground || explicit)
                .await
            {
                Ok(crate::lms_downloader::Fetched::Saved) => {}
                Ok(crate::lms_downloader::Fetched::TooLarge(bytes)) => {
                    return error(Reason::TooLarge, Some(bytes))
                }
                Err(crate::lms_downloader::FetchError::Login) => return login_error(&app),
                Err(crate::lms_downloader::FetchError::Invalid) => {
                    return error(Reason::Invalid, None)
                }
                Err(crate::lms_downloader::FetchError::Timeout) => {
                    return error(Reason::Timeout, None)
                }
                Err(fetch_error) => {
                    eprintln!("[lms-file] fetch failed: {fetch_error}");
                    return error(Reason::Network, None);
                }
            }
        }
        #[cfg(mobile)]
        return error(Reason::Network, None);
    }
    let _ = fs::File::options()
        .write(true)
        .open(&path)
        .and_then(|file| file.set_modified(std::time::SystemTime::now()));
    let material_name = lms_files::material_name(&url);
    let stem = Path::new(&material_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(&material_name);
    let digest = stem.rsplit_once('-').map_or("file", |(_, digest)| digest);
    serve(&path, request.headers().get(http::header::RANGE), digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_preflight_is_allowed() {
        let request = http::Request::builder()
            .method(http::Method::OPTIONS)
            .header(http::header::ACCESS_CONTROL_REQUEST_HEADERS, "range")
            .body(Vec::new())
            .unwrap();
        let response = preflight(&request).expect("OPTIONS is a preflight request");
        assert_eq!(response.status(), http::StatusCode::NO_CONTENT);
        assert_eq!(
            response
                .headers()
                .get(http::header::ACCESS_CONTROL_ALLOW_METHODS),
            Some(&http::HeaderValue::from_static("GET, OPTIONS"))
        );
        assert_eq!(
            response
                .headers()
                .get(http::header::ACCESS_CONTROL_ALLOW_HEADERS),
            Some(&http::HeaderValue::from_static("Range"))
        );
    }

    #[test]
    fn parses_single_ranges() {
        let value = http::HeaderValue::from_static("bytes=2-5");
        assert_eq!(parse_range(Some(&value), 10), Ok(Some(2..=5)));
        let suffix = http::HeaderValue::from_static("bytes=-3");
        assert_eq!(parse_range(Some(&suffix), 10), Ok(Some(7..=9)));
        let multiple = http::HeaderValue::from_static("bytes=0-1,4-5");
        assert!(parse_range(Some(&multiple), 10).is_err());
    }

    #[test]
    fn external_links_require_plain_https_origins() {
        assert!(external_url("https://example.edu/page").is_some());
        assert!(external_url("http://example.edu/page").is_none());
        assert!(external_url("https://user@example.edu/page").is_none());
        assert!(external_url("https://example.edu:8443/page").is_none());
    }

    #[test]
    fn serves_exact_partial_file_bytes() {
        let path = std::env::temp_dir().join(format!("lms-range-{}.pdf", uuid::Uuid::new_v4()));
        fs::write(&path, b"0123456789").unwrap();
        let range = http::HeaderValue::from_static("bytes=3-6");
        let response = serve(&path, Some(&range), "digest");
        assert_eq!(response.status(), http::StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            response.headers()[http::header::CONTENT_RANGE],
            "bytes 3-6/10"
        );
        assert_eq!(response.headers()[http::header::ACCEPT_RANGES], "bytes");
        assert_eq!(response.body(), b"3456");
        fs::remove_file(path).unwrap();
    }
}
