//! Shared catalog acquisition and freshness policy.
use crate::course_query::{OfficialFilterOption, OfficialSelection};
use crate::sync_state::{self, Dataset, Key, KeyState, RunOutcome, RunRecord, Term};
use crate::{data, syllabus};
use chrono::{DateTime, Utc};
use futures_util::{stream::FuturesOrdered, StreamExt};

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error(transparent)]
    Data(#[from] data::DataError),
    #[error(transparent)]
    Net(#[from] crate::net::NetError),
    #[error("Sign in to 教務Web in Settings to search courses.")]
    SignInRequired,
    /// 教務Web shows its maintenance page; the school's notice.
    #[error("{0}")]
    Maintenance(String),
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
    let kind = match error {
        syllabus::SyllabusError::Network(_) => "network",
        syllabus::SyllabusError::SiteChanged(_) => "parse",
        syllabus::SyllabusError::InvalidInput(_) => "invalid-input",
    };
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

/// The official search result. The IDs address full official schedule rows
/// already in the shared store, so every shell can render them through
/// finder_query. Every match is returned at once; there is no next page.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogPage {
    pub ids: Vec<String>,
    pub next_cursor: Option<String>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CatalogFilters {
    pub controls: Option<Vec<OfficialSelection>>,
    pub recommended: Option<bool>,
    pub teacher: Option<String>,
    pub code: Option<String>,
    pub department: Option<String>,
    pub groups: Option<Vec<String>>,
    pub quarters: Option<Vec<u8>>,
    pub days: Option<Vec<u8>>,
    pub period_starts: Option<Vec<u8>>,
    pub language: Option<String>,
    pub requirement_groups: Option<Vec<String>>,
}

pub(crate) const KYOMU_SEARCH_PATH: &str =
    "/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_0_1.aspx";

/// Search the authenticated official catalogue, then persist its full timing
/// rows before returning IDs. Syllabus enriches these rows on demand.
///
/// Name, code (or the department's code prefix) and teacher go to the
/// school as search parameters; one search returns every row (up to the
/// school's 1000-row cap). The other filters are applied here to those rows,
/// and every returned row is stored, matching or not.
pub async fn kyomu_query(
    store: data::DataStore,
    client: &crate::net::kyomu::KyomuClient,
    query: String,
    year: u16,
    filters: Option<CatalogFilters>,
) -> Result<CatalogPage, CatalogError> {
    use crate::net::{kyomu::CatalogSearch, KeyOutcome, NetError};
    let filters = filters.unwrap_or_default();
    let departments: Vec<crate::course_query::FinderDepartment> =
        serde_json::from_str(include_str!("course_query/departments.json"))?;
    let department = filters
        .department
        .as_ref()
        .and_then(|scope| departments.iter().find(|d| d.scope.as_ref() == Some(scope)));
    if filters.department.is_some() && department.is_none() {
        return Err(CatalogError::InvalidInput(
            "Unknown course department.".into(),
        ));
    }
    let controls = filters.controls.clone().unwrap_or_default();
    if controls.len() > 40
        || controls.iter().any(|c| {
            c.id.len() > 100
                || c.value.len() > 100
                || !c.id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                || c.value.chars().any(char::is_control)
        })
    {
        return Err(CatalogError::InvalidInput(
            "Invalid official search controls.".into(),
        ));
    }
    let code = filters
        .code
        .as_deref()
        .or_else(|| department.map(|d| d.code_prefix.as_str()))
        .unwrap_or("");
    let word = if query.trim() == "." {
        ""
    } else {
        query.trim()
    };
    if filters
        .quarters
        .as_ref()
        .is_some_and(|values| values.iter().any(|q| !(1..=4).contains(q)))
        || filters
            .days
            .as_ref()
            .is_some_and(|values| values.iter().any(|day| !(1..=7).contains(day)))
        || filters
            .period_starts
            .as_ref()
            .is_some_and(|values| values.contains(&0))
        || filters
            .language
            .as_ref()
            .is_some_and(|language| !["ja", "en"].contains(&language.as_str()))
    {
        return Err(CatalogError::InvalidInput("Invalid course filters.".into()));
    }
    let search = CatalogSearch {
        code: code.into(),
        name: word.into(),
        teacher: filters.teacher.clone().unwrap_or_default(),
    };
    let rows = match client.search(year, &search).await {
        Ok(KeyOutcome::Complete(rows) | KeyOutcome::Partial { data: rows, .. }) => rows,
        Ok(KeyOutcome::SignInRequired) => return Err(CatalogError::SignInRequired),
        Ok(KeyOutcome::Unavailable(reason)) => {
            return Err(CatalogError::InvalidInput(reason.into()))
        }
        Err(error @ NetError::HostPaused { .. }) => {
            return Err(client
                .maintenance_notice()
                .map_or(CatalogError::Net(error), CatalogError::Maintenance))
        }
        Err(NetError::Invalid(message)) => return Err(CatalogError::InvalidInput(message)),
        Err(error) => return Err(error.into()),
    };
    // Captured public form metadata is device-local and never cloud user data.
    store.record_kyomu_search_options(
        year,
        &serde_json::to_value(&rows.search_options)?,
        Utc::now(),
    )?;
    // Controls name choices of this account's current form; one the form no
    // longer offers cannot be interpreted.
    let controls = controls
        .iter()
        .map(|control| {
            rows.search_options
                .iter()
                .find(|option| option.id == control.id && option.value == control.value)
                .ok_or_else(|| {
                    CatalogError::InvalidInput(
                        "This filter is not offered by the current official search form.".into(),
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    // Persistence uses the same field ownership merge as visible captures and
    // later syllabus detail refreshes.
    store.catalog_upsert_offerings(&rows.courses)?;
    Ok(CatalogPage {
        ids: rows
            .courses
            .iter()
            .filter(|row| official_match(row, &filters, &controls))
            .filter_map(|row| row["id"].as_str().map(str::to_string))
            .collect(),
        next_cursor: None,
        truncated: rows.truncated,
    })
}

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
fn control_match(row: &serde_json::Value, option: &OfficialFilterOption) -> bool {
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

/// Whether a returned row meets the filters the school was not asked for.
/// Within one family any choice may match; every family must match.
pub(crate) fn official_match(
    row: &serde_json::Value,
    filters: &CatalogFilters,
    controls: &[&OfficialFilterOption],
) -> bool {
    let kyomu = &row["kyomu"];
    let slots = weekly_slots(row);
    let level = row["code"]
        .as_str()
        .and_then(|code| code.chars().nth(5))
        .and_then(|digit| digit.to_digit(10))
        .map(|digit| (digit * 100).to_string());
    let wanted = |values: &Option<Vec<u8>>, test: &dyn Fn(u64) -> bool| {
        values
            .as_ref()
            .is_none_or(|values| values.is_empty() || values.iter().any(|v| test(u64::from(*v))))
    };
    let mut families: Vec<&str> = controls
        .iter()
        .map(|option| option.family.as_str())
        .collect();
    families.sort_unstable();
    families.dedup();
    filters.groups.as_ref().is_none_or(|groups| {
        groups.is_empty() || level.as_ref().is_some_and(|level| groups.contains(level))
    }) && wanted(&filters.quarters, &|quarter| has_quarter(row, quarter))
        && wanted(&filters.days, &|day| {
            slots.iter().any(|(slot, _)| *slot == day)
        })
        && wanted(&filters.period_starts, &|start| {
            slots.iter().any(|(_, first)| *first == start)
        })
        && filters
            .language
            .as_ref()
            .is_none_or(|language| row["instructionLanguage"] == language.as_str())
        && filters.requirement_groups.as_ref().is_none_or(|groups| {
            groups.is_empty()
                || kyomu["group"]
                    .as_str()
                    .is_some_and(|group| groups.iter().any(|wanted| wanted == group))
        })
        && (filters.recommended != Some(true)
            || kyomu["mark"].as_str().is_some_and(|mark| !mark.is_empty()))
        && families.iter().all(|family| {
            controls
                .iter()
                .filter(|option| option.family == *family)
                .any(|option| control_match(row, option))
        })
}
