use crate::data::{
    resolve_course_id,
    versioned::{content_hash, keep_unchanged},
};
use crate::lms_ingest::IngestError;
use crate::sync_state::{self, Commit, Dataset, Key, Outcome};
use adpt_isct::kyomu::KyomuPayload;
use rusqlite::{params, OptionalExtension, Transaction};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap, HashSet};

/// Fields that say when a row was read, not what it holds: left out of the
/// content hash, so a read that finds the same content writes nothing. The
/// key's own freshness (`dataset_state.fetched_at`) says when it was read.
const READ_TIME_FIELDS: [&str; 1] = ["fetchedAt"];

fn upsert(tx: &Transaction<'_>, table: &str, id: &str, value: &Value) -> Result<bool, IngestError> {
    let mut hashed = value.clone();
    if let Some(object) = hashed.as_object_mut() {
        for field in READ_TIME_FIELDS {
            object.remove(field);
        }
    }
    let hash = content_hash(&hashed)?;
    let old: Option<(String, u64, bool)> = tx
        .query_row(
            &format!("SELECT hash,rev,missing_since IS NOT NULL OR deleted_at IS NOT NULL FROM {table} WHERE id=?1"),
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(IngestError::from)?;
    let timestamp = crate::clock::now().to_rfc3339();
    if let Some((stored, _, gone)) = &old {
        if stored == &hash {
            return keep_unchanged(tx, table, id, *gone, &timestamp).map_err(IngestError::from);
        }
    }
    let rev = old.map_or(0, |x| x.1) + 1;
    let mut stored = value.clone();
    if let Some(object) = stored.as_object_mut() {
        object.insert("rev".into(), rev.into());
        object.insert("updatedAt".into(), timestamp.clone().into());
        object.insert("updatedBy".into(), "kyomu".into());
        object.insert("source".into(), json!({"provider":"kyomu"}));
        object.insert(
            "sourceKey".into(),
            format!("kyomu:default:{table}:{id}").into(),
        );
        object.insert("lastSeenAt".into(), timestamp.clone().into());
        object.remove("missingSince");
        object.remove("deletedAt");
    }
    let json = serde_json::to_string(&stored).map_err(IngestError::from)?;
    tx.execute(
        &format!(
            "INSERT INTO {table}
             (id,value,hash,updated_at,updated_by,rev,source,deleted_at,last_seen_at,missing_since)
             VALUES (?1,?2,?3,?4,'kyomu',?5,'kyomu',NULL,?4,NULL)
             ON CONFLICT(id) DO UPDATE SET value=excluded.value,hash=excluded.hash,
               updated_at=excluded.updated_at,updated_by=excluded.updated_by,rev=excluded.rev,
               source=excluded.source,deleted_at=NULL,last_seen_at=excluded.last_seen_at,
               missing_since=NULL"
        ),
        params![id, json, hash, timestamp, rev as i64],
    )
    .map_err(IngestError::from)?;
    Ok(true)
}

fn mark_missing(
    tx: &Transaction<'_>,
    table: &str,
    prefix: &str,
    live: &HashSet<String>,
) -> Result<usize, IngestError> {
    let mut statement = tx
        .prepare(&format!(
            "SELECT id FROM {table} WHERE id LIKE ?1 AND missing_since IS NULL"
        ))
        .map_err(IngestError::from)?;
    let pattern = format!("{prefix}%");
    let stale = statement
        .query_map([pattern], |row| row.get::<_, String>(0))
        .map_err(IngestError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(IngestError::from)?;
    drop(statement);
    let missing = crate::clock::now().to_rfc3339();
    crate::data::versioned::mark_missing_rows(tx, table, stale, live, || missing.as_str())
        .map_err(IngestError::from)
}

fn number(raw: &str) -> Option<f64> {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    cleaned.parse().ok()
}

/// `table.tblGPASummary.mediaPC` under 通算: row 1 names the groups
/// (全ての科目 / 教養科目群 / 専門科目群), row 2 the columns (GPA, GPT,
/// 修得済単位, 合格率%), row 3 the values in the same order.
pub(crate) fn gpa_summaries(data: &Value) -> Vec<Value> {
    let Some(table) = data
        .get("overall")
        .and_then(Value::as_array)
        .and_then(|tables| tables.first())
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let row = |index: usize| -> Vec<String> {
        table
            .get(index)
            .and_then(Value::as_array)
            .map(|cells| {
                cells
                    .iter()
                    .map(|cell| cell.as_str().unwrap_or("").replace(' ', ""))
                    .collect()
            })
            .unwrap_or_default()
    };
    let (labels, values) = (row(1), row(2));
    if labels.is_empty() || labels.len() != values.len() {
        return Vec::new();
    }
    let fetched = crate::clock::now().to_rfc3339();
    let mut scopes: Vec<Value> = Vec::new();
    let mut current: Option<serde_json::Map<String, Value>> = None;
    let order = ["all", "liberal-arts", "major"];
    for (label, value) in labels.iter().zip(values.iter()) {
        // Every group starts with its GPA column.
        if label.eq_ignore_ascii_case("GPA") {
            if let Some(done) = current.take() {
                scopes.push(Value::Object(done));
            }
            let scope = order.get(scopes.len()).copied().unwrap_or("other");
            let mut entry = serde_json::Map::new();
            entry.insert("id".into(), format!("kyomu-gpa:{scope}").into());
            entry.insert("scope".into(), scope.into());
            entry.insert("earnedCredits".into(), 0.into());
            entry.insert("fetchedAt".into(), fetched.clone().into());
            current = Some(entry);
        }
        let Some(entry) = current.as_mut() else {
            continue;
        };
        let key = if label.eq_ignore_ascii_case("GPA") {
            "gpa"
        } else if label.eq_ignore_ascii_case("GPT") {
            "gpt"
        } else if label.contains("単位") {
            "earnedCredits"
        } else if label.contains("合格率") {
            "passRate"
        } else {
            continue;
        };
        if let Some(number) = number(value) {
            entry.insert(key.into(), number.into());
        }
    }
    if let Some(done) = current.take() {
        scopes.push(Value::Object(done));
    }
    scopes
        .into_iter()
        .filter(|scope| scope["scope"] != "other")
        .collect()
}

/// One row per declared course with the credits 教務Web counts: the row's
/// split, scaled so each category adds up to the total the page states.
pub(crate) fn declared_credits(data: &Value) -> Vec<Value> {
    let courses = data
        .get("courses")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let raw = |course: &Value| {
        ["lecture", "exercise", "lab"]
            .iter()
            .filter_map(|key| {
                course
                    .pointer(&format!("/credits/{key}"))
                    .and_then(Value::as_f64)
            })
            .sum::<f64>()
    };
    let mut unit_sums: HashMap<String, f64> = HashMap::new();
    for course in &courses {
        let unit = course
            .get("unit")
            .and_then(Value::as_str)
            .unwrap_or_default();
        *unit_sums.entry(unit.to_string()).or_default() += raw(course);
    }
    courses
        .iter()
        .filter_map(|course| {
            let offering = course.get("offeringId")?.as_str()?;
            let unit = course
                .get("unit")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let listed = raw(course);
            let sum = unit_sums.get(unit).copied().unwrap_or(0.0);
            let effective = match course.get("unitTotal").and_then(Value::as_f64) {
                Some(total) if sum > 0.0 => listed * total / sum,
                _ => listed,
            };
            let mut row = course.clone();
            let object = row.as_object_mut()?;
            object.insert("id".into(), format!("kyomu-declared:{offering}").into());
            object.insert("kind".into(), "declared".into());
            // Two decimals: halves and quarters stay exact, float noise does not.
            object.insert(
                "effectiveCredits".into(),
                ((effective * 100.0).round() / 100.0).into(),
            );
            Some(row)
        })
        .collect()
}

fn grade_row_id(
    graduate_flat: bool,
    section: &str,
    row: &Value,
    index: usize,
) -> Result<String, IngestError> {
    let code = row.get("code").and_then(Value::as_str).unwrap_or("");
    let completed = row.get("completed").and_then(Value::as_str).unwrap_or("");
    if !graduate_flat {
        return Ok(format!("kyomu-grade:{code}:{completed}:{index}"));
    }
    let normalized = |value: &str| value.split_whitespace().collect::<Vec<_>>().join(" ");
    let section = normalized(section);
    let title = normalized(row.get("title").and_then(Value::as_str).unwrap_or(""));
    let completed = normalized(completed);
    if section.is_empty() || title.is_empty() || completed.is_empty() {
        return Err("graduate grade identity is incomplete".into());
    }
    let identity = content_hash(&json!({
        "section": section,
        "title": title,
        "completed": completed,
    }))?;
    Ok(format!("kyomu-grade:flat:{identity}"))
}

/// What a complete registered-courses page covers: the academic year its
/// courses belong to and the quarters it lists. Only such a page, read in
/// the same operation, may say a declared course is gone (`d/aevz1j`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RegistrationScope {
    pub year: i32,
    pub quarters: Vec<Value>,
}

/// The scope a complete registered-courses page proves (`d/sgfx8a`): its
/// quarters, and the one academic year every listed course belongs to (the
/// first four digits of an official offering id). `None` for an unavailable
/// or other page, one listing no quarters, and one whose year the page does
/// not prove: no course to tell it, or courses of two years (a rollover).
/// The page's row order never matters, and the device's date is no evidence.
pub(crate) fn registration_scope(payload: &KyomuPayload) -> Option<RegistrationScope> {
    if payload.status != "ok" || payload.kind != "registrations" {
        return None;
    }
    let years: BTreeSet<i32> = payload.data["courses"]
        .as_array()?
        .iter()
        .map(|course| {
            course["offeringId"]
                .as_str()
                .and_then(|id| id.get(..4))
                .and_then(|year| year.parse::<i32>().ok())
        })
        .collect::<Option<_>>()?;
    let year = match years.len() {
        1 => years.into_iter().next()?,
        _ => return None,
    };
    let quarters = payload.data["quarters"].as_array()?.clone();
    (!quarters.is_empty()).then_some(RegistrationScope { year, quarters })
}

/// Stores one approved 教務 page; a registered list expires rows inside the
/// scope it proves. A declared page stored this way only adds: use
/// [`apply_kyomu_in_scope`] with the scope of the registered list read
/// beside it to let it drop courses.
pub(crate) fn apply_kyomu_payload(
    tx: &Transaction<'_>,
    payload: &KyomuPayload,
) -> Result<usize, IngestError> {
    apply_page(tx, payload, registration_scope(payload).as_ref())
}

/// Stores a page with the registered list's scope already proven
/// ([`registration_scope`]): a registered list's own, or for a declared
/// page that of the list read in the same operation, inside which courses
/// it no longer lists leave.
pub(crate) fn apply_kyomu_in_scope(
    tx: &Transaction<'_>,
    payload: &KyomuPayload,
    scope: &RegistrationScope,
) -> Result<usize, IngestError> {
    apply_page(tx, payload, Some(scope))
}

/// `scope`: what the registered list of this operation covers, the only
/// ground for expiring registered or declared rows.
fn apply_page(
    tx: &Transaction<'_>,
    payload: &KyomuPayload,
    scope: Option<&RegistrationScope>,
) -> Result<usize, IngestError> {
    if payload.status != "ok" {
        return Err("教務 extraction is not ready".into());
    }
    let mut changed = 0;
    match payload.kind.as_str() {
        "profile" => {
            let academic = json!({"department":payload.data["department"],"admittedOn":payload.data["admittedOn"],"programme":payload.data["programme"],"entryCategory":payload.data["entryCategory"]});
            let value = serde_json::to_string(&academic)?;
            let hash = content_hash(&academic)?;
            changed += tx.execute("INSERT INTO settings(key,value,hash,updated_at,updated_by,rev,source)
                VALUES('kyomuAcademicIdentity',?1,?2,?3,'kyomu',1,'kyomu') ON CONFLICT(key) DO UPDATE SET
                value=excluded.value,hash=excluded.hash,updated_at=excluded.updated_at,rev=settings.rev+1
                WHERE settings.hash<>excluded.hash", params![value,hash,crate::clock::now().to_rfc3339()])?;
        }
        "registrations" => {
            let mut state_ids = HashSet::new();
            if let Some(state) = payload.data.get("state") {
                let id = "kyomu-registration-state";
                state_ids.insert(id.to_string());
                let mut value = state.clone();
                if let Some(o) = value.as_object_mut() {
                    o.insert("id".into(), id.into());
                }
                changed += usize::from(upsert(tx, "registration_states", id, &value)?);
            }
            let mut ids = HashSet::new();
            for course in payload
                .data
                .get("courses")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(offering) = course.get("offeringId").and_then(Value::as_str) else {
                    continue;
                };
                if let Some(Ok(year)) = offering.get(..4).map(str::parse::<i32>) {
                    resolve_course_id(tx, "kyomu", offering, year, Some(offering))?;
                }
                let id = format!("kyomu-registration:{offering}");
                ids.insert(id.clone());
                let mut value = course.clone();
                if let Some(o) = value.as_object_mut() {
                    o.insert("id".into(), id.clone().into());
                }
                changed += usize::from(upsert(tx, "registrations", &id, &value)?);
            }
            // This page covers one academic year. Never expire previous years.
            if let Some(scope) = scope {
                changed += expire_registration_scope(
                    tx,
                    "kyomu-registration:",
                    scope.year,
                    &scope.quarters,
                    &ids,
                )?;
            }
            changed += mark_missing(
                tx,
                "registration_states",
                "kyomu-registration-state",
                &state_ids,
            )?;
        }
        "declared" => {
            let rows = declared_credits(&payload.data);
            let live: HashSet<String> = rows
                .iter()
                .filter_map(|row| row["id"].as_str().map(str::to_string))
                .collect();
            for row in &rows {
                let id = row["id"].as_str().unwrap_or_default();
                changed += usize::from(upsert(tx, "registrations", id, row)?);
            }
            // Rows leave the declared list only inside the scope the
            // registered list read in this operation covers, and only when
            // every listed row is of that year (`d/aevz1j`).
            if let Some(scope) =
                scope.filter(|scope| rows_of_year(&live, "kyomu-declared:", scope.year))
            {
                changed += expire_registration_scope(
                    tx,
                    "kyomu-declared:",
                    scope.year,
                    &scope.quarters,
                    &live,
                )?;
            }
        }
        "grades" => {
            let mut live = HashSet::new();
            let graduate_flat =
                payload.data.get("shape").and_then(Value::as_str) == Some("graduate-flat");
            for group in payload
                .data
                .get("groups")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let category = group.get("category").and_then(Value::as_str).unwrap_or("");
                let section = group.get("section").and_then(Value::as_str).unwrap_or("");
                for (index, row) in group
                    .get("rows")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    let id = grade_row_id(graduate_flat, section, row, index)?;
                    if !live.insert(id.clone()) {
                        return Err("duplicate graduate grade identity".into());
                    }
                    let mut value = row.clone();
                    if let Some(o) = value.as_object_mut() {
                        o.insert("id".into(), id.clone().into());
                        o.insert("category".into(), json!({"ja":category}));
                    }
                    changed += usize::from(upsert(tx, "grades", &id, &value)?);
                }
            }
            changed += mark_missing(tx, "grades", "kyomu-grade:", &live)?;
        }
        "requirements" => {
            let mut live = HashSet::new();
            for (table_index, table) in payload
                .data
                .get("tables")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                let headers = table
                    .get("headers")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                for row in table
                    .get("rows")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let label = row.get("label").and_then(Value::as_str).unwrap_or("");
                    for (column, value) in row
                        .get("values")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .enumerate()
                    {
                        let category = headers
                            .get(column + 1)
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let id =
                            format!("kyomu-requirement:{table_index}:{column}:{category}:{label}");
                        live.insert(id.clone());
                        let record = json!({"id":id,"table":table_index,"category":category,"label":label,"value":value});
                        changed += usize::from(upsert(tx, "requirement_progress", &id, &record)?);
                    }
                }
            }
            changed += mark_missing(tx, "requirement_progress", "kyomu-requirement:", &live)?;
        }
        "gpa" => {
            let mut summary_ids = HashSet::new();
            for summary in gpa_summaries(&payload.data) {
                let id = summary["id"].as_str().unwrap_or_default().to_string();
                summary_ids.insert(id.clone());
                changed += usize::from(upsert(tx, "gpa_summary", &id, &summary)?);
            }
            let mut term_ids = HashSet::new();
            for (index, row) in payload
                .data
                .get("terms")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                let id = format!("kyomu-gpa-term:{index}");
                term_ids.insert(id.clone());
                let value = json!({"id":id,"cells":row});
                changed += usize::from(upsert(tx, "gpa_by_term", &id, &value)?);
            }
            changed += mark_missing(tx, "gpa_summary", "kyomu-gpa:", &summary_ids)?;
            changed += mark_missing(tx, "gpa_by_term", "kyomu-gpa-term:", &term_ids)?;
        }
        _ => return Err("unsupported kyomu payload".into()),
    }
    commit_page(tx, payload, Outcome::Complete, None)?;
    Ok(changed)
}

/// Persists a positive observation that an approved 教務 page is not offered
/// to the student now. Existing page-owned rows remain untouched; the key
/// reads as current with outcome `unavailable`, so the UI can tell retained
/// data from a current result.
pub(crate) fn record_kyomu_unavailable(
    tx: &Transaction<'_>,
    payload: &KyomuPayload,
) -> Result<(), IngestError> {
    if payload.status != "unavailable" {
        return Err("教務 page is not reported unavailable".into());
    }
    let message = Some(payload.message.as_str()).filter(|message| !message.is_empty());
    commit_page(tx, payload, Outcome::Unavailable, message)
}

fn commit_page(
    tx: &Transaction<'_>,
    payload: &KyomuPayload,
    outcome: Outcome,
    message: Option<&str>,
) -> Result<(), IngestError> {
    if adpt_isct::kyomu::page_url(&payload.kind).is_none() {
        return Err("unsupported kyomu payload".into());
    }
    sync_state::commit(
        tx,
        &Key::new(Dataset::KyomuPage, payload.kind.as_str()),
        Commit {
            validator: None,
            outcome,
            message,
        },
        crate::clock::now(),
    )?;
    Ok(())
}

#[cfg(all(test, kyomu_captures))]
#[path = "tests/kyomu_ingest.rs"]
mod tests;

pub(crate) fn registration_quarters(course: &Value) -> Vec<String> {
    let mut quarters = course
        .get("quarters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|q| {
            q.as_u64()
                .filter(|q| (1..=4).contains(q))
                .map(|q| format!("{q}Q"))
        })
        .collect::<Vec<_>>();
    for slot in course
        .get("schedule")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(quarter) = slot.get("quarter").and_then(Value::as_str) {
            quarters.push(quarter.to_string());
        } else if let Some(raw) = slot.get("raw").and_then(Value::as_str) {
            let prefix = raw.split('Q').next().unwrap_or_default();
            let digits: Vec<_> = prefix
                .chars()
                .filter_map(|c| c.to_digit(10))
                .filter(|v| (1..=4).contains(v))
                .collect();
            if let (Some(first), Some(last)) = (digits.first(), digits.last()) {
                quarters.extend((*first..=*last).map(|q| format!("{q}Q")));
            }
        }
    }
    quarters.sort();
    quarters.dedup();
    let inactive = course.get("inactiveQuarters").and_then(Value::as_array);
    quarters.retain(|q| inactive.is_none_or(|values| !values.contains(&json!(q))));
    quarters
}

/// Whether every id (`{prefix}{offering}`) names an offering of `year`: the
/// first four digits of an official offering id are its academic year.
fn rows_of_year(ids: &HashSet<String>, prefix: &str, year: i32) -> bool {
    let year = year.to_string();
    ids.iter().all(|id| {
        id.strip_prefix(prefix)
            .is_some_and(|offering| offering.starts_with(&year))
    })
}

fn expire_registration_scope(
    tx: &Transaction<'_>,
    prefix: &str,
    year: i32,
    quarters: &[Value],
    live: &HashSet<String>,
) -> Result<usize, IngestError> {
    // The current-semester page cannot say anything about a previous semester.
    if quarters.is_empty() {
        return Ok(0);
    }
    let mut statement =
        tx.prepare("SELECT value FROM registrations WHERE id LIKE ?1 AND missing_since IS NULL")?;
    let rows = statement
        .query_map([format!("{prefix}{year}%")], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    let mut changed = 0;
    for raw in rows {
        let mut row: Value = serde_json::from_str(&raw)?;
        let id = row["id"].as_str().unwrap_or_default().to_string();
        if live.contains(&id) {
            continue;
        }
        let active = registration_quarters(&row);
        let removed: Vec<_> = active
            .iter()
            .filter(|q| quarters.contains(&json!(q)))
            .map(|q| json!(q))
            .collect();
        if removed.is_empty() {
            continue;
        }
        let mut inactive = row
            .get("inactiveQuarters")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        inactive.extend(removed);
        row["inactiveQuarters"] = json!(inactive);
        changed += usize::from(upsert(tx, "registrations", &id, &row)?);
        if registration_quarters(&row).is_empty() {
            let missing = crate::clock::now().to_rfc3339();
            tx.execute("UPDATE registrations SET missing_since=?2,value=json_set(value,'$.missingSince',?2) WHERE id=?1",params![id,missing])?;
        }
    }
    Ok(changed)
}
