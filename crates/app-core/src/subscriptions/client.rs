use super::{
    validate_academic_calendar, validate_class_schedules, validate_departments, validate_exam_feed,
    validate_important_dates, AcademicCalendar, ClassSchedules, Departments, ExamFeedKind,
    ImportantDates, PublishedExamFeed, SubscriptionError,
};
use crate::data::versioned::content_hash;
use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const MAX_INDEX_BYTES: usize = 128 * 1024;
pub(super) const MAX_DATA_BYTES: usize = 4 * 1024 * 1024;

// Index entries ignore keys they do not know, so publishing a new dataset
// kind does not break apps that predate it.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AcademicYearEntry {
    academic_calendar: IndexedSource,
    #[serde(default)]
    exams: Vec<IndexedSource>,
    #[serde(default)]
    exam_week_classes: Vec<IndexedSource>,
    #[serde(default)]
    important_dates: Option<IndexedSource>,
    #[serde(default)]
    class_schedules: Option<IndexedSource>,
}

#[derive(Clone, Debug, Deserialize)]
struct AdmissionYearEntry {
    departments: IndexedSource,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IndexedSource {
    source: String,
    hash: String,
    ts: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubscriptionIndex {
    #[serde(default)]
    academic_years: BTreeMap<String, AcademicYearEntry>,
    #[serde(default)]
    admission_years: BTreeMap<String, AdmissionYearEntry>,
    /// The campus map by format version; this app reads only `"1"`.
    #[cfg(feature = "campus-map")]
    #[serde(default)]
    campus_map: BTreeMap<String, IndexedSource>,
}

#[derive(Clone, Debug)]
pub(crate) struct ReferenceFetch<T> {
    pub value: Option<T>,
    pub(crate) data_url: String,
    pub(crate) remote_hash: String,
    pub(crate) source_ts: String,
}

impl<T> ReferenceFetch<T> {
    fn from_source(url: Url, source: &IndexedSource, value: Option<T>) -> Self {
        Self {
            value,
            data_url: url.to_string(),
            remote_hash: source.hash.clone(),
            source_ts: source.ts.clone(),
        }
    }

    fn validate<U>(
        self,
        validate: impl FnOnce(&T) -> Result<U, SubscriptionError>,
    ) -> Result<ReferenceFetch<U>, SubscriptionError> {
        Ok(ReferenceFetch {
            value: self.value.as_ref().map(validate).transpose()?,
            data_url: self.data_url,
            remote_hash: self.remote_hash,
            source_ts: self.source_ts,
        })
    }
}

fn validate_indexed_source(source: &IndexedSource) -> Result<(), SubscriptionError> {
    if source.hash.len() != 64 || !source.hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("subscription index hash must be a SHA-256 hex digest".into());
    }
    chrono::DateTime::parse_from_rfc3339(&source.ts)
        .map_err(|_| "subscription index ts must be RFC 3339".to_string())?;
    Ok(())
}

fn source_url(base: &Url, source: &IndexedSource) -> Result<Url, SubscriptionError> {
    validate_indexed_source(source)?;
    https_url(
        base.join(&source.source)
            .map_err(|_| "invalid subscription source path")?
            .as_str(),
    )
}

fn exam_manifest_hash(sources: &[IndexedSource]) -> Result<String, SubscriptionError> {
    content_hash(&Value::Array(
        sources
            .iter()
            .map(|source| json!({ "source": source.source, "hash": source.hash }))
            .collect(),
    ))
    .map_err(SubscriptionError::from)
}

fn verify_source_hash(raw: &str, expected: &str) -> Result<(), SubscriptionError> {
    let value: Value =
        serde_json::from_str(raw).map_err(|error| format!("invalid subscription JSON: {error}"))?;
    let actual = content_hash(&value)?;
    if actual != expected.to_ascii_lowercase() {
        return Err("subscription data does not match the hash declared by index.json".into());
    }
    Ok(())
}

pub(super) fn https_url(raw: &str) -> Result<Url, SubscriptionError> {
    let url = Url::parse(raw).map_err(|_| format!("invalid HTTPS URL: {raw}"))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
    {
        return Err(format!("invalid HTTPS URL: {raw}").into());
    }
    Ok(url)
}

fn exam_scope(path: &str, year: i32, kind: ExamFeedKind) -> Option<(String, String)> {
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() != 3
        || parts[0] != kind.directory()
        || !matches!(parts[1], "undergraduate" | "graduate")
    {
        return None;
    }
    let stem = parts[2].strip_suffix(".json")?;
    let (raw_year, quarter) = stem.split_once('-')?;
    if raw_year.parse::<i32>().ok()? != year || !matches!(quarter, "1q" | "2q" | "3q" | "4q") {
        return None;
    }
    Some((parts[1].to_string(), quarter.to_string()))
}

pub(crate) struct SubscriptionClient {
    client: Client,
}

impl SubscriptionClient {
    pub(crate) fn new() -> Result<Self, SubscriptionError> {
        let client = crate::net::client_builder()
            .timeout(std::time::Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()
            .map_err(SubscriptionError::from)?;
        Ok(Self { client })
    }
    /// Reads and validates the single repository entry point. Its raw hash is
    /// used to skip dataset requests when no metadata changed.
    pub(crate) async fn probe_index_source(
        &self,
        index_url: &str,
    ) -> Result<String, SubscriptionError> {
        let (_, _, hash) = self.fetch_index(index_url, None).await?;
        Ok(hash)
    }
    async fn get(&self, url: Url, limit: usize) -> Result<String, SubscriptionError> {
        let bytes = self.get_bytes(url, limit, "application/json").await?;
        String::from_utf8(bytes).map_err(|_| "subscription response is not UTF-8".into())
    }

    /// The response body as bytes, at most `limit` of them, over HTTPS only.
    async fn get_bytes(
        &self,
        url: Url,
        limit: usize,
        accept: &str,
    ) -> Result<Vec<u8>, SubscriptionError> {
        let mut response = self
            .client
            .get(url.clone())
            .header("Accept", accept)
            .send()
            .await
            .map_err(SubscriptionError::from)?;
        if !response.status().is_success() {
            return Err(SubscriptionError::Network(format!(
                "subscription GET failed: {}",
                response.status()
            )));
        }
        if response.url().scheme() != "https" {
            return Err("subscription redirect left HTTPS".into());
        }
        if let Some(length) = response.content_length() {
            if length as usize > limit {
                return Err("subscription response exceeds the size limit".into());
            }
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(SubscriptionError::from)? {
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err("subscription response exceeds the size limit".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    async fn fetch_index(
        &self,
        index_url: &str,
        expected_hash: Option<&str>,
    ) -> Result<(Url, SubscriptionIndex, String), SubscriptionError> {
        let base = https_url(index_url)?;
        let raw = self.get(base.clone(), MAX_INDEX_BYTES).await?;
        let actual = data_encoding::HEXLOWER.encode(&Sha256::digest(raw.as_bytes()));
        if let Some(expected) = expected_hash {
            if actual != expected.to_ascii_lowercase() {
                return Err("index.json changed while the refresh was running".into());
            }
        }
        let index = serde_json::from_str(&raw)
            .map_err(|error| format!("invalid subscription index: {error}"))?;
        Ok((base, index, actual))
    }

    async fn download_indexed_text(
        &self,
        url: Url,
        source: &IndexedSource,
    ) -> Result<String, SubscriptionError> {
        let raw = self.get(url, MAX_DATA_BYTES).await?;
        verify_source_hash(&raw, &source.hash)?;
        Ok(raw)
    }

    async fn fetch_indexed_text(
        &self,
        base: &Url,
        source: &IndexedSource,
        known_hash: Option<&str>,
    ) -> Result<ReferenceFetch<String>, SubscriptionError> {
        let url = source_url(base, source)?;
        let value = if known_hash == Some(source.hash.as_str()) {
            None
        } else {
            Some(self.download_indexed_text(url.clone(), source).await?)
        };
        Ok(ReferenceFetch::from_source(url, source, value))
    }

    pub(crate) async fn fetch_calendar(
        &self,
        index_url: &str,
        year: i32,
        expected_index_hash: Option<&str>,
        known_hash: Option<&str>,
    ) -> Result<ReferenceFetch<AcademicCalendar>, SubscriptionError> {
        let (base, index, _) = self.fetch_index(index_url, expected_index_hash).await?;
        let source = &index
            .academic_years
            .get(&year.to_string())
            .ok_or("calendar year is not published in this index")?
            .academic_calendar;
        self.fetch_indexed_text(&base, source, known_hash)
            .await?
            .validate(|raw| {
                let calendar = validate_academic_calendar(raw)?;
                if calendar.academic_year != year {
                    return Err("calendar year does not match the selection".into());
                }
                Ok(calendar)
            })
    }
    pub(crate) async fn fetch_departments(
        &self,
        index_url: &str,
        year: i32,
        expected_index_hash: Option<&str>,
        known_hash: Option<&str>,
    ) -> Result<ReferenceFetch<Departments>, SubscriptionError> {
        let (base, index, _) = self.fetch_index(index_url, expected_index_hash).await?;
        let source = &index
            .admission_years
            .get(&year.to_string())
            .ok_or("admission year is not published in this index")?
            .departments;
        self.fetch_indexed_text(&base, source, known_hash)
            .await?
            .validate(|raw| {
                let departments = validate_departments(raw)?;
                if departments.admission_year != year {
                    return Err("guide year does not match the selection".into());
                }
                Ok(departments)
            })
    }
    pub(crate) async fn fetch_important_dates(
        &self,
        index_url: &str,
        year: i32,
        expected_index_hash: Option<&str>,
        known_hash: Option<&str>,
    ) -> Result<ReferenceFetch<ImportantDates>, SubscriptionError> {
        let (base, index, _) = self.fetch_index(index_url, expected_index_hash).await?;
        let source = index
            .academic_years
            .get(&year.to_string())
            .and_then(|entry| entry.important_dates.as_ref())
            .ok_or("important dates are not published for this year")?;
        self.fetch_indexed_text(&base, source, known_hash)
            .await?
            .validate(|raw| validate_important_dates(raw, year))
    }

    /// A year without published class schedules yields an empty feed, so
    /// rows of a withdrawn file are marked missing instead of lingering.
    pub(crate) async fn fetch_class_schedules(
        &self,
        index_url: &str,
        year: i32,
        expected_index_hash: Option<&str>,
        known_hash: Option<&str>,
    ) -> Result<ReferenceFetch<ClassSchedules>, SubscriptionError> {
        let (base, index, _) = self.fetch_index(index_url, expected_index_hash).await?;
        let Some(source) = index
            .academic_years
            .get(&year.to_string())
            .ok_or("academic year is not published in this index")?
            .class_schedules
            .as_ref()
        else {
            let remote_hash = content_hash(&json!({"classSchedules": null, "year": year}))?;
            return Ok(ReferenceFetch {
                value: (known_hash != Some(remote_hash.as_str())).then(|| ClassSchedules {
                    schema_version: 1,
                    academic_year: year,
                    timezone: "Asia/Tokyo".into(),
                    source: Vec::new(),
                    entries: Vec::new(),
                }),
                data_url: index_url.to_string(),
                remote_hash,
                source_ts: String::new(),
            });
        };
        self.fetch_indexed_text(&base, source, known_hash)
            .await?
            .validate(|raw| validate_class_schedules(raw, year))
    }

    /// The campus map of format 2: its `.gz` bytes, checked against the
    /// index hash (which covers those bytes) and unpacked and parsed before
    /// anything is stored. An index without format 2, a bad hash or a file
    /// of another format fails the read and keeps the stored copy.
    #[cfg(feature = "campus-map")]
    pub(crate) async fn fetch_campus_map(
        &self,
        index_url: &str,
        expected_index_hash: Option<&str>,
        known_hash: Option<&str>,
    ) -> Result<ReferenceFetch<crate::campus_map::CampusMapFile>, SubscriptionError> {
        let (base, index, _) = self.fetch_index(index_url, expected_index_hash).await?;
        let source = index
            .campus_map
            .get(crate::campus_map::FORMAT_KEY)
            .ok_or("the campus map of format 2 is not published in this index")?;
        let url = source_url(&base, source)?;
        if known_hash == Some(source.hash.as_str()) {
            return Ok(ReferenceFetch::from_source(url, source, None));
        }
        let bytes = self
            .get_bytes(
                url.clone(),
                crate::campus_map::MAX_COMPRESSED_BYTES,
                "application/gzip",
            )
            .await?;
        let actual = data_encoding::HEXLOWER.encode(&Sha256::digest(&bytes));
        if actual != source.hash.to_ascii_lowercase() {
            return Err("the campus map does not match the hash declared by index.json".into());
        }
        let file = crate::campus_map::CampusMapFile::read(bytes, source.hash.clone())
            .map_err(|error| SubscriptionError::Validation(error.to_string()))?;
        Ok(ReferenceFetch::from_source(url, source, Some(file)))
    }

    pub(crate) async fn fetch_exams(
        &self,
        index_url: &str,
        year: i32,
        expected_index_hash: Option<&str>,
        known_hash: Option<&str>,
    ) -> Result<ReferenceFetch<Vec<PublishedExamFeed>>, SubscriptionError> {
        let (base, index, _) = self.fetch_index(index_url, expected_index_hash).await?;
        let entry = index
            .academic_years
            .get(&year.to_string())
            .ok_or("academic year is not published in this index")?;
        // Class rows share the exam subscription; with none published the
        // combined hash equals the exams-only hash older apps stored.
        let selected: Vec<_> = entry
            .exams
            .iter()
            .map(|source| (ExamFeedKind::Exams, source))
            .chain(
                entry
                    .exam_week_classes
                    .iter()
                    .map(|source| (ExamFeedKind::ExamWeekClasses, source)),
            )
            .collect();
        let sources: Vec<_> = selected
            .iter()
            .map(|(_, source)| (*source).clone())
            .collect();
        for source in &sources {
            validate_indexed_source(source)?;
        }
        let remote_hash = exam_manifest_hash(&sources)?;
        let source_ts = sources
            .iter()
            .map(|source| source.ts.as_str())
            .max()
            .unwrap_or_default()
            .to_string();
        if known_hash == Some(remote_hash.as_str()) {
            return Ok(ReferenceFetch {
                value: None,
                data_url: index_url.to_string(),
                remote_hash,
                source_ts,
            });
        }
        let mut feeds = Vec::new();
        for (kind, source) in selected {
            let (level, quarter) = exam_scope(&source.source, year, kind).ok_or_else(|| {
                format!("invalid exam path in subscription index: {}", source.source)
            })?;
            let url = source_url(&base, source)?;
            let raw = self.download_indexed_text(url.clone(), source).await?;
            let feed = validate_exam_feed(&raw, year, &quarter, kind)?;
            feeds.push(PublishedExamFeed {
                kind,
                level,
                quarter,
                url: url.to_string(),
                feed,
            });
        }
        Ok(ReferenceFetch {
            value: Some(feeds),
            data_url: index_url.to_string(),
            remote_hash,
            source_ts,
        })
    }
}

#[cfg(test)]
#[path = "../tests/subscriptions__client.rs"]
mod tests;
