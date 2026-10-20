//! `catalog.syllabus/{year}`: the シラバス catalog of one academic year, as
//! Isct-Data publishes it once for every user.
//!
//! The publisher runs `examples/export-catalog-pack.rs`, which reads every
//! unit of the year from シラバス and writes one gzip JSON pack; the pack's
//! canonical SHA-256 goes into Isct-Data's `index.json` under
//! `catalogs.{year}`. The client reads that entry, downloads the pack
//! (resuming with HTTP Range and If-Range when the host gives a strong
//! ETag), verifies the hash, holds back a pack that shrank suspiciously, and
//! imports it in one transaction through the catalog store's field
//! ownership, so 教務 fields stay.
use crate::data::{versioned::content_hash, DataError, DataStore};
use crate::net::{Body, Host, Http, Limits, NetError, NetRequest, Response};
use crate::reference_refresh::{suspicious_shrink, PUBLIC_INDEX, SUSPICIOUS_SHRINK};
use crate::sync_engine::{Gated, KeyEnd, Lease};
use crate::sync_state::{self, Commit, Dataset, Due, Key, Outcome, RunOutcome, RunRecord, Term};
use crate::syllabus::Offering;
use chrono::{DateTime, Datelike, FixedOffset, Utc};
use rusqlite::{Connection, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;
use url::Url;

pub const SCHEMA_VERSION: u32 = 1;
/// Largest compressed pack accepted.
const MAX_PACK_BYTES: usize = 32 * 1024 * 1024;
/// Largest decompressed pack JSON accepted.
const MAX_JSON_BYTES: u64 = 256 * 1024 * 1024;
const MAX_INDEX_BYTES: usize = 128 * 1024;
/// Bytes asked for per ranged request, so an interrupted download resumes.
const CHUNK: usize = 1024 * 1024;
const REQUEST_DEADLINE: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error("{0}")]
    Invalid(String),
    #[error("the pack does not match the hash index.json declares")]
    HashMismatch,
    #[error(transparent)]
    Net(#[from] NetError),
    #[error(transparent)]
    Data(#[from] DataError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

impl PackError {
    /// The short `sync_runs.error_kind` code; never the message.
    fn kind(&self) -> String {
        match self {
            Self::Invalid(_) | Self::Json(_) => "parse".into(),
            Self::HashMismatch => "hash-mismatch".into(),
            Self::Net(error) => error.kind(),
            Self::Data(_) => "storage".into(),
            Self::Io(_) => "io".into(),
        }
    }
}

impl From<String> for PackError {
    fn from(message: String) -> Self {
        Self::Invalid(message)
    }
}

/// Where one year's pack is published: `index.json` → `catalogs.{year}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackEntry {
    /// Path relative to `index.json`, e.g. `catalog/2026.json.gz`.
    pub source: String,
    /// SHA-256 of the decompressed pack as canonical JSON (sorted keys, no
    /// whitespace, UTF-8; whole numbers without a fraction), lowercase hex.
    pub hash: String,
    /// When the pack was generated (RFC 3339).
    pub ts: String,
    /// Offerings in the pack.
    pub count: usize,
}

/// The whole シラバス catalog of one academic year.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogPack {
    pub schema_version: u32,
    pub academic_year: u16,
    pub generated_at: String,
    pub source: PackSource,
    /// Every unit listing the exporter read, so a reviewer sees coverage.
    pub scopes: Vec<PackScope>,
    /// Ordered by id, each as one bilingual シラバス detail read stores it.
    pub offerings: Vec<Offering>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackSource {
    pub title: String,
    pub url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackScope {
    /// シラバス unit path, e.g. `4/0-904-342300-0-0`.
    pub scope: String,
    pub listed: usize,
    /// The listing hit the search cap even split by course level.
    pub truncated: bool,
    /// Listed offerings whose detail page the parser refused, left out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<String>,
}

impl CatalogPack {
    /// The gzip file and its canonical hash. The file is the JSON with
    /// sorted keys, compressed without a timestamp, so one pack always gives
    /// the same bytes.
    pub fn encode(&self) -> Result<(Vec<u8>, String), PackError> {
        let value = serde_json::to_value(self)?;
        let hash = content_hash(&value)?;
        let json = serde_json::to_vec(&value)?;
        let mut encoder = flate2::GzBuilder::new()
            .mtime(0)
            .write(Vec::new(), flate2::Compression::best());
        encoder.write_all(&json)?;
        Ok((encoder.finish()?, hash))
    }

    /// Reads a downloaded pack: decompressed within bounds, its canonical
    /// hash equal to the index entry's, of the expected year and schema.
    pub fn decode(bytes: &[u8], expected_hash: &str, year: u16) -> Result<Self, PackError> {
        let mut json = Vec::new();
        flate2::read::GzDecoder::new(bytes)
            .take(MAX_JSON_BYTES + 1)
            .read_to_end(&mut json)?;
        if json.len() as u64 > MAX_JSON_BYTES {
            return Err(NetError::TooLarge.into());
        }
        let value: Value = serde_json::from_slice(&json)?;
        if content_hash(&value)? != expected_hash.to_ascii_lowercase() {
            return Err(PackError::HashMismatch);
        }
        if value["schemaVersion"].as_u64() != Some(u64::from(SCHEMA_VERSION)) {
            return Err(format!("unsupported catalog pack schema {}", value["schemaVersion"]).into());
        }
        let pack: Self = serde_json::from_value(value)?;
        if pack.academic_year != year {
            return Err(format!("catalog pack is of {}, not {year}", pack.academic_year).into());
        }
        let mut ids = HashSet::with_capacity(pack.offerings.len());
        for offering in &pack.offerings {
            if offering.year != year || !ids.insert(offering.id.as_str()) {
                return Err(format!("catalog pack offering {} is invalid", offering.id).into());
            }
        }
        drop(ids);
        Ok(pack)
    }
}

/// `catalogs.{year}` of an Isct-Data index, or `None` when the year has no
/// pack. The catalogs sit beside `academicYears` so an index can list a
/// year's pack before that year's calendar exists.
pub fn index_entry(index: &Value, year: u16) -> Result<Option<PackEntry>, PackError> {
    let Some(raw) = index
        .get("catalogs")
        .and_then(|catalogs| catalogs.get(year.to_string()))
    else {
        return Ok(None);
    };
    let entry: PackEntry = serde_json::from_value(raw.clone())?;
    if entry.hash.len() != 64
        || !entry
            .hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("catalog hash must be a lowercase SHA-256 hex digest".to_string().into());
    }
    if entry.source.starts_with('/')
        || entry.source.contains("..")
        || entry.source.contains(['?', '#', '\\'])
        || !entry.source.ends_with(".json.gz")
    {
        return Err("catalog source must be a relative .json.gz path".to_string().into());
    }
    DateTime::parse_from_rfc3339(&entry.ts)
        .map_err(|_| "catalog ts must be RFC 3339".to_string())?;
    Ok(Some(entry))
}

pub(crate) fn pack_key(year: u16) -> Key {
    Key::new(Dataset::CatalogSyllabus, year.to_string())
}

/// The academic years Core keeps a catalog of without being asked: the
/// current one (April to March, Japan time) and the next.
pub(crate) fn watched_years(now: DateTime<Utc>) -> Vec<u16> {
    let today = FixedOffset::east_opt(9 * 3600)
        .map(|offset| now.with_timezone(&offset).date_naive())
        .unwrap_or_else(|| now.date_naive());
    let current = if today.month() >= 4 {
        today.year()
    } else {
        today.year() - 1
    };
    [current, current + 1]
        .into_iter()
        .filter_map(|year| u16::try_from(year).ok())
        .collect()
}

fn key_year(key: &Key) -> Result<u16, DataError> {
    key.key
        .parse()
        .map_err(|_| DataError::InvalidInput(format!("invalid catalog pack key {key}")))
}

/// Whether a pack key is read now: `sync_state::due`, and a fresh key whose
/// rows are gone, or that holds back a shrunk pack, is read again.
pub(crate) fn pack_due(
    connection: &Connection,
    key: &Key,
    now: DateTime<Utc>,
) -> Result<Due, DataError> {
    let state = sync_state::get(connection, key)?;
    let ttl = sync_state::ttl(Dataset::CatalogSyllabus, Term::Current);
    Ok(match sync_state::due(state.as_ref(), ttl, "", now) {
        Due::Fresh if sync_state::cursor(connection, key)?.is_some() => Due::Stale,
        Due::Fresh
            if state.as_ref().is_some_and(|state| state.validator.is_some())
                && DataStore::catalog_pack_rows(connection, key_year(key)?)? == 0 =>
        {
            Due::Missing
        }
        due => due,
    })
}

/// The stored pack is the published one: renew its key, but only while its
/// rows are stored (missing content beats validators).
pub(crate) fn renew(
    connection: &Connection,
    key: &Key,
    entry: &PackEntry,
    now: DateTime<Utc>,
) -> Result<bool, DataError> {
    let current = sync_state::get(connection, key)?
        .and_then(|state| state.validator)
        .is_some_and(|validator| validator == entry.hash);
    if !current
        || DataStore::catalog_pack_rows(connection, key_year(key)?)? == 0
        || !sync_state::renew(connection, key, "", now)?
    {
        return Ok(false);
    }
    sync_state::set_cursor(connection, key, "", None)?;
    Ok(true)
}

/// What one pack import changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PackImport {
    /// Offerings written (new, changed or back from missing).
    pub(crate) changed: usize,
    /// Offerings an earlier pack delivered that this one no longer lists.
    pub(crate) missing: usize,
}

/// How a verified pack was applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Applied {
    Imported(PackImport),
    /// Lost 30% or more of the stored rows: held until a later read returns
    /// the same pack.
    Held,
}

/// Applies a verified pack in the caller's transaction: the shrink check,
/// then every offering, the rows it no longer lists, the year's search
/// index and the key's freshness together.
pub(crate) fn import(
    transaction: &Transaction<'_>,
    key: &Key,
    entry: &PackEntry,
    pack: &CatalogPack,
    now: DateTime<Utc>,
) -> Result<Applied, DataError> {
    let year = key_year(key)?;
    let stored = DataStore::catalog_pack_rows(transaction, year)?;
    let held = sync_state::cursor(transaction, key)?;
    if suspicious_shrink(stored, pack.offerings.len())
        && held.as_deref() != Some(entry.hash.as_str())
    {
        sync_state::set_cursor(transaction, key, "", Some(&entry.hash))?;
        sync_state::fail(transaction, key, "", SUSPICIOUS_SHRINK, now)?;
        return Ok(Applied::Held);
    }
    let rows = pack
        .offerings
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?;
    let imported = DataStore::catalog_import_pack_in(transaction, year, &rows)?;
    sync_state::set_cursor(transaction, key, "", None)?;
    sync_state::commit(
        transaction,
        key,
        Commit {
            account: "",
            validator: Some(&entry.hash),
            outcome: Outcome::Complete,
            message: None,
        },
        now,
    )?;
    Ok(Applied::Imported(imported))
}

// -- Executor ----------------------------------------------------------------

/// Public data needs no school session, so packs keep their own client.
fn http() -> Result<&'static Http, PackError> {
    static HTTP: OnceLock<Http> = OnceLock::new();
    if let Some(http) = HTTP.get() {
        return Ok(http);
    }
    let http = Http::new()?;
    Ok(HTTP.get_or_init(|| http))
}

fn request(url: &Url, headers: Vec<(reqwest::header::HeaderName, String)>) -> NetRequest {
    NetRequest {
        host: Host::IsctData,
        method: reqwest::Method::GET,
        url: url.clone(),
        headers,
        body: Body::None,
        limits: Limits {
            max_bytes: MAX_PACK_BYTES,
            deadline: REQUEST_DEADLINE,
        },
    }
}

/// The year's index entry and the pack URL it names.
async fn fetch_entry(
    http: &Http,
    index_url: &str,
    year: u16,
) -> Result<Option<(Url, PackEntry)>, PackError> {
    let url = Url::parse(index_url).map_err(|_| format!("invalid index URL {index_url}"))?;
    let mut get = NetRequest::get(Host::IsctData, url.clone());
    get.limits.max_bytes = MAX_INDEX_BYTES;
    let response = http.send(get).await?;
    if response.status != 200 {
        return Err(NetError::Http(response.status).into());
    }
    let index: Value = serde_json::from_slice(&response.body)?;
    let Some(entry) = index_entry(&index, year)? else {
        return Ok(None);
    };
    let pack = url
        .join(&entry.source)
        .map_err(|_| format!("invalid catalog source {}", entry.source))?;
    Ok(Some((pack, entry)))
}

/// Bytes of a pack received so far, kept across leases of this process. A
/// part belongs to one pack hash and one strong ETag; anything else starts
/// over, so two versions are never joined.
struct Partial {
    hash: String,
    etag: Option<String>,
    bytes: Vec<u8>,
}

fn partials() -> std::sync::MutexGuard<'static, HashMap<u16, Partial>> {
    static PARTIALS: OnceLock<Mutex<HashMap<u16, Partial>>> = OnceLock::new();
    PARTIALS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// The received length and ETag of `year`'s part for `hash`.
fn resume_point(year: u16, hash: &str) -> (usize, Option<String>) {
    let mut partials = partials();
    let partial = partials.entry(year).or_insert_with(|| Partial {
        hash: hash.into(),
        etag: None,
        bytes: Vec::new(),
    });
    if partial.hash != hash {
        *partial = Partial {
            hash: hash.into(),
            etag: None,
            bytes: Vec::new(),
        };
    }
    (partial.bytes.len(), partial.etag.clone())
}

fn discard(year: u16) {
    partials().remove(&year);
}

/// Adds a received chunk; the whole file once `total` bytes are in.
fn append(
    year: u16,
    hash: &str,
    etag: Option<String>,
    chunk: &[u8],
    total: usize,
) -> Option<Vec<u8>> {
    let mut partials = partials();
    let partial = partials.get_mut(&year).filter(|part| part.hash == hash)?;
    partial.etag = etag;
    partial.bytes.extend_from_slice(chunk);
    if partial.bytes.len() < total {
        return None;
    }
    partials.remove(&year).map(|partial| partial.bytes)
}

/// A strong ETag; a weak one cannot guard a resumed range.
fn strong_etag(response: &Response) -> Option<String> {
    response
        .headers
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .filter(|etag| etag.starts_with('"'))
        .map(str::to_owned)
}

/// `Content-Range: bytes {start}-{end}/{total}`.
fn content_range(response: &Response) -> Option<(usize, usize, usize)> {
    let raw = response
        .headers
        .get(reqwest::header::CONTENT_RANGE)?
        .to_str()
        .ok()?;
    let (range, total) = raw.strip_prefix("bytes ")?.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    Some((start.parse().ok()?, end.parse().ok()?, total.parse().ok()?))
}

/// Downloads the pack in ranged chunks when the host supports it, else in
/// one request. Received chunks survive a cancelled or timed-out lease.
async fn download(
    http: &Http,
    url: &Url,
    year: u16,
    hash: &str,
    lease: &Lease,
) -> Result<Vec<u8>, PackError> {
    use reqwest::header::{IF_RANGE, RANGE};
    loop {
        if lease.is_cancelled() {
            return Err(NetError::Cancelled.into());
        }
        let (offset, etag) = resume_point(year, hash);
        let mut headers = vec![(RANGE, format!("bytes={offset}-{}", offset + CHUNK - 1))];
        if offset > 0 {
            let Some(etag) = etag.clone() else {
                discard(year);
                continue;
            };
            headers.push((IF_RANGE, etag));
        }
        let response = http.send(request(url, headers)).await?;
        match response.status {
            // No range support, or the file changed since the stored part:
            // this is the whole current file.
            200 => {
                discard(year);
                return Ok(response.body.to_vec());
            }
            206 => {
                let Some((start, end, total)) = content_range(&response) else {
                    return Err("ranged response without Content-Range".to_string().into());
                };
                let received = strong_etag(&response);
                if total > MAX_PACK_BYTES {
                    discard(year);
                    return Err(NetError::TooLarge.into());
                }
                let length = end.checked_sub(start).map(|last| last + 1);
                if start != offset
                    || length != Some(response.body.len())
                    || received.is_none()
                    || (offset > 0 && received != etag)
                {
                    // Nothing to resume against safely: read it whole.
                    discard(year);
                    let response = http.send(request(url, Vec::new())).await?;
                    if response.status != 200 {
                        return Err(NetError::Http(response.status).into());
                    }
                    return Ok(response.body.to_vec());
                }
                if let Some(whole) = append(year, hash, received, &response.body, total) {
                    return Ok(whole);
                }
            }
            // The stored part no longer fits the file.
            416 if offset > 0 => discard(year),
            status => return Err(NetError::Http(status).into()),
        }
    }
}

/// The ending of a read that failed before anything was written.
fn end_of(error: PackError, source: &str) -> KeyEnd {
    match error {
        PackError::Net(NetError::Cancelled) => KeyEnd::Cancelled {
            source: source.into(),
        },
        PackError::Net(NetError::HostPaused { host, until_unix }) => KeyEnd::HostPaused {
            host,
            until: DateTime::from_timestamp(until_unix, 0).unwrap_or_else(Utc::now),
            notice: None,
            source: source.into(),
        },
        error => KeyEnd::failed(error.kind(), source),
    }
}

/// Reads one leased `catalog.syllabus/{year}` key from Isct-Data.
pub(crate) async fn read_pack(lease: &Lease) -> KeyEnd {
    let index_url = lease
        .locator
        .clone()
        .unwrap_or_else(|| PUBLIC_INDEX.to_string());
    let Ok(year) = lease.key.key.parse::<u16>() else {
        return KeyEnd::failed("invalid-key", index_url);
    };
    let http = match http() {
        Ok(http) => http,
        Err(error) => return end_of(error, &index_url),
    };
    let found = match fetch_entry(http, &index_url, year).await {
        Ok(found) => found,
        Err(error) => return end_of(error, &index_url),
    };
    if lease.is_cancelled() {
        return KeyEnd::Cancelled { source: index_url };
    }
    let writer = lease.writer();
    let record = |transaction: &Transaction<'_>,
                  outcome: RunOutcome,
                  changed: usize,
                  source: &str,
                  now: DateTime<Utc>| {
        sync_state::record_run(
            transaction,
            &lease.run_record(outcome, changed, source, None, now),
        )
    };
    let Some((url, entry)) = found else {
        // Not published for this year (yet): stored rows stay, and the read
        // counts as current until the TTL asks again.
        let now = Utc::now();
        let committed = writer.write(None, |transaction| {
            sync_state::commit(
                transaction,
                &lease.key,
                Commit {
                    account: "",
                    validator: None,
                    outcome: Outcome::Unavailable,
                    message: None,
                },
                now,
            )?;
            record(transaction, RunOutcome::Unavailable, 0, &index_url, now)
        });
        return match committed {
            Ok(()) => KeyEnd::Done { changed: 0 },
            Err(gated) => KeyEnd::from_gated(gated, "storage", &index_url),
        };
    };
    let source = sync_state::source_of_url(&url);
    let now = Utc::now();
    let renewed = writer.write(None, |transaction| {
        let renewed = renew(transaction, &lease.key, &entry, now)?;
        if renewed {
            record(transaction, RunOutcome::Unchanged, 0, &source, now)?;
        }
        Ok::<_, DataError>(renewed)
    });
    match renewed {
        Ok(true) => return KeyEnd::Done { changed: 0 },
        Ok(false) => {}
        Err(gated) => return KeyEnd::from_gated(gated, "storage", &source),
    }
    let bytes = match download(http, &url, year, &entry.hash, lease).await {
        Ok(bytes) => bytes,
        Err(error) => return end_of(error, &source),
    };
    if lease.is_cancelled() {
        return KeyEnd::Cancelled { source };
    }
    let (key, run, started) = (lease.key.clone(), lease.run.clone(), lease.started);
    let record_source = source.clone();
    let applied = tokio::task::spawn_blocking(move || {
        let pack = match CatalogPack::decode(&bytes, &entry.hash, year) {
            Ok(pack) => pack,
            Err(error) => return Err(Gated::Failed(error)),
        };
        let size = u64::try_from(bytes.len()).ok();
        writer.write(None, |transaction| {
            let now = Utc::now();
            let applied = import(transaction, &key, &entry, &pack, now)?;
            let (outcome, changed, error_kind) = match applied {
                Applied::Imported(imported) => (
                    RunOutcome::Complete,
                    imported.changed + imported.missing,
                    None,
                ),
                Applied::Held => (RunOutcome::Failed, 0, Some(SUSPICIOUS_SHRINK)),
            };
            sync_state::record_run(
                transaction,
                &RunRecord {
                    run: &run,
                    key: &key,
                    account: "",
                    started_at: started,
                    finished_at: now,
                    outcome,
                    changed,
                    requests: None,
                    bytes: size,
                    source: &record_source,
                    error_kind,
                },
            )?;
            Ok::<_, PackError>(applied)
        })
    })
    .await;
    match applied {
        Ok(Ok(Applied::Imported(imported))) => KeyEnd::Done {
            changed: imported.changed + imported.missing,
        },
        Ok(Ok(Applied::Held)) => KeyEnd::Refused {
            kind: SUSPICIOUS_SHRINK.into(),
        },
        Ok(Err(Gated::Stale)) => KeyEnd::Stale,
        Ok(Err(Gated::Failed(error))) => {
            log::warn!(target: "isct_core::catalog", "catalog pack {year} not imported: {error}");
            end_of(error, &source)
        }
        Err(_) => KeyEnd::failed("stopped", source),
    }
}

#[cfg(test)]
#[path = "tests/catalog_pack.rs"]
mod tests;
