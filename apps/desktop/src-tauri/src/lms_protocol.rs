use crate::lms_files::{self, CourseKey, Material};
use isct_core::course_materials::{MaterialError, MaterialFailure};
use serde_json::json;
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    ops::RangeInclusive,
    path::Path,
    time::UNIX_EPOCH,
};
use tauri::{http, AppHandle};

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
    // Core coalesces session checks from concurrent failed images/files.
    if let Ok(engine) = crate::core_ports::core(app).school_browser() {
        let _ = engine.request_session(isct_core::school_auth::SchoolAuthProfile::Science);
    }
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
    let Ok(mut file) = fs::File::open(path) else {
        return error(Reason::Network, None);
    };
    let Ok(metadata) = file.metadata() else {
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
    // Reading from the same handle as metadata also avoids a replacement race.
    // PDF/media range requests must not allocate/read the whole cached file.
    let (offset, length) = requested.as_ref().map_or((0, size), |range| {
        (*range.start(), range.end() - range.start() + 1)
    });
    let Ok(length) = usize::try_from(length) else {
        return error(Reason::Network, None);
    };
    let mut bytes = Vec::new();
    if bytes.try_reserve_exact(length).is_err() {
        return error(Reason::Network, None);
    }
    bytes.resize(length, 0);
    if file
        .seek(SeekFrom::Start(offset))
        .and_then(|_| file.read_exact(&mut bytes))
        .is_err()
    {
        return error(Reason::Network, None);
    }
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
                bytes,
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
    let materials = match lms_files::materials(&app) {
        Ok(service) => service,
        Err(_) => return error(Reason::Network, None),
    };
    let key = course.clone();
    let result = crate::core_ports::runtime(&app)
        .run(async move { materials.preview(key, source, explicit, foreground).await })
        .await;
    match result {
        Ok(Ok(Material::Ready { file })) => {
            let range = request.headers().get(http::header::RANGE).cloned();
            // Reading (possibly the whole file) blocks; keep it off the executor.
            tauri::async_runtime::spawn_blocking(move || {
                let path = Path::new(&file.path);
                let stem = path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or("file");
                let digest = stem.rsplit_once('-').map_or("file", |(_, digest)| digest);
                serve(path, range.as_ref(), digest)
            })
            .await
            .unwrap_or_else(|_| error(Reason::Network, None))
        }
        Ok(Ok(Material::Folder { files })) => {
            let files = files.into_iter().map(|file| {
                json!({"name":file.name,"url":app_url(&request,&file.url,&course,explicit,true)})
            }).collect::<Vec<_>>();
            if files.is_empty() {
                error(Reason::Unresolved, None)
            } else {
                json_response(json!({"kind":"folder","files":files}))
            }
        }
        Ok(Ok(Material::External { url })) => json_response(json!({"kind":"external","url":url})),
        Ok(Ok(Material::TooLarge { bytes })) => error(Reason::TooLarge, Some(bytes)),
        Ok(Ok(Material::Missing)) => error(Reason::Unresolved, None),
        Ok(Ok(Material::Failed { reason })) => match reason {
            MaterialFailure::Login => login_error(&app),
            MaterialFailure::Invalid => error(Reason::Invalid, None),
            MaterialFailure::Timeout => error(Reason::Timeout, None),
            MaterialFailure::Unresolved => error(Reason::Unresolved, None),
            MaterialFailure::Network | MaterialFailure::Cancelled => error(Reason::Network, None),
        },
        Ok(Err(MaterialError::Input(_))) => error(Reason::Invalid, None),
        _ => error(Reason::Network, None),
    }
}

#[cfg(test)]
#[path = "tests/lms_protocol.rs"]
mod tests;
