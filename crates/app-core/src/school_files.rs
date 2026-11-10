//! Files a school page hands over to be saved: what the shell asks for, what
//! Core answers, and the name the file is saved under. The download itself is
//! `Downloader::save_school_file`.

use percent_encoding::percent_decode;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use url::Url;

const NAME_LIMIT: usize = 200;
/// A staged file the shell never collected (the app was killed between
/// Core's save and the shell's move) is removed after this long.
const STALE_STAGED: Duration = Duration::from_secs(24 * 60 * 60);

/// What a school page handed over: its URL, the page that linked it (sent as
/// the Referer to that page's own host) and what the engine read from the
/// response before handing it over.
#[derive(Clone, Debug, Default)]
pub struct SchoolFileRequest {
    pub url: String,
    pub referer: Option<String>,
    pub content_disposition: Option<String>,
    pub mime_type: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SchoolFile {
    /// Saved at `path` in the staging directory, under `name`.
    Saved {
        path: PathBuf,
        name: String,
        mime_type: String,
    },
    /// Not on a school host: Core fetches only school files, so the shell
    /// hands it to the system browser.
    External { url: String },
}

/// The name a school file is saved under. The response's
/// `Content-Disposition` names it whatever its type (`inline` included, as
/// Chrome saves it), RFC 6266 `filename*` over `filename`; otherwise the last
/// path segment of `url`, with the extension `mime_type` calls for.
pub fn file_name(disposition: Option<&[u8]>, url: &Url, mime_type: Option<&str>) -> String {
    let mime = mime_type
        .and_then(|value| value.split(';').next())
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if let Some(name) = disposition.and_then(disposition_name) {
        return plain_name(&name, &mime);
    }
    let segment = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .unwrap_or("");
    let decoded = percent_decode(segment.as_bytes()).decode_utf8_lossy();
    let name = match extensions(&mime) {
        Some(known) => {
            let (stem, extension) = split_extension(&decoded);
            let extension = extension.trim_start_matches('.');
            if known
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
            {
                decoded.into_owned()
            } else if stem.is_empty() {
                format!("download.{}", known[0])
            } else {
                format!("{stem}.{}", known[0])
            }
        }
        None => decoded.into_owned(),
    };
    plain_name(&name, &mime)
}

/// The filename a `Content-Disposition` value gives, if any. Header bytes
/// that are not UTF-8 are read as Latin-1, as HTTP defines them.
///
/// Parsed here rather than with the `content_disposition` crate: that crate
/// lets `filename` win over `filename*`, the reverse of RFC 6266, and panics
/// on some malformed values.
fn disposition_name(value: &[u8]) -> Option<String> {
    let text = match std::str::from_utf8(value) {
        Ok(text) => text.to_owned(),
        Err(_) => encoding_rs::WINDOWS_1252.decode(value).0.into_owned(),
    };
    let mut plain = None;
    let mut extended = None;
    for (name, value) in parameters(&text) {
        match name.as_str() {
            "filename*" => extended = extended.or_else(|| extended_value(&value)),
            "filename" => plain = plain.or(Some(value)),
            _ => {}
        }
    }
    extended
        .or_else(|| plain.map(|value| percent_decoded(&value)))
        .filter(|name| !name.trim().is_empty())
}

/// `name=value` parameters after the disposition type, names lowercased and
/// quoted strings unescaped.
fn parameters(text: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut rest = match text.split_once(';') {
        Some((_, rest)) => rest,
        None => return found,
    };
    loop {
        let Some((name, after)) = rest.split_once('=') else {
            return found;
        };
        let name = name.trim().to_ascii_lowercase();
        let after = after.trim_start();
        let (value, next) = if let Some(quoted) = after.strip_prefix('"') {
            let mut value = String::new();
            let mut characters = quoted.char_indices();
            let mut end = quoted.len();
            while let Some((index, character)) = characters.next() {
                match character {
                    '\\' => {
                        if let Some((_, escaped)) = characters.next() {
                            value.push(escaped);
                        }
                    }
                    '"' => {
                        end = index + 1;
                        break;
                    }
                    character => value.push(character),
                }
            }
            let tail = &quoted[end..];
            (value, tail.split_once(';').map_or("", |(_, next)| next))
        } else {
            match after.split_once(';') {
                Some((value, next)) => (value.trim().to_owned(), next),
                None => (after.trim().to_owned(), ""),
            }
        };
        found.push((name, value));
        if next.is_empty() {
            return found;
        }
        rest = next;
    }
}

/// An RFC 8187 `charset'language'value`, in UTF-8 or Latin-1.
fn extended_value(value: &str) -> Option<String> {
    let mut parts = value.splitn(3, '\'');
    let charset = parts.next()?.trim();
    let _language = parts.next()?;
    let bytes: Vec<u8> = percent_decode(parts.next()?.as_bytes()).collect();
    if charset.eq_ignore_ascii_case("utf-8") {
        String::from_utf8(bytes).ok()
    } else if charset.eq_ignore_ascii_case("iso-8859-1") {
        Some(encoding_rs::WINDOWS_1252.decode(&bytes).0.into_owned())
    } else {
        None
    }
}

/// A plain `filename` some servers percent-encode; kept as sent unless it
/// decodes to UTF-8.
fn percent_decoded(value: &str) -> String {
    if !value.contains('%') {
        return value.to_owned();
    }
    percent_decode(value.as_bytes())
        .decode_utf8()
        .map_or_else(|_| value.to_owned(), |decoded| decoded.into_owned())
}

/// Extensions for the types school sites serve, the usual one first. A short
/// table instead of `mime_guess`, whose reverse lookup has no preferred
/// extension per type.
fn extensions(mime: &str) -> Option<&'static [&'static str]> {
    Some(match mime {
        "application/pdf" => &["pdf"],
        "application/zip" | "application/x-zip-compressed" => &["zip"],
        "application/msword" => &["doc"],
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => &["docx"],
        "application/vnd.ms-excel" => &["xls"],
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => &["xlsx"],
        "application/vnd.ms-powerpoint" => &["ppt"],
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => &["pptx"],
        "text/plain" => &["txt"],
        "text/csv" => &["csv"],
        "image/png" => &["png"],
        "image/jpeg" => &["jpg", "jpeg"],
        "image/gif" => &["gif"],
        "video/mp4" => &["mp4"],
        _ => return None,
    })
}

/// A file name with no path, control characters or leading dots, at most 200
/// bytes, and a PDF ending in `.pdf`.
fn plain_name(suggested: &str, mime_type: &str) -> String {
    let cleaned = suggested
        .chars()
        .filter(|character| !character.is_control())
        .map(|character| {
            if matches!(character, '/' | '\\' | ':') {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    let cleaned = cleaned.trim().trim_start_matches('.').trim_start();
    let pdf = mime_type.eq_ignore_ascii_case("application/pdf");
    let mut name = match cleaned {
        "" if pdf => "download.pdf".to_string(),
        "" => "download".to_string(),
        cleaned => cleaned.to_string(),
    };
    if pdf && !name.to_ascii_lowercase().ends_with(".pdf") {
        name.push_str(".pdf");
    }
    if name.len() <= NAME_LIMIT {
        return name;
    }
    let (stem, extension) = split_extension(&name);
    let mut keep = NAME_LIMIT.saturating_sub(extension.len());
    while !stem.is_char_boundary(keep) {
        keep -= 1;
    }
    format!("{}{extension}", stem[..keep].trim_end())
}

fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(index) if index > 0 && name.len() - index <= 16 => name.split_at(index),
        _ => (name, ""),
    }
}

/// `name`, then `name (1)`, `name (2)`, … before the extension, for a folder
/// that already holds a file of that name.
pub(crate) fn name_candidates(name: &str) -> impl Iterator<Item = String> + '_ {
    let (stem, extension) = split_extension(name);
    std::iter::once(name.to_string())
        .chain((1..1000).map(move |number| format!("{stem} ({number}){extension}")))
}

/// Removes what an earlier process left in the staging directory: files the
/// shell never collected and partial downloads, once they are a day old.
pub(crate) fn sweep_staging(directory: &Path) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry.file_type().is_ok_and(|kind| kind.is_file())
            && entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age > STALE_STAGED);
        if stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
#[path = "tests/school_files.rs"]
mod tests;
