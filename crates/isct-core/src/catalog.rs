//! Shared catalog acquisition and freshness policy.
use crate::{contracts, data, syllabus};
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
    let preferred = synced
        .content
        .iter()
        .find(|row| row.lang == language)
        .or_else(|| synced.content.first());
    let offering = serde_json::to_value(&synced.offering)?;
    let content = preferred
        .map(|row| serde_json::to_value(&row.content))
        .transpose()?;
    crate::workspace::catalog_course(&offering, &language, content.as_ref())
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
            .and_then(|url| url.path_segments()?.next_back().map(str::to_string))
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
    let result = syllabus::sync_offering(&id).await?;
    let offering = serde_json::to_value(&result.offering)?;
    let content = result
        .content
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?;
    store.catalog_upsert_offerings(&[offering])?;
    store.upsert_offering_content(&content)?;
    Ok(result)
}

/// Search rows carry no schedule; they only fill gaps and never replace an
/// offering whose detail page was already stored.
fn store_search_items(
    store: &data::DataStore,
    items: &[syllabus::CatalogSearchItem],
) -> Result<(), CatalogError> {
    let mut rows = Vec::new();
    for item in items {
        if store.catalog_offering(&item.id)?.is_none() {
            rows.push(serde_json::to_value(item)?);
        }
    }
    if !rows.is_empty() {
        store.catalog_upsert_search_items(&rows)?;
    }
    Ok(())
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
            Ok(synced) => {
                let offering = serde_json::to_value(&synced.offering)?;
                let content = synced
                    .content
                    .iter()
                    .map(serde_json::to_value)
                    .collect::<Result<Vec<_>, _>>()?;
                store.catalog_upsert_offerings(&[offering])?;
                store.upsert_offering_content(&content)?;
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

/// One syllabus search, exactly as the official site answers it (at most 100
/// rows, no paging). Rows are stored without replacing fetched details.
pub async fn catalog_query(
    store: &data::DataStore,
    query: String,
    year: u16,
    filters: Option<syllabus::SyllabusFilters>,
) -> Result<syllabus::CatalogSearchResult, CatalogError> {
    let result = syllabus::search_bilingual(&query, year, 100, filters).await?;
    store_search_items(store, &result.items)?;
    Ok(result)
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
mod tests {
    use super::*;
    use serde_json::json;
    #[tokio::test]
    async fn cached_bilingual_details_preserve_wire_and_do_not_fetch() {
        let root = std::env::temp_dir().join(format!("isct-catalog-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let store = data::DataStore::initialize(root.join("school.db"))
            .unwrap()
            .0;
        let now = chrono::Utc::now().to_rfc3339();
        let value = json!({
            "id":"202602431", "year":2026, "quarters":["3Q"], "code":"TEST.1",
            "title":{"en":"Test"}, "instructors":[], "instructionLanguage":"en",
            "format":{"kinds":[]}, "media":{}, "credits":{"lecture":2,"exercise":0,"lab":0},
            "schedule":[], "academicUnit":{}, "intensive":false,
            "syllabusUrl":"https://example.invalid/202602431", "fetchedAt":{"ja":now,"en":now}
        });
        store.catalog_upsert_offerings(&[value]).unwrap();
        let content = ["ja","en"].map(|lang| json!({"offeringId":"202602431","lang":lang,"content":{
            "schemaVersion":1,"sourceUrl":"https://example.invalid/202602431","sourceLanguage":lang,
            "fetchedAt":now,"sourceUpdated":null,"metadata":[],"sections":[],"lecturePlan":[],"competencies":[]
        }}));
        store.upsert_offering_content(&content).unwrap();
        let result = catalog_sync_offering(&store, "https://example.invalid/202602431".into())
            .await
            .unwrap();
        assert_eq!(result.offering.id, "202602431");
        assert_eq!(result.content.len(), 2);
        assert!(result.warnings.is_empty());
        assert_eq!(offerings(&store, 2026).unwrap().len(), 1);
        assert!(offerings(&store, 2025).unwrap().is_empty());
        let warnings = catalog_sync_offerings(&store, vec!["202602431".into(), "missing".into()])
            .await
            .unwrap();
        assert_eq!(warnings, vec!["missing: not in the catalog"]);
        assert!(matches!(
            catalog_sync_offerings(&store, vec![String::new(); 21]).await,
            Err(CatalogError::InvalidInput(_))
        ));
        std::fs::remove_dir_all(root).unwrap();
    }
}
