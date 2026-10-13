//! One native reference pull path for manual checks and scheduled refreshes.
use crate::{
    data::DataStore,
    probe_reference_source_inner,
    pull_coordinator::{PullCoordinator, PullOptions, PullPriority, PullTarget},
    refresh_reference_source_inner,
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

const PUBLIC_INDEX: &str =
    "https://raw.githubusercontent.com/isct-companion/Isct-Data/main/index.json";
type Probes = Arc<Mutex<HashMap<String, Result<String, String>>>>;
fn timestamp(value: Option<&str>) -> Option<DateTime<Utc>> {
    value
        .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
        .map(|v| v.with_timezone(&Utc))
}
pub(crate) fn calendar_cache_present(workspace: &Value, year: i64) -> bool {
    // Subscription metadata survives backup/restore, but reference caches do
    // not. A saved hash cannot stand in for missing quarter boundaries.
    let boundaries = workspace
        .get("academicCalendar")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let kind = row.get("type").and_then(Value::as_str)?;
            let quarter = row.get("quarter").and_then(Value::as_str)?;
            let raw_date = row.get("date").and_then(Value::as_str)?;
            let valid_date = NaiveDate::parse_from_str(raw_date, "%Y-%m-%d")
                .is_ok_and(|date| date.format("%Y-%m-%d").to_string() == raw_date);
            (matches!(kind, "quarter-start" | "quarter-end")
                && valid_date
                && row.get("academicYear").and_then(Value::as_i64) == Some(year)
                && row.get("deletedAt").is_none_or(Value::is_null)
                && row.get("missingSince").is_none_or(Value::is_null))
            .then(|| format!("{kind}:{quarter}"))
        })
        .collect::<std::collections::HashSet<_>>();
    ["1Q", "2Q", "3Q", "4Q"].into_iter().all(|quarter| {
        boundaries.contains(&format!("quarter-start:{quarter}"))
            && boundaries.contains(&format!("quarter-end:{quarter}"))
    })
}

pub(crate) fn reference_is_fresh(row: &Value, year: i64, now: DateTime<Utc>) -> bool {
    row.get("year").and_then(Value::as_i64) == Some(year)
        && row.get("lastError").is_none_or(Value::is_null)
        && timestamp(row.get("fetchedAt").and_then(Value::as_str))
            .is_some_and(|last| now - last < Duration::hours(24))
}

fn cache_fingerprint(workspace: &Value, kind: &str, year: i32) -> Result<String, String> {
    let fields: &[&str] = match kind {
        "departments" => &["departments", "recommendedCourses"],
        "important-dates" => &["importantDates"],
        "exams" => &["exams"],
        _ => return Err("unsupported reference cache kind".into()),
    };
    let year_field = if kind == "departments" {
        "cohortYear"
    } else {
        "academicYear"
    };
    let mut rows: Vec<_> = fields
        .iter()
        .flat_map(|field| {
            workspace[*field]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|row| {
                    row[year_field].as_i64() == Some(year.into())
                        && row.get("deletedAt").is_none_or(Value::is_null)
                        && row.get("missingSince").is_none_or(Value::is_null)
                })
                .map(move |row| json!({"collection":field,"record":row}))
        })
        .collect();
    rows.sort_by_key(|row| format!("{}:{}", row["collection"], row["record"]["id"]));
    crate::data::versioned::content_hash(&json!(rows))
}

fn cache_present(
    store: &DataStore,
    workspace: &Value,
    request: &ReferenceRequest,
) -> Result<bool, String> {
    if request.kind == "calendar" {
        return Ok(calendar_cache_present(workspace, request.year.into()));
    }
    let Some(marker) = store.reference_cache_marker(&request.kind, request.year)? else {
        return Ok(false);
    };
    Ok(marker["indexUrl"].as_str() == Some(&request.index_url)
        && marker["fingerprint"].as_str()
            == Some(&cache_fingerprint(workspace, &request.kind, request.year)?))
}

fn same_index(saved: Option<&str>, requested: &str) -> bool {
    saved
        .and_then(|saved| {
            let mut url = url::Url::parse(saved.trim()).ok()?;
            url.set_fragment(None);
            Some(url.to_string() == requested)
        })
        .unwrap_or(false)
}

#[derive(Clone)]
pub(crate) struct ReferenceRequest {
    pub kind: String,
    pub index_url: String,
    pub year: i32,
    pub force: bool,
    pub expected_index_hash: Option<String>,
}

pub(crate) async fn refresh_one(
    app: &tauri::AppHandle,
    mut request: ReferenceRequest,
    priority: PullPriority,
    probes: Probes,
) -> Result<Value, String> {
    if let PullTarget::Reference {
        kind, index_url, ..
    } = (PullTarget::Reference {
        kind: request.kind.clone(),
        index_url: request.index_url.clone(),
        year: request.year,
    })
    .normalized().map_err(|error| error.to_string())?
    {
        request.kind = kind;
        request.index_url = index_url;
    }
    let coordinator = app.state::<PullCoordinator>().inner().clone();
    let app = app.clone();
    let target = PullTarget::Reference {
        kind: request.kind.clone(),
        index_url: request.index_url.clone(),
        year: request.year,
    };
    let mut options = if priority == PullPriority::Manual {
        PullOptions::manual()
    } else {
        PullOptions::automatic()
    };
    options.priority = priority;
    coordinator
        .run(target, options, move |context| {
            let app = app.clone();
            let request = request.clone();
            let probes = probes.clone();
            async move { run_reference(&app, &request, &context, &probes).await.map_err(crate::pull_coordinator::PullError::Execution) }
        })
        .await.map_err(|error| error.to_string())
}

async fn run_reference(
    app: &tauri::AppHandle,
    request: &ReferenceRequest,
    context: &crate::pull_coordinator::PullContext,
    probes: &Probes,
) -> Result<Value, String> {
    if !["calendar", "departments", "important-dates", "exams"].contains(&request.kind.as_str()) {
        return Err("unsupported subscription type".into());
    }
    let store = app.state::<DataStore>().inner().clone();
    // Read revisions inside the shared job. Each coalesced request commits once.
    let workspace = store.load_workspace()?;
    let existing = workspace["subscriptions"].as_array().and_then(|rows| {
        rows.iter().find(|row| {
            row["kind"].as_str() == Some(&request.kind)
                && row.get("deletedAt").is_none_or(Value::is_null)
        })
    });
    let priority = context.priority();
    if priority == PullPriority::Automatic
        && existing.is_some_and(|row| row["enabled"].as_bool() == Some(false))
    {
        return Ok(json!({"changed":0,"skipped":true,"notModified":true}));
    }
    let same_source =
        existing.is_some_and(|row| same_index(row["url"].as_str(), &request.index_url));
    let has_cache = cache_present(&store, &workspace, request)?;
    if !request.force
        && priority != PullPriority::Manual
        && same_source
        && has_cache
        && existing.is_some_and(|row| reference_is_fresh(row, request.year.into(), Utc::now()))
    {
        return Ok(json!({"changed":0,"skipped":true,"notModified":true}));
    }
    let mut row = existing.cloned().unwrap_or_else(|| {
        json!({
            "id":format!("subscription:public:{}",request.kind),"kind":request.kind,"enabled":true
        })
    });
    let revision = row["rev"].as_u64().unwrap_or(0);
    row["url"] = json!(request.index_url);
    let result = async {
        if !context.phase("index").await {
            return Err("Pull cancelled.".into());
        }
        let index_hash = if let Some(expected) = request.expected_index_hash.clone() {
            expected
        } else {
            let mut cache = probes.lock().await;
            if let Some(result) = cache.get(&request.index_url) {
                result.clone()?
            } else {
                let result = probe_reference_source_inner(&request.index_url).await;
                cache.insert(request.index_url.clone(), result.clone());
                result?
            }
        };
        let same_year = same_source && row["year"].as_i64() == Some(request.year.into());
        let cached = same_year && has_cache && row["indexHash"].as_str() == Some(&index_hash);
        let value = if cached {
            json!({"changed":0,"skipped":true,"notModified":true})
        } else {
            let known_hash = (same_year && has_cache)
                .then(|| row["remoteHash"].as_str().map(str::to_string))
                .flatten();
            if !context.phase("fetching").await {
                return Err("Pull cancelled.".into());
            }
            refresh_reference_source_inner(
                &store,
                &request.kind,
                &request.index_url,
                request.year,
                Some(index_hash.clone()),
                known_hash,
                context,
            )
            .await?
        };
        if !context.applying().await {
            return Err("Pull cancelled.".into());
        }
        if request.kind != "calendar" && value["notModified"].as_bool() == Some(false) {
            let materialized = store.load_workspace()?;
            store.record_reference_cache(
                &request.kind,
                request.year,
                json!({
                    "indexUrl":request.index_url,
                    "fingerprint":cache_fingerprint(&materialized, &request.kind, request.year)?
                }),
            )?;
        }
        row["year"] = json!(request.year);
        row["indexHash"] = json!(index_hash);
        for key in ["dataUrl", "remoteHash", "sourceTs"] {
            if let Some(value) = value.get(key) {
                row[key] = value.clone();
            }
        }
        row["fetchedAt"] = json!(Utc::now().to_rfc3339());
        row["lastError"] = Value::Null;
        Ok(value)
    }
    .await;
    // Error metadata is a write too, so cancellation and SQL admission share a gate.
    if !context.applying().await {
        return Err("Pull cancelled.".into());
    }
    if let Err(error) = &result {
        row["lastError"] = json!(error);
    }
    // User edits during a fetch take precedence over its metadata.
    // Checked writes also guard the small interval after this read.
    let latest = store
        .user_rows("subscriptions")?
        .into_iter()
        .find(|current| current["id"] == row["id"]);
    if latest
        .as_ref()
        .is_none_or(|current| current["rev"].as_u64().unwrap_or(0) == revision)
    {
        store.put_user_record_checked("subscriptions", row, "pull-coordinator", Some(revision))?;
    }
    let _ = app.emit("workspace-changed", json!({"source":"reference-pull"}));
    result
}

pub(crate) async fn refresh_all(
    app: &tauri::AppHandle,
    force: bool,
    selected: bool,
) -> Result<String, String> {
    let workspace = app.state::<DataStore>().load_workspace()?;
    let probes: Probes = Arc::new(Mutex::new(HashMap::new()));
    let mut changed = 0u64;
    let mut failures = 0usize;
    for kind in ["calendar", "departments", "important-dates", "exams"] {
        let row = workspace["subscriptions"].as_array().and_then(|rows| {
            rows.iter().find(|row| {
                row["kind"].as_str() == Some(kind)
                    && row.get("deletedAt").is_none_or(Value::is_null)
            })
        });
        if row.is_some_and(|row| row["enabled"].as_bool() == Some(false)) {
            continue;
        }
        let year = if kind == "departments" {
            workspace
                .pointer("/academicProfile/admissionYear")
                .and_then(Value::as_i64)
        } else {
            workspace[if selected {
                "selectedTerm"
            } else {
                "currentTerm"
            }]
            .as_str()
            .and_then(|term| term.get(..4))
            .and_then(|year| year.parse::<i64>().ok())
        };
        let Some(year) = year.and_then(|year| i32::try_from(year).ok()) else {
            continue;
        };
        let url = row
            .and_then(|row| row["url"].as_str())
            .filter(|url| url.starts_with("https://"))
            .unwrap_or(PUBLIC_INDEX)
            .to_string();
        let request = ReferenceRequest {
            kind: kind.into(),
            index_url: url,
            year,
            force,
            expected_index_hash: None,
        };
        match refresh_one(
            app,
            request,
            if force {
                PullPriority::Manual
            } else {
                PullPriority::Automatic
            },
            probes.clone(),
        )
        .await
        {
            Ok(result) => changed += result["changed"].as_u64().unwrap_or(0),
            Err(_) => failures += 1,
        }
    }
    app.state::<DataStore>().record_reference_poll()?;
    let _ = app.emit("workspace-changed", json!({"source":"reference-poll"}));
    if failures > 0 {
        Err(format!("{failures} reference source(s) failed"))
    } else if changed > 0 {
        Ok(format!("Updated {changed} reference dataset(s)"))
    } else {
        Ok("Reference data is up to date".into())
    }
}

pub(crate) fn probes() -> Probes {
    Arc::new(Mutex::new(HashMap::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restored_metadata_cannot_replace_materialization_evidence_even_for_empty_feeds() {
        let path =
            std::env::temp_dir().join(format!("isct-reference-cache-{}.db", uuid::Uuid::new_v4()));
        let store = DataStore::initialize(path).unwrap().0;
        for kind in ["departments", "important-dates", "exams"] {
            let request = ReferenceRequest {
                kind: kind.into(),
                index_url: "https://example.com/index.json".into(),
                year: 2026,
                force: false,
                expected_index_hash: None,
            };
            let workspace = store.load_workspace().unwrap();
            assert!(!cache_present(&store, &workspace, &request).unwrap());
            store.record_reference_cache(kind, 2026, json!({
                "indexUrl":request.index_url,"fingerprint":cache_fingerprint(&workspace, kind, 2026).unwrap()
            })).unwrap();
            assert!(cache_present(&store, &workspace, &request).unwrap());
            let mut changed = workspace.clone();
            let field = if kind == "departments" {
                "departments"
            } else if kind == "exams" {
                "exams"
            } else {
                "importantDates"
            };
            changed[field] = json!([{ "id":"new-entry", "cohortYear":2026,"academicYear":2026 }]);
            assert!(!cache_present(&store, &changed, &request).unwrap());
        }
        let backup: Value = serde_json::from_str(&store.export_data(json!([])).unwrap()).unwrap();
        assert!(backup["tables"]["settings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| !row["key"].as_str().unwrap().starts_with("referenceCache:")));
        let restored_path = std::env::temp_dir().join(format!(
            "isct-reference-restored-{}.db",
            uuid::Uuid::new_v4()
        ));
        let restored = DataStore::initialize(restored_path).unwrap().0;
        restored.import_data(&backup).unwrap();
        assert!(restored
            .reference_cache_marker("exams", 2026)
            .unwrap()
            .is_none());
    }

    #[test]
    fn equivalent_index_urls_preserve_cache_identity() {
        assert!(same_index(
            Some(" HTTPS://EXAMPLE.COM:443/index.json#old "),
            "https://example.com/index.json"
        ));
        assert!(!same_index(
            Some("https://example.com/other.json"),
            "https://example.com/index.json"
        ));
        assert!(!same_index(None, "https://example.com/index.json"));
    }
}
