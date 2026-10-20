//! Catalog acquisition: the official 教務 department reads and シラバス
//! details as sync keys, and the stored offerings the finder searches. A
//! search never reads the network.
use crate::course_query::OfficialFilterOption;
use crate::net::{kyomu::KyomuClient, KeyOutcome, NetError};
use crate::sync_engine::{KeyEnd, Lease};
use crate::sync_state::{
    self, Commit, Dataset, Key, KeyState, Outcome, RunOutcome, RunRecord, Term,
};
use crate::{data, school_auth::SchoolAuthProfile, syllabus};
use chrono::{DateTime, Utc};
use futures_util::{stream::FuturesOrdered, StreamExt};

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error(transparent)]
    Data(#[from] data::DataError),
    #[error(transparent)]
    Syllabus(#[from] syllabus::SyllabusError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    InvalidInput(String),
}

/// Fetch and project one syllabus course for a shell editor.
pub async fn syllabus_course(
    store: &data::DataStore,
    id: String,
    language: String,
) -> Result<serde_json::Value, CatalogError> {
    if language != "en" && language != "ja" {
        return Err(CatalogError::InvalidInput(
            "Unsupported syllabus language.".into(),
        ));
    }
    let synced = catalog_sync_offering(store, id).await?;
    // The interface reads this detail now, so gc keeps it.
    store.write(None, |transaction| {
        sync_state::touch(transaction, &[detail_key(&synced.offering.id)], Utc::now())
    })?;
    let preferred = synced
        .content
        .iter()
        .find(|row| row.lang == language)
        .or_else(|| synced.content.first());
    let offering = serde_json::to_value(&synced.offering)?;
    let content = preferred
        .map(|row| serde_json::to_value(&row.content))
        .transpose()?;
    let workspace = store.load_workspace()?;
    let teaching_periods = crate::schedule::teaching_periods(&workspace)
        .map_err(|error| CatalogError::InvalidInput(error.to_string()))?;
    crate::workspace::catalog_course(&offering, &language, content.as_ref(), &teaching_periods)
        .map_err(|error| CatalogError::InvalidInput(error.to_string()))
}

pub async fn catalog_sync_offering(
    store: &data::DataStore,
    id: String,
) -> Result<syllabus::SyncOfferingResult, CatalogError> {
    let offering_id = if id.len() == 9 && id.chars().all(|value| value.is_ascii_digit()) {
        Some(id.clone())
    } else {
        url::Url::parse(&id)
            .ok()
            .and_then(|url| {
                url.query_pairs()
                    .find(|(k, _)| k == "jwc")
                    .map(|(_, v)| v.into_owned())
                    .or_else(|| url.path_segments()?.next_back().map(str::to_string))
            })
            .filter(|value| value.len() == 9 && value.chars().all(|c| c.is_ascii_digit()))
    };
    if let Some(offering_id) = offering_id.as_deref() {
        if let Some(stored) = stored_detail(store, offering_id)? {
            return Ok(stored);
        }
    }
    let run = uuid::Uuid::new_v4().to_string();
    let started = Utc::now();
    match syllabus::sync_offering(&id).await {
        Ok(synced) => store_detail(store, &run, started, synced),
        Err(error) => {
            if let Some(offering_id) = offering_id.as_deref() {
                record_detail_failure(store, &run, offering_id, &id, started, &error)?;
            }
            Err(error.into())
        }
    }
}

fn detail_key(offering_id: &str) -> Key {
    Key::new(Dataset::CatalogDetail, offering_id)
}

/// Whether an offering's syllabus detail should be read now: missing or
/// older than its TTL, and not backing off after a failure.
fn detail_due(state: Option<&KeyState>, now: DateTime<Utc>) -> bool {
    let ttl = sync_state::ttl(Dataset::CatalogDetail, Term::Current);
    sync_state::due(state, ttl, "", now).is_due()
}

/// The stored bilingual detail, when its key needs no read now.
fn stored_detail(
    store: &data::DataStore,
    offering_id: &str,
) -> Result<Option<syllabus::SyncOfferingResult>, CatalogError> {
    let state = store
        .catalog_key_states(&[detail_key(offering_id)])?
        .pop()
        .flatten();
    if detail_due(state.as_ref(), Utc::now()) {
        return Ok(None);
    }
    let Some(stored) = store.catalog_offering(offering_id)? else {
        return Ok(None);
    };
    let content = store
        .table_rows("offering_content")?
        .into_iter()
        .filter(|row| {
            row.get("offeringId").and_then(serde_json::Value::as_str) == Some(offering_id)
        })
        .map(serde_json::from_value)
        .collect::<Result<Vec<_>, _>>()?;
    if content.len() != 2 {
        return Ok(None);
    }
    Ok(Some(syllabus::SyncOfferingResult {
        offering: serde_json::from_value(stored)?,
        content,
        warnings: Vec::new(),
    }))
}

/// `host/path` of a syllabus locator, or `syllabus` for a bare offering id.
fn detail_source(locator: &str) -> String {
    url::Url::parse(locator)
        .map(|url| sync_state::source_of_url(&url))
        .unwrap_or_else(|_| "syllabus".into())
}

/// Stores one detail read and marks its `catalog.detail` key fresh in one
/// transaction.
fn store_detail(
    store: &data::DataStore,
    run: &str,
    started: DateTime<Utc>,
    mut synced: syllabus::SyncOfferingResult,
) -> Result<syllabus::SyncOfferingResult, CatalogError> {
    let offering = serde_json::to_value(&synced.offering)?;
    let content = synced
        .content
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?;
    let key = detail_key(&synced.offering.id);
    let source = detail_source(&synced.offering.syllabus_url);
    let merged = store.catalog_store_detail(
        &offering,
        &content,
        &RunRecord {
            run,
            key: &key,
            account: "",
            started_at: started,
            finished_at: Utc::now(),
            outcome: RunOutcome::Complete,
            changed: 1 + content.len(),
            requests: None,
            bytes: None,
            source: &source,
            error_kind: None,
        },
    )?;
    if let Some(merged) = merged {
        synced.offering = serde_json::from_value(merged)?;
    }
    Ok(synced)
}

/// A failed detail read: stored rows stay, the key backs off.
fn record_detail_failure(
    store: &data::DataStore,
    run: &str,
    offering_id: &str,
    locator: &str,
    started: DateTime<Utc>,
    error: &syllabus::SyllabusError,
) -> Result<(), CatalogError> {
    let key = detail_key(offering_id);
    let kind = detail_error_kind(error);
    let source = detail_source(locator);
    let finished = Utc::now();
    store.write(None, |transaction| {
        sync_state::fail(transaction, &key, "", kind, finished)?;
        sync_state::record_run(
            transaction,
            &RunRecord {
                run,
                key: &key,
                account: "",
                started_at: started,
                finished_at: finished,
                outcome: RunOutcome::Failed,
                changed: 0,
                requests: None,
                bytes: None,
                source: &source,
                error_kind: Some(kind),
            },
        )
    })?;
    Ok(())
}

/// Fetches detail pages for offerings whose `catalog.detail` key is due,
/// storing each one as it arrives.
async fn sync_stale_details(
    store: &data::DataStore,
    targets: Vec<(String, String)>,
    summary_only: bool,
) -> Result<(Vec<syllabus::SyncOfferingResult>, Vec<String>), CatalogError> {
    let started = Utc::now();
    let keys = targets
        .iter()
        .map(|(id, _)| detail_key(id))
        .collect::<Vec<_>>();
    let stale = targets
        .into_iter()
        .zip(store.catalog_key_states(&keys)?)
        .filter(|(_, state)| detail_due(state.as_ref(), started))
        .map(|(target, _)| target)
        .collect::<Vec<_>>();
    let run = uuid::Uuid::new_v4().to_string();
    let mut offerings = Vec::with_capacity(stale.len());
    let mut warnings = Vec::new();
    // All at once: the syllabus transport's limiter paces the requests.
    let mut handles = stale
        .iter()
        .map(|(_, url)| {
            let url = url.clone();
            async move {
                if summary_only {
                    syllabus::sync_offering_summary(&url).await
                } else {
                    syllabus::sync_offering(&url).await
                }
            }
        })
        .collect::<FuturesOrdered<_>>();
    // Each result is stored as soon as it (and those before it) arrive.
    for (id, url) in &stale {
        let Some(result) = handles.next().await else {
            break;
        };
        match result {
            Ok(synced) => offerings.push(store_detail(store, &run, started, synced)?),
            Err(error) => {
                record_detail_failure(store, &run, id, url, started, &error)?;
                warnings.push(format!("{id}: {error}"));
            }
        }
    }
    Ok((offerings, warnings))
}

pub async fn catalog_sync_department(
    store: &data::DataStore,
    year: u16,
    scope_path: String,
    quarter: Option<String>,
) -> Result<syllabus::DepartmentSyncResult, CatalogError> {
    let catalog = syllabus::department_catalog(year, &scope_path, quarter.as_deref()).await?;
    let targets = catalog
        .items
        .iter()
        .map(|item| (item.id.clone(), item.syllabus_url.clone()))
        .collect();
    let (offerings, warnings) = sync_stale_details(store, targets, false).await?;
    Ok(syllabus::DepartmentSyncResult {
        listed: catalog.items.len(),
        synced: offerings.len(),
        truncated: catalog.truncated,
        offerings,
        warnings,
    })
}

/// Detail pages for the given stored offerings (schedule, credits, format).
pub async fn catalog_sync_offerings(
    store: &data::DataStore,
    ids: Vec<String>,
) -> Result<Vec<String>, CatalogError> {
    if ids.len() > 20 {
        return Err(CatalogError::InvalidInput(
            "Too many offerings in one request.".into(),
        ));
    }
    let mut targets = Vec::new();
    let mut missing = Vec::new();
    for id in ids {
        // A course found by a search is stored as a search row until its
        // details have been read once.
        let url = [
            store.catalog_offering(&id)?,
            store.catalog_search_item(&id)?,
        ]
        .into_iter()
        .flatten()
        .find_map(|row| {
            row.get("syllabusUrl")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        });
        match url {
            Some(url) => targets.push((id, url)),
            // An LMS offering id is already an official syllabus locator;
            // a fresh install need not search the catalog before reading it.
            None if id.len() == 9 && id.bytes().all(|byte| byte.is_ascii_digit()) => {
                targets.push((id.clone(), id));
            }
            None => missing.push(format!("{id}: not in the catalog")),
        }
    }
    let (_, mut warnings) = sync_stale_details(store, targets, true).await?;
    warnings.extend(missing);
    Ok(warnings)
}

/// Stored offerings for one year, including search rows until details arrive.
pub fn offerings(
    store: &data::DataStore,
    year: u16,
) -> Result<Vec<serde_json::Value>, CatalogError> {
    Ok(store
        .catalog_search("")?
        .into_iter()
        .filter(|row| {
            row.get("year").and_then(serde_json::Value::as_u64) == Some(u64::from(year))
                && row.get("deletedAt").is_none_or(serde_json::Value::is_null)
        })
        .collect())
}

/// Offerings of one academic year in the course finder's normalized shape
/// (schedule, credits, language and format); rows that cannot be normalized
/// are left out.
pub fn finder_offerings(
    store: &data::DataStore,
    year: u16,
) -> Result<Vec<serde_json::Value>, CatalogError> {
    Ok(offerings(store, year)?
        .into_iter()
        .filter_map(crate::course_query::normalize_offering)
        .collect())
}

#[cfg(test)]
#[path = "tests/catalog.rs"]
mod tests;

pub(crate) const KYOMU_SEARCH_PATH: &str =
    "/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_0_1.aspx";

/// Weekly slots of a course row as (day, first period).
fn weekly_slots(row: &serde_json::Value) -> Vec<(u64, u64)> {
    row["schedule"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|slot| slot["kind"] == "weekly")
        .filter_map(|slot| Some((slot["day"].as_u64()?, slot["periods"][0].as_u64()?)))
        .collect()
}

fn has_quarter(row: &serde_json::Value, quarter: u64) -> bool {
    row["quarters"]
        .as_array()
        .is_some_and(|quarters| quarters.iter().any(|q| *q == format!("{quarter}Q")))
}

/// The leading number of a label such as `100番台` or `3限`.
fn leading_number(label: &str) -> Option<u64> {
    let digits: String = label.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Whether a row meets one choice of the official form, read from its label
/// the way the form means it. A choice whose meaning the row cannot show
/// (an unknown family, a category the grid does not list) keeps the row.
pub(crate) fn official_control_match(
    row: &serde_json::Value,
    option: &OfficialFilterOption,
) -> bool {
    const DAYS: &str = "月火水木金土日";
    let label = option.label.as_str();
    let kyomu = &row["kyomu"];
    let schedule_kind = |kind: &str| {
        row["schedule"]
            .as_array()
            .is_some_and(|slots| slots.iter().any(|slot| slot["kind"] == kind))
    };
    match option.family.as_str() {
        "level" => leading_number(label).is_none_or(|level| {
            row["code"]
                .as_str()
                .and_then(|code| code.chars().nth(5))
                .and_then(|digit| digit.to_digit(10))
                .is_some_and(|digit| u64::from(digit) * 100 == level)
        }),
        "quarter" => {
            let digits: Vec<u64> = label
                .chars()
                .filter_map(|c| c.to_digit(10))
                .filter(|digit| (1..=4).contains(digit))
                .map(u64::from)
                .collect();
            let quarters: Vec<u64> = match digits.as_slice() {
                [from, to] if label.contains(['~', '〜', '～', '-']) => (*from..=*to).collect(),
                _ => digits,
            };
            quarters
                .into_iter()
                .any(|quarter| has_quarter(row, quarter))
        }
        "delivery" => match DAYS.chars().position(|day| label == day.to_string()) {
            Some(index) => weekly_slots(row)
                .iter()
                .any(|(day, _)| *day == index as u64 + 1),
            None if label.contains("オンデマンド") => schedule_kind("on-demand"),
            None if label.contains("集中") => schedule_kind("intensive"),
            None if label.contains("申告専用") => schedule_kind("registration-only"),
            None => true,
        },
        "start" => leading_number(label)
            .is_none_or(|start| weekly_slots(row).iter().any(|(_, first)| *first == start)),
        "language" => {
            let lower = label.to_lowercase();
            if label.contains("日本語") || lower.contains("japanese") {
                row["instructionLanguage"] == "ja"
            } else if label.contains("英語") || lower.contains("english") {
                row["instructionLanguage"] == "en"
            } else {
                true
            }
        }
        "requirement" => kyomu["group"] == label,
        "recommended" => kyomu["mark"].as_str().is_some_and(|mark| !mark.is_empty()),
        "category" => kyomu["category"]
            .as_str()
            .filter(|category| !category.is_empty())
            .is_none_or(|category| category.contains(label) || label.contains(category)),
        _ => true,
    }
}

/// `catalog.official/{year}/{prefix}`: the whole department from the
/// official course search, merged into `offerings` with the official-field
/// ownership (教務 owns timing, credits and instructors), its freshness and
/// its run row in one transaction. A year the school does not offer yet is
/// recorded as unavailable; stored rows stay.
pub(crate) async fn read_official(client: &KyomuClient, lease: &Lease) -> KeyEnd {
    let source = format!("{}{}", crate::school_parser::kyomu::HOST, KYOMU_SEARCH_PATH);
    let Some((year, prefix)) = lease
        .key
        .key
        .split_once('/')
        .and_then(|(year, prefix)| Some((year.parse::<u16>().ok()?, prefix.to_string())))
    else {
        return KeyEnd::failed("invalid-key", source);
    };
    let (rows, outcome) = match client.catalog(year, &prefix).await {
        Ok(KeyOutcome::Complete(rows)) => (Some(rows), Outcome::Complete),
        Ok(KeyOutcome::Unavailable(_)) => (None, Outcome::Unavailable),
        // The search never splits a department; a split is incomplete.
        Ok(KeyOutcome::Partial { .. }) => return KeyEnd::failed("partial", source),
        Ok(KeyOutcome::SignInRequired) | Err(NetError::SignInRequired) => {
            return KeyEnd::SignInRequired {
                profile: SchoolAuthProfile::TokyoTech,
                source,
            }
        }
        Err(NetError::HostPaused { host, until_unix }) => {
            return KeyEnd::HostPaused {
                host,
                until: DateTime::from_timestamp(until_unix, 0).unwrap_or_else(Utc::now),
                notice: client.maintenance_notice(),
                source,
            }
        }
        Err(NetError::Cancelled) => return KeyEnd::Cancelled { source },
        Err(error) => return KeyEnd::failed(error.kind(), source),
    };
    if lease.is_cancelled() {
        return KeyEnd::Cancelled { source };
    }
    if rows.as_ref().is_some_and(|rows| rows.truncated) {
        log::warn!(target: "isct_core::catalog", "official catalog {} reached the search cap", lease.key);
    }
    let writer = lease.writer();
    let (key, account, run, started, record_source) = (
        lease.key.clone(),
        lease.account.clone(),
        lease.run.clone(),
        lease.started,
        source.clone(),
    );
    let applied = tokio::task::spawn_blocking(move || {
        let now = Utc::now();
        let committed = writer.write(None, |transaction| {
            let changed = match &rows {
                Some(rows) => {
                    data::DataStore::catalog_upsert_offerings_in(transaction, &rows.courses)?
                }
                None => 0,
            };
            sync_state::commit(
                transaction,
                &key,
                Commit {
                    account: &account,
                    validator: None,
                    outcome,
                    message: None,
                },
                now,
            )?;
            sync_state::record_run(
                transaction,
                &RunRecord {
                    run: &run,
                    key: &key,
                    account: &account,
                    started_at: started,
                    finished_at: now,
                    outcome: if outcome == Outcome::Unavailable {
                        RunOutcome::Unavailable
                    } else {
                        RunOutcome::Complete
                    },
                    changed,
                    requests: None,
                    bytes: None,
                    source: &record_source,
                    error_kind: None,
                },
            )?;
            Ok::<_, data::DataError>(changed)
        });
        // The form's filter choices are their own key, `{year}/options`.
        if let (Ok(_), Some(rows)) = (&committed, &rows) {
            if let (Some(_gate), Ok(options)) =
                (writer.enter(), serde_json::to_value(&rows.search_options))
            {
                if let Err(error) = writer
                    .store()
                    .record_kyomu_search_options(year, &options, now)
                {
                    crate::diagnostics::report("catalog", &error);
                }
            }
        }
        committed
    })
    .await;
    match applied {
        Ok(Ok(changed)) => KeyEnd::Done { changed },
        Ok(Err(gated)) => KeyEnd::from_gated(gated, "storage", &source),
        Err(_) => KeyEnd::failed("stopped", source),
    }
}

/// `catalog.detail/{offering}`: both languages of one シラバス page, stored
/// with the offering and the key's freshness in one transaction.
pub(crate) async fn read_detail(lease: &Lease) -> KeyEnd {
    let id = lease.key.key.clone();
    let store = lease.writer().store().clone();
    let lookup = id.clone();
    // A course found by a search is stored as a search row until its
    // details have been read once; a bare offering id is a locator too.
    let locator = tokio::task::spawn_blocking(move || {
        [
            store.catalog_offering(&lookup),
            store.catalog_search_item(&lookup),
        ]
        .into_iter()
        .filter_map(Result::ok)
        .flatten()
        .find_map(|row| {
            row.get("syllabusUrl")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
    })
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| id.clone());
    let source = detail_source(&locator);
    let synced = match syllabus::sync_offering(&locator).await {
        Ok(synced) => synced,
        Err(error) => return KeyEnd::failed(detail_error_kind(&error), source),
    };
    if lease.is_cancelled() {
        return KeyEnd::Cancelled { source };
    }
    let writer = lease.writer();
    let (run, started) = (lease.run.clone(), lease.started);
    let stored = tokio::task::spawn_blocking(move || {
        let _gate = writer.enter()?;
        Some(store_detail(writer.store(), &run, started, synced).map(|synced| synced.content.len()))
    })
    .await;
    match stored {
        Ok(Some(Ok(content))) => KeyEnd::Done {
            changed: 1 + content,
        },
        Ok(Some(Err(error))) => {
            log::warn!(target: "isct_core::catalog", "detail {id} not stored: {error}");
            KeyEnd::failed("storage", source)
        }
        Ok(None) => KeyEnd::Stale,
        Err(_) => KeyEnd::failed("stopped", source),
    }
}

fn detail_error_kind(error: &syllabus::SyllabusError) -> &'static str {
    match error {
        syllabus::SyllabusError::Network(_) => "network",
        syllabus::SyllabusError::SiteChanged(_) => "parse",
        syllabus::SyllabusError::InvalidInput(_) => "invalid-input",
    }
}
