//! Shared catalog acquisition and freshness policy.
use crate::course_query::OfficialSelection;
use crate::{contracts, data, syllabus};
use futures_util::{stream::FuturesOrdered, StreamExt};

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error(transparent)]
    Data(#[from] data::DataError),
    #[error(transparent)]
    Pull(#[from] crate::pull_coordinator::PullError),
    #[error(transparent)]
    School(#[from] crate::school_browser::SchoolBrowserError),
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
    if let Some(offering_id) = offering_id {
        if let Some(stored) = store.catalog_offering(&offering_id)? {
            let cutoff = chrono::Utc::now()
                - chrono::Duration::seconds(contracts::SYLLABUS_STALE_SECONDS as i64);
            let fresh = ["ja", "en"].iter().all(|language| {
                stored
                    .pointer(&format!("/fetchedAt/{language}"))
                    .and_then(serde_json::Value::as_str)
                    .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                    .is_some_and(|value| value.with_timezone(&chrono::Utc) >= cutoff)
            });
            if fresh {
                let offering = serde_json::from_value(stored)?;
                let content = store
                    .table_rows("offering_content")?
                    .into_iter()
                    .filter(|row| {
                        row.get("offeringId").and_then(serde_json::Value::as_str)
                            == Some(offering_id.as_str())
                    })
                    .map(serde_json::from_value)
                    .collect::<Result<Vec<_>, _>>()?;
                if content.len() == 2 {
                    return Ok(syllabus::SyncOfferingResult {
                        offering,
                        content,
                        warnings: Vec::new(),
                    });
                }
            }
        }
    }
    let mut result = syllabus::sync_offering(&id).await?;
    let offering = serde_json::to_value(&result.offering)?;
    let content = result
        .content
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?;
    store.catalog_upsert_offerings(&[offering])?;
    store.upsert_offering_content(&content)?;
    if let Some(merged) = store.catalog_offering(&result.offering.id)? {
        result.offering = serde_json::from_value(merged)?;
    }
    Ok(result)
}

/// Fetches detail pages for offerings that have none or a stale one (older
/// than a week), storing each bounded batch as it arrives.
async fn sync_stale_details(
    store: &data::DataStore,
    targets: Vec<(String, String)>,
    summary_only: bool,
) -> Result<(Vec<syllabus::SyncOfferingResult>, Vec<String>), CatalogError> {
    let cutoff =
        chrono::Utc::now() - chrono::Duration::seconds(contracts::SYLLABUS_STALE_SECONDS as i64);
    let stale = targets
        .into_iter()
        .filter(|(id, _)| {
            let Ok(Some(stored)) = store.catalog_offering(id) else {
                return true;
            };
            stored.get("schedule").is_none()
                || (if summary_only {
                    &["en"][..]
                } else {
                    &["ja", "en"][..]
                })
                .iter()
                .any(|language| {
                    stored
                        .pointer(&format!("/fetchedAt/{language}"))
                        .and_then(serde_json::Value::as_str)
                        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                        .is_none_or(|value| value.with_timezone(&chrono::Utc) < cutoff)
                })
        })
        .collect::<Vec<_>>();
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
    for (id, _) in &stale {
        let Some(result) = handles.next().await else {
            break;
        };
        match result {
            Ok(mut synced) => {
                let offering = serde_json::to_value(&synced.offering)?;
                let content = synced
                    .content
                    .iter()
                    .map(serde_json::to_value)
                    .collect::<Result<Vec<_>, _>>()?;
                store.catalog_upsert_offerings(&[offering])?;
                store.upsert_offering_content(&content)?;
                if let Some(merged) = store.catalog_offering(&synced.offering.id)? {
                    synced.offering = serde_json::from_value(merged)?;
                }
                offerings.push(synced);
            }
            Err(error) => warnings.push(format!("{id}: {error}")),
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

/// One bounded page. The IDs address full official schedule rows already in
/// the shared store, so every shell can render them through finder_query.
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
pub async fn kyomu_query(
    store: data::DataStore,
    browser: std::sync::Arc<crate::school_browser::SchoolBrowserEngine>,
    coordinator: crate::pull_coordinator::PullCoordinator,
    query: String,
    year: u16,
    filters: Option<CatalogFilters>,
    cursor: Option<String>,
) -> Result<CatalogPage, CatalogError> {
    use crate::pull_coordinator::{PullOptions, PullTarget};
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
    if !(2020..=2200).contains(&year)
        || [word, code, filters.teacher.as_deref().unwrap_or("")]
            .iter()
            .any(|v| v.chars().count() > 200 || v.chars().any(char::is_control))
    {
        return Err(CatalogError::InvalidInput(
            "Invalid official course search.".into(),
        ));
    }
    let mut url = url::Url::parse(&format!(
        "https://kyomu0.gakumu.titech.ac.jp{KYOMU_SEARCH_PATH}"
    ))
    .map_err(|e| CatalogError::InvalidInput(e.to_string()))?;
    url.query_pairs_mut().extend_pairs([
        ("back", "0"),
        ("ZengakuSenmonAll", "True"),
        ("TermUnit", &format!("{year}2")),
        ("_search", "True"),
    ]);
    if filters.recommended == Some(true) {
        url.query_pairs_mut().append_pair("Suisho", "True");
    }
    if !code.is_empty() {
        url.query_pairs_mut().append_pair("ShinkokuNoS", code);
    }
    if !word.is_empty() {
        url.query_pairs_mut().append_pair("Name", word);
    }
    if let Some(teacher) = &filters.teacher {
        url.query_pairs_mut().append_pair("KyouinName", teacher);
    }
    // These quarter IDs are the values used by the official search form,
    // including combinations whose meetings overlap the requested quarter.
    if let Some(quarters) = &filters.quarters {
        if quarters.iter().any(|q| !(1..=4).contains(q)) {
            return Err(CatalogError::InvalidInput("Invalid course quarter.".into()));
        }
        let options: [(u8, &[u8]); 10] = [
            (1, &[1]),
            (2, &[2]),
            (3, &[3]),
            (4, &[4]),
            (7, &[1, 2]),
            (8, &[3, 4]),
            (9, &[1, 2, 3, 4]),
            (10, &[1, 3]),
            (12, &[2, 4]),
            (6, &[2, 3]),
        ];
        let values = options
            .iter()
            .filter(|(_, covered)| covered.iter().any(|q| quarters.contains(q)))
            .map(|(id, _)| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        if !values.is_empty() {
            url.query_pairs_mut().append_pair("Quarters", &values);
        }
    }
    for (key, values) in [
        (
            "KamokuCodeSeries",
            filters.groups.clone().unwrap_or_default(),
        ),
        (
            "HisshuKbn",
            filters.requirement_groups.clone().unwrap_or_default(),
        ),
        (
            "Youbi",
            filters
                .days
                .as_ref()
                .filter(|days| !days.contains(&6))
                .map(|days| {
                    days.iter()
                        .map(|d| if *d == 7 { 0 } else { *d })
                        .map(|d| d.to_string())
                        .collect()
                })
                .unwrap_or_default(),
        ),
        (
            "KaisiJigenS",
            filters
                .period_starts
                .as_ref()
                .filter(|periods| periods.iter().all(|p| (1..=10).contains(p)))
                .map(|periods| periods.iter().map(|p| p.to_string()).collect())
                .unwrap_or_default(),
        ),
    ] {
        if values
            .iter()
            .any(|v| v.len() > 16 || !v.bytes().all(|b| b.is_ascii_alphanumeric()))
            || values.len() > 12
        {
            return Err(CatalogError::InvalidInput("Invalid course filters.".into()));
        }
        if !values.is_empty() {
            url.query_pairs_mut().append_pair(key, &values.join(","));
        }
    }
    if let Some(language) = &filters.language {
        match language.as_str() {
            "ja" => {
                url.query_pairs_mut().append_pair("Lang_kj", "True");
            }
            "en" => {
                url.query_pairs_mut().append_pair("Lang_ke", "True");
            }
            _ => {
                return Err(CatalogError::InvalidInput(
                    "Invalid course language.".into(),
                ))
            }
        }
    }
    let url = url.to_string();
    let identity = serde_json::to_string(&controls)?;
    let browser_filters = filters.clone();
    let payload = coordinator
        .run(
            PullTarget::Catalog {
                year,
                query: format!("{url}#{identity}#{}", cursor.as_deref().unwrap_or("")),
            },
            PullOptions::manual(),
            move |context| {
                let browser = browser.clone();
                let url = url.clone();
                let cursor = cursor.clone();
                let browser_filters = browser_filters.clone();
                async move {
                    browser
                        .catalog_search(&url, cursor, browser_filters, &context)
                        .await
                        .map_err(crate::pull_coordinator::PullError::from)
                }
            },
        )
        .await?;
    if payload["searchOptions"].is_array() {
        // Captured public form metadata is device-local and never cloud user data.
        let options: Vec<crate::course_query::OfficialFilterOption> =
            serde_json::from_value(payload["searchOptions"].clone())?;
        let options = serde_json::to_value(options)?;
        let previous = store.reference_cache_marker("kyomu-search-options", i32::from(year))?;
        let now = chrono::Utc::now().timestamp();
        let changed = previous.as_ref().is_none_or(|p| {
            p["options"] != options
                || p["capturedAt"]
                    .as_i64()
                    .is_none_or(|at| now - at >= 30 * 24 * 60 * 60)
        });
        if changed {
            store.record_reference_cache(
                "kyomu-search-options",
                i32::from(year),
                serde_json::json!({"options":options,"capturedAt":now}),
            )?;
        }
    }
    let rows = payload["courses"].as_array().ok_or_else(|| {
        CatalogError::InvalidInput("Official course search returned no collection.".into())
    })?;
    if rows
        .iter()
        .any(|row| row["year"].as_u64() != Some(u64::from(year)))
    {
        return Err(CatalogError::InvalidInput("教務Web did not return the requested academic year. The current year is available after sign-in.".into()));
    }
    let mut items = Vec::new();
    for row in rows {
        if filters.groups.as_ref().is_some_and(|groups| {
            !groups.is_empty()
                && !groups.iter().any(|group| {
                    row["code"]
                        .as_str()
                        .and_then(|c| c.chars().nth(5))
                        .and_then(|c| c.to_digit(10))
                        .is_some_and(|level| group == &(level * 100).to_string())
                })
        }) {
            continue;
        }
        items.push(serde_json::from_value::<syllabus::CatalogSearchItem>(
            row.clone(),
        )?);
    }
    // Persistence uses the same field ownership merge as visible captures and
    // later syllabus detail refreshes.
    store.catalog_upsert_offerings(rows)?;
    Ok(CatalogPage {
        next_cursor: payload["nextCursor"].as_str().map(str::to_string),
        ids: items
            .into_iter()
            .map(|item: syllabus::CatalogSearchItem| item.id)
            .collect(),
        truncated: payload["truncated"].as_bool().unwrap_or(true),
    })
}
