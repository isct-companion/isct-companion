//! In-app announcements: one file Isct-Data serves outside its index, read
//! while the app is in the foreground and on a refresh press, kept in one
//! cache row, and shown on Today (slotledger `44zhww`, `09vdww`, `y5jw4j`,
//! `gnba6h`).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::{
    data::{DataError, WorkspaceChangeSource},
    locale::{ContentLanguage, Localized},
    plan::{Gated, KeyEnd, Lease},
    sync_state::{self, Commit, Dataset, Key, Outcome, RunOutcome},
};
use chrono::{DateTime, FixedOffset, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
#[cfg(feature = "ts")]
use ts_rs::TS;

/// The file, read straight from the public repository: it is a few KB, so
/// the index would save nothing, and a conditional GET answers 304 when it
/// is unchanged.
pub(crate) const URL: &str =
    "https://raw.githubusercontent.com/isct-companion/Isct-Data/main/announcements/v1/announcements.json";
/// The file format this build reads; a new format gets a new path.
const FORMAT: u32 = 1;
/// The longest file Core accepts.
const LIMIT: usize = 256 * 1024;
/// Hosts a link may open: the school's and the app's own sites.
const LINK_HOSTS: [&str; 2] = ["isct.ac.jp", "isctcomp.app"];
/// The settings row that keeps the ids the person closed on this device.
/// It never syncs: each device closes what it shows, so a device never
/// drops a closing for a file it has not read yet.
pub(crate) const ANSWERED: &str = "answeredAnnouncements";

/// The shell's app version, which Core cannot know at build time: releases
/// stamp it into each native bundle.
static APP_VERSION: OnceLock<(u64, u64, u64)> = OnceLock::new();

/// Records the running app's version (`1.4.0`) once at startup.
pub fn set_app_version(version: &str) {
    if let Some(version) = parse_version(version) {
        let _ = APP_VERSION.set(version);
    }
}

pub(crate) fn key() -> Key {
    Key::new(Dataset::Announcements, format!("v{FORMAT}"))
}

/// How prominently an announcement is shown.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum AnnouncementLevel {
    Info,
    Important,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AnnouncementLink {
    pub url: String,
    pub label: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AnnouncementView {
    pub id: String,
    pub level: AnnouncementLevel,
    pub title: String,
    pub body: String,
    pub link: Option<AnnouncementLink>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AnnouncementsView {
    pub active: Vec<AnnouncementView>,
    pub pending: Vec<AnnouncementView>,
}

/// One announcement as the file publishes it.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Announcement {
    pub(crate) id: String,
    level: AnnouncementLevel,
    starts_at: DateTime<FixedOffset>,
    expires_at: DateTime<FixedOffset>,
    title: Localized,
    body: Localized,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    platforms: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    min_app_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_app_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    link: Option<Link>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Link {
    url: String,
    label: Localized,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct File {
    format_version: u32,
    announcements: Vec<Announcement>,
}

impl Announcement {
    /// Whether this build is one the announcement is for.
    fn addressed(&self) -> bool {
        let platform = self
            .platforms
            .as_ref()
            .is_none_or(|list| list.iter().any(|platform| platform == std::env::consts::OS));
        let versions = self.min_app_version.is_none() && self.max_app_version.is_none();
        // A bound Core cannot read addresses no version.
        let bound = |raw: &Option<String>, holds: &dyn Fn((u64, u64, u64)) -> bool| {
            raw.as_deref()
                .is_none_or(|raw| parse_version(raw).is_some_and(holds))
        };
        let version = versions
            || APP_VERSION.get().is_some_and(|running| {
                bound(&self.min_app_version, &|min| *running >= min)
                    && bound(&self.max_app_version, &|max| *running <= max)
            });
        platform && version
    }

    fn active_at(&self, now: DateTime<Utc>) -> bool {
        self.addressed() && self.starts_at <= now && now < self.expires_at
    }

    fn view(&self, language: ContentLanguage) -> AnnouncementView {
        let text = |text: &Localized| text.pick(language).unwrap_or_default().to_owned();
        AnnouncementView {
            id: self.id.clone(),
            level: self.level,
            title: text(&self.title),
            body: text(&self.body),
            link: self.link.as_ref().map(|link| AnnouncementLink {
                url: link.url.clone(),
                label: text(&link.label),
            }),
        }
    }
}

/// `1.4.0`: exactly three numbers. Releases carry no suffix.
fn parse_version(raw: &str) -> Option<(u64, u64, u64)> {
    let mut parts = raw.trim().split('.').map(|part| part.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

fn allowed_link(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|url| {
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.host_str().is_some_and(|host| {
                let host = host.to_ascii_lowercase();
                LINK_HOSTS
                    .iter()
                    .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")))
            })
    })
}

/// The file's announcements, or why it is not one Core reads. A link to
/// another site is dropped; a malformed announcement invalidates the file.
fn parse(body: &str) -> Result<Vec<Announcement>, String> {
    let file: File = serde_json::from_str(body).map_err(|error| error.to_string())?;
    if file.format_version != FORMAT {
        return Err(format!("format {} is not {FORMAT}", file.format_version));
    }
    let mut seen = std::collections::HashSet::new();
    let mut announcements = file.announcements;
    for item in &mut announcements {
        if item.id.trim().is_empty()
            || !seen.insert(item.id.clone())
            || item.starts_at >= item.expires_at
            || item.title.pick(ContentLanguage::En).is_none()
            || item.body.pick(ContentLanguage::En).is_none()
            || item.platforms.as_ref().is_some_and(|platforms| {
                platforms.is_empty()
                    || platforms.iter().any(|platform| {
                        !matches!(platform.as_str(), "macos" | "windows" | "ios" | "android")
                    })
            })
            || item
                .min_app_version
                .as_ref()
                .is_some_and(|raw| parse_version(raw).is_none())
            || item
                .max_app_version
                .as_ref()
                .is_some_and(|raw| parse_version(raw).is_none())
            || matches!(
                (
                    item.min_app_version.as_deref().and_then(parse_version),
                    item.max_app_version.as_deref().and_then(parse_version),
                ),
                (Some(min), Some(max)) if min > max
            )
        {
            return Err(format!("invalid announcement {}", item.id));
        }
        if item.link.as_ref().is_some_and(|link| {
            !allowed_link(&link.url) || link.label.pick(ContentLanguage::En).is_none()
        }) {
            item.link = None;
        }
    }
    Ok(announcements)
}

/// Both surfaces are derived from the same ordered set at the real wall time.
pub(crate) fn view(
    announcements: &[Announcement],
    workspace: &serde_json::Value,
    answered: &[String],
    now: DateTime<Utc>,
) -> AnnouncementsView {
    let language = ContentLanguage::of(workspace);
    let mut current: Vec<_> = announcements
        .iter()
        .filter(|announcement| announcement.active_at(now))
        .collect();
    current.sort_by(|a, b| b.starts_at.cmp(&a.starts_at).then(a.id.cmp(&b.id)));
    let active: Vec<_> = current
        .into_iter()
        .map(|item| item.view(language))
        .collect();
    let pending = active
        .iter()
        .filter(|item| !answered.contains(&item.id))
        .cloned()
        .collect();
    AnnouncementsView { active, pending }
}

pub(crate) fn next_boundary(
    announcements: &[Announcement],
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    announcements
        .iter()
        .filter(|item| item.addressed())
        .flat_map(|item| [item.starts_at.to_utc(), item.expires_at.to_utc()])
        .filter(|at| *at > now)
        .min()
}

/// The stored announcements; none before the first read.
pub(crate) fn stored(connection: &Connection) -> Result<Vec<Announcement>, DataError> {
    let data: Option<String> = connection
        .query_row(
            "SELECT data FROM announcements WHERE format=?1",
            [FORMAT],
            |row| row.get(0),
        )
        .optional()?;
    data.map_or(Ok(Vec::new()), |data| Ok(serde_json::from_str(&data)?))
}

/// Replaces the stored announcements. Returns 1 when they changed.
fn store(
    transaction: &Transaction<'_>,
    announcements: &[Announcement],
) -> Result<usize, DataError> {
    let data = serde_json::to_string(announcements)?;
    let current: Option<String> = transaction
        .query_row(
            "SELECT data FROM announcements WHERE format=?1",
            [FORMAT],
            |row| row.get(0),
        )
        .optional()?;
    if current.as_deref() == Some(data.as_str()) {
        return Ok(0);
    }
    transaction.execute(
        "INSERT INTO announcements(format,data) VALUES(?1,?2)
         ON CONFLICT(format) DO UPDATE SET data=excluded.data",
        params![FORMAT, data],
    )?;
    Ok(1)
}

/// Acknowledge only an active, addressed announcement. A later feed can
/// remove an announcement without changing the device's answers.
pub(crate) fn answering(
    connection: &Connection,
    current: &serde_json::Value,
    id: &str,
    now: DateTime<Utc>,
) -> Result<Option<serde_json::Value>, DataError> {
    let mut ids = current.as_array().cloned().unwrap_or_default();
    if ids.iter().any(|value| value.as_str() == Some(id))
        || !stored(connection)?
            .iter()
            .any(|item| item.id == id && item.active_at(now))
    {
        return Ok(None);
    }
    ids.push(id.into());
    Ok(Some(ids.into()))
}

/// How one GET ended.
enum Fetched {
    /// 304: the stored file is current.
    Unchanged,
    File {
        etag: Option<String>,
        body: String,
    },
}

/// The raw host answers the file itself; a redirect is no answer Core
/// follows, and shows as `http-3xx`.
async fn fetch(etag: Option<&str>) -> Result<Fetched, String> {
    let kind = |error: reqwest::Error| crate::net::classify(error).kind();
    let client = crate::net::client_builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(kind)?;
    let mut request = client
        .get(URL)
        .header(reqwest::header::ACCEPT, "application/json");
    if let Some(etag) = etag {
        request = request.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    let response = request.send().await.map_err(kind)?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(Fetched::Unchanged);
    }
    if !status.is_success() {
        // `http-429` in the run's row is how a rate limit shows.
        return Err(format!("http-{}", status.as_u16()));
    }
    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = crate::net::read_response_limited(response, LIMIT)
        .await
        .map_err(|error| error.kind())?;
    let body = String::from_utf8(bytes).map_err(|_| "parse".to_string())?;
    Ok(Fetched::File { etag, body })
}

/// Reads the leased announcements key: a conditional GET, then the file and
/// its freshness in one transaction.
pub(crate) async fn read(lease: &Lease) -> KeyEnd {
    let writer = lease.writer();
    let key = &lease.key;
    let known = writer.store().write(None, |transaction| {
        let state = sync_state::get(transaction, key)?;
        let has_cached_file = transaction
            .query_row(
                "SELECT 1 FROM announcements WHERE format=?1",
                [FORMAT],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        let etag = state
            .and_then(|state| state.validator)
            .filter(|_| has_cached_file);
        Ok::<_, DataError>(etag)
    });
    let etag = match known {
        Ok(known) => known,
        Err(_) => return KeyEnd::failed("storage", URL),
    };
    let fetched = match fetch(etag.as_deref()).await {
        Ok(fetched) => fetched,
        Err(kind) => return KeyEnd::failed(kind, URL),
    };
    if lease.is_cancelled() {
        return KeyEnd::Cancelled { source: URL.into() };
    }
    let (announcements, etag) = match fetched {
        Fetched::Unchanged if etag.is_some() => (None, etag),
        Fetched::Unchanged => return KeyEnd::failed("http-304", URL),
        Fetched::File { etag, body } => match parse(&body) {
            Ok(announcements) => (Some(announcements), etag),
            Err(reason) => return KeyEnd::failed_because("parse", URL, &reason),
        },
    };
    let now = crate::clock::now();
    let written = writer.write(
        Some(WorkspaceChangeSource::ReferencePull),
        |transaction| -> Result<usize, DataError> {
            let (changed, outcome) = match &announcements {
                Some(announcements) => (store(transaction, announcements)?, RunOutcome::Complete),
                None => (0, RunOutcome::Unchanged),
            };
            sync_state::commit(
                transaction,
                key,
                Commit {
                    validator: etag.as_deref(),
                    outcome: Outcome::Complete,
                    message: None,
                },
                now,
            )?;
            sync_state::record_run(
                transaction,
                &lease.run_record(outcome, changed, URL, None, now),
            )?;
            Ok(changed)
        },
    );
    match written {
        Ok(changed) => KeyEnd::Done { changed },
        Err(Gated::Stale) => KeyEnd::Stale,
        Err(Gated::Failed(_)) => KeyEnd::failed("storage", URL),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
#[path = "tests/announcements.rs"]
mod tests;
