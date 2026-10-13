#[cfg(test)]
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::collections::HashSet;
#[cfg(test)]
use std::fs::{self, OpenOptions};
#[cfg(test)]
use std::io::Write;
#[cfg(test)]
use std::path::Path;
use url::Url;

#[cfg(test)]
const MAX_RAW_BYTES: usize = 8 * 1024 * 1024;
#[cfg(test)]
const MAX_COURSES: usize = 500;
#[cfg(test)]
const MAX_ITEMS: usize = 5_000;

/// The resident school WebView still imports this historical symbol. Its
/// implementation now lives in the typed LMS ingestion module.
pub const T2SCHOLA_EXTRACT_SCRIPT: &str = crate::lms_ingest::LMS_EXTRACT_SCRIPT;

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedItem {
    pub external_id: String,
    pub title: String,
    pub url: String,
    pub kind: String,
    pub due_at: Option<String>,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedCourse {
    pub external_id: String,
    pub code: String,
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub items: Vec<ExtractedItem>,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedPayload {
    pub status: String,
    pub source_url: String,
    #[serde(default)]
    pub courses: Vec<ExtractedCourse>,
}

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StageState {
    Fresh,
    Cached,
    ReconnectRequired,
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub struct StagedSchoolData {
    pub state: StageState,
    pub payload: Option<ExtractedPayload>,
    pub message: &'static str,
}

/// Shared allowlist passed to Android's synchronous navigation delegate.
pub(crate) const SCHOOL_HOSTS: &[&str] = &[
    "isct.ex-tic.com",
    "portal.isct.ac.jp",
    "portal.titech.ac.jp",
    "lms.s.isct.ac.jp",
    "t2schola.titech.ac.jp",
    "syllabus.s.isct.ac.jp",
    "portal.nap.gsic.titech.ac.jp",
    "rp.nap.gsic.titech.ac.jp",
    "mailv4.m.titech.ac.jp",
    "kyomu0.gakumu.titech.ac.jp",
    "request.libra.titech.ac.jp",
    "portal.t4.gsic.titech.ac.jp",
    "www.titech.ac.jp",
];

pub fn sanitize_school_url(raw: &str) -> Option<String> {
    let mut url = Url::parse(raw).ok()?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    if !url
        .host_str()
        .is_some_and(|host| SCHOOL_HOSTS.contains(&host))
    {
        return None;
    }
    url.set_fragment(None);
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, value)| {
            let safe_identifier = value.len() <= 128
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
            (matches!(key.as_ref(), "id" | "course" | "page" | "section") && safe_identifier)
                // Syllabus course pages: `courses?jwc={9-digit offering id}`.
                || (key == "jwc" && value.len() == 9 && value.bytes().all(|b| b.is_ascii_digit()))
                || (key == "lang" && matches!(value.as_ref(), "ja" | "en"))
                || (key == "plang" && matches!(value.as_ref(), "jpn" | "eng"))
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    url.set_query(None);
    if !kept.is_empty() {
        url.query_pairs_mut().extend_pairs(kept);
    }
    Some(url.to_string())
}

#[cfg(test)]
fn valid_rfc3339(value: &str) -> bool {
    if value.len() < 20 || value.len() > 35 {
        return false;
    }
    let bytes = value.as_bytes();
    if bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return false;
    }
    let number = |start: usize, end: usize| {
        value
            .get(start..end)
            .and_then(|part| part.parse::<u32>().ok())
    };
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
        number(0, 4),
        number(5, 7),
        number(8, 10),
        number(11, 13),
        number(14, 16),
        number(17, 19),
    ) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    if day == 0 || day > max_day || hour > 23 || minute > 59 || second > 59 {
        return false;
    }
    let mut suffix = &value[19..];
    if let Some(fraction) = suffix.strip_prefix('.') {
        let split = fraction.find(['Z', '+', '-']).unwrap_or(fraction.len());
        if split == 0 || !fraction[..split].bytes().all(|byte| byte.is_ascii_digit()) {
            return false;
        }
        suffix = &fraction[split..];
    }
    if suffix == "Z" {
        return true;
    }
    let offset = suffix
        .strip_prefix('+')
        .or_else(|| suffix.strip_prefix('-'));
    let Some(offset) = offset else {
        return false;
    };
    if offset.len() != 5 || offset.as_bytes().get(2) != Some(&b':') {
        return false;
    }
    matches!((offset[0..2].parse::<u32>(), offset[3..5].parse::<u32>()), (Ok(h), Ok(m)) if h <= 23 && m <= 59)
}

#[cfg(test)]
pub fn stage_school_payload(raw: &str, cached: Option<ExtractedPayload>) -> StagedSchoolData {
    if raw.len() > MAX_RAW_BYTES {
        return StagedSchoolData {
            state: StageState::Cached,
            payload: cached,
            message: "School response exceeded the safe size; cached data was kept.",
        };
    }
    let mut parsed: ExtractedPayload = match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(_) => {
            return StagedSchoolData {
                state: StageState::Cached,
                payload: cached,
                message: "School response was invalid; cached data was kept.",
            }
        }
    };
    let Some(source_url) = sanitize_school_url(&parsed.source_url) else {
        return StagedSchoolData {
            state: StageState::Cached,
            payload: cached,
            message: "School response source was invalid; cached data was kept.",
        };
    };
    parsed.source_url = source_url;
    if parsed.status == "login" {
        return StagedSchoolData {
            state: StageState::ReconnectRequired,
            payload: cached,
            message: "The official site requested sign-in; cached data was kept.",
        };
    }
    if parsed.status != "ok" || parsed.courses.is_empty() || parsed.courses.len() > MAX_COURSES {
        return StagedSchoolData {
            state: StageState::Cached,
            payload: cached,
            message: "School response was empty or incomplete; cached data was kept.",
        };
    }
    let mut validated = parsed;
    let mut item_count = 0usize;
    let mut course_ids = HashSet::new();
    let mut item_ids = HashSet::new();
    for course in &mut validated.courses {
        course.title = course.title.trim().to_string();
        course.code = course.code.trim().to_string();
        let Some(url) = sanitize_school_url(&course.url) else {
            return StagedSchoolData {
                state: StageState::Cached,
                payload: cached,
                message: "School response contained an invalid course URL; cached data was kept.",
            };
        };
        course.url = url;
        if course.external_id.is_empty()
            || course.external_id.len() > 512
            || !course_ids.insert(course.external_id.clone())
            || course.title.is_empty()
            || course.title.len() > 1_000
        {
            return StagedSchoolData {
                state: StageState::Cached,
                payload: cached,
                message: "School course fields failed validation; cached data was kept.",
            };
        }
        item_count = item_count.saturating_add(course.items.len());
        for item in &mut course.items {
            let Some(url) = sanitize_school_url(&item.url) else {
                return StagedSchoolData {
                    state: StageState::Cached,
                    payload: cached,
                    message: "School response contained an invalid item URL; cached data was kept.",
                };
            };
            item.url = url;
            let due_valid = item.due_at.as_ref().is_none_or(|due| valid_rfc3339(due));
            if item.external_id.is_empty()
                || item.external_id.len() > 512
                || !item_ids.insert(item.external_id.clone())
                || item.title.is_empty()
                || item.title.len() > 1_000
                || !matches!(item.kind.as_str(), "assignment" | "material")
                || !due_valid
            {
                return StagedSchoolData {
                    state: StageState::Cached,
                    payload: cached,
                    message: "School item fields failed validation; cached data was kept.",
                };
            }
        }
    }
    if item_count > MAX_ITEMS {
        return StagedSchoolData {
            state: StageState::Cached,
            payload: cached,
            message: "School response contained too many items; cached data was kept.",
        };
    }
    StagedSchoolData {
        state: StageState::Fresh,
        payload: Some(validated),
        message: "School cache refreshed.",
    }
}

#[cfg(test)]
pub fn merge_school_payload(
    previous: Option<ExtractedPayload>,
    mut fresh: ExtractedPayload,
) -> ExtractedPayload {
    let Some(previous) = previous else {
        return fresh;
    };
    let mut prior_by_id: std::collections::HashMap<_, _> = previous
        .courses
        .into_iter()
        .map(|course| (course.external_id.clone(), course))
        .collect();
    for course in &mut fresh.courses {
        if let Some(prior) = prior_by_id.remove(&course.external_id) {
            let fresh_item_ids: HashSet<_> = course
                .items
                .iter()
                .map(|item| item.external_id.clone())
                .collect();
            course.items.extend(
                prior
                    .items
                    .into_iter()
                    .filter(|item| !fresh_item_ids.contains(&item.external_id)),
            );
        }
    }
    let fresh_ids: HashSet<_> = fresh
        .courses
        .iter()
        .map(|course| course.external_id.clone())
        .collect();
    fresh.courses.extend(
        prior_by_id
            .into_values()
            .filter(|course| !fresh_ids.contains(&course.external_id)),
    );
    fresh
}

#[cfg(test)]
pub fn publish_school_cache(path: &Path, payload: &ExtractedPayload) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "School cache path is invalid.".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|_| "School cache directory could not be created.".to_string())?;
    let bytes = serde_json::to_vec(payload)
        .map_err(|_| "School cache serialization failed.".to_string())?;
    if bytes.len() > MAX_RAW_BYTES {
        return Err("School cache is too large.".into());
    }
    let temporary = path.with_extension("json.tmp");
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|_| "School cache temporary file could not be opened.".to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "School cache write failed.".to_string())?;
    fs::rename(&temporary, path).map_err(|_| "School cache publication failed.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_and_empty_results_keep_cache() {
        let login = stage_school_payload(
            r#"{"status":"login","sourceUrl":"https://t2schola.titech.ac.jp/login","courses":[]}"#,
            None,
        );
        assert_eq!(login.state, StageState::ReconnectRequired);
        let empty = stage_school_payload(
            r#"{"status":"empty","sourceUrl":"https://t2schola.titech.ac.jp/","courses":[]}"#,
            None,
        );
        assert_eq!(empty.state, StageState::Cached);
    }

    #[test]
    fn syllabus_course_links_keep_their_offering_id() {
        assert_eq!(
            sanitize_school_url("https://syllabus.s.isct.ac.jp/courses?jwc=202602431&x=1")
                .as_deref(),
            Some("https://syllabus.s.isct.ac.jp/courses?jwc=202602431")
        );
        assert_eq!(
            sanitize_school_url("https://syllabus.s.isct.ac.jp/courses?jwc=javascript").as_deref(),
            Some("https://syllabus.s.isct.ac.jp/courses")
        );
    }
}
