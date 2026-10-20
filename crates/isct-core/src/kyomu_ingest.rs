use crate::data::{resolve_course_id, versioned::content_hash};
use crate::lms_ingest::IngestError;
use crate::school_parser::kyomu::KyomuPayload;
use crate::sync_state::{self, Commit, Dataset, Key, Outcome};
use chrono::Utc;
use rusqlite::{params, OptionalExtension, Transaction};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

fn upsert(tx: &Transaction<'_>, table: &str, id: &str, value: &Value) -> Result<bool, IngestError> {
    let hash = content_hash(value)?;
    let old: Option<(String, u64, Option<String>)> = tx
        .query_row(
            &format!("SELECT hash,rev,missing_since FROM {table} WHERE id=?1"),
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(IngestError::from)?;
    let timestamp = Utc::now().to_rfc3339();
    if old.as_ref().is_some_and(|(h, _, _)| h == &hash) {
        let changed = old
            .as_ref()
            .is_some_and(|(_, _, missing)| missing.is_some());
        tx.execute(&format!("UPDATE {table} SET last_seen_at=?2,missing_since=NULL,deleted_at=NULL,
            value=json_remove(json_set(value,'$.lastSeenAt',?2),'$.missingSince','$.deletedAt') WHERE id=?1"), params![id,timestamp])
            .map_err(IngestError::from)?;
        return Ok(changed);
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
    let missing = Utc::now().to_rfc3339();
    let mut changed = 0;
    for id in stale.into_iter().filter(|id| !live.contains(id)) {
        changed += tx.execute(
            &format!("UPDATE {table} SET missing_since=?2,rev=rev+1,value=json_set(value,'$.missingSince',?2,'$.rev',rev+1,'$.updatedAt',?2) WHERE id=?1"),
            params![id, missing],
        ).map_err(IngestError::from)?;
    }
    Ok(changed)
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
pub fn gpa_summaries(data: &Value) -> Vec<Value> {
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
    let fetched = Utc::now().to_rfc3339();
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
pub fn declared_credits(data: &Value) -> Vec<Value> {
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

/// Projects one 教務Web page inside the caller's transaction and marks its
/// `kyomu.page/{kind}` key fresh for `account` in the same transaction.
pub fn apply_kyomu_payload(
    tx: &Transaction<'_>,
    payload: &KyomuPayload,
    account: &str,
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
                WHERE settings.hash<>excluded.hash", params![value,hash,Utc::now().to_rfc3339()])?;
        }
        "registrations" => {
            let mut state_ids = HashSet::new();
            if let Some(state) = payload.data.get("state") {
                let current = (Utc::now() + chrono::Duration::hours(9)).date_naive();
                let year = chrono::Datelike::year(&current)
                    - i32::from(chrono::Datelike::month(&current) < 4);
                let id = "kyomu-registration-state";
                state_ids.insert(id.to_string());
                let mut value = state.clone();
                if let Some(o) = value.as_object_mut() {
                    o.insert("id".into(), id.into());
                    o.insert(
                        "academicYear".into(),
                        payload.data["courses"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .find_map(|c| c["offeringId"].as_str()?.get(..4)?.parse::<i32>().ok())
                            .unwrap_or(year)
                            .into(),
                    );
                    o.insert(
                        "quarters".into(),
                        payload
                            .data
                            .get("quarters")
                            .cloned()
                            .unwrap_or_else(|| json!([])),
                    );
                }
                changed += usize::from(upsert(tx, "registration_states", id, &value)?);
                let scope_id = format!(
                    "kyomu-scope:{}:{}",
                    value["academicYear"], value["quarters"]
                );
                value["id"] = scope_id.clone().into();
                changed += usize::from(upsert(tx, "registration_states", &scope_id, &value)?);
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
            let current = (Utc::now() + chrono::Duration::hours(9)).date_naive();
            let year = payload.data["courses"]
                .as_array()
                .into_iter()
                .flatten()
                .find_map(|c| c["offeringId"].as_str()?.get(..4)?.parse::<i32>().ok())
                .unwrap_or(
                    chrono::Datelike::year(&current)
                        - i32::from(chrono::Datelike::month(&current) < 4),
                );
            changed += expire_registration_scope(
                tx,
                "kyomu-registration:",
                year,
                payload.data["quarters"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                &ids,
            )?;
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
            let scope: Option<String> = tx
                .query_row(
                    "SELECT value FROM registration_states WHERE id='kyomu-registration-state'",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(scope) = scope {
                let scope: Value = serde_json::from_str(&scope)?;
                if let Some(year) = scope["academicYear"]
                    .as_i64()
                    .and_then(|y| i32::try_from(y).ok())
                {
                    changed += expire_registration_scope(
                        tx,
                        "kyomu-declared:",
                        year,
                        scope["quarters"]
                            .as_array()
                            .map(Vec::as_slice)
                            .unwrap_or(&[]),
                        &live,
                    )?;
                }
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
    commit_page(tx, payload, account, Outcome::Complete, None)?;
    Ok(changed)
}

/// Persists a positive observation that an approved 教務 page is not offered
/// to this account. Existing page-owned rows remain untouched; the key reads
/// as current with outcome `unavailable`, so the UI can tell retained data
/// from a current result.
pub fn record_kyomu_unavailable(
    tx: &Transaction<'_>,
    payload: &KyomuPayload,
    account: &str,
) -> Result<(), IngestError> {
    if payload.status != "unavailable" {
        return Err("教務 page is not reported unavailable".into());
    }
    let message = Some(payload.message.as_str()).filter(|message| !message.is_empty());
    commit_page(tx, payload, account, Outcome::Unavailable, message)
}

fn commit_page(
    tx: &Transaction<'_>,
    payload: &KyomuPayload,
    account: &str,
    outcome: Outcome,
    message: Option<&str>,
) -> Result<(), IngestError> {
    if crate::school_parser::kyomu::page_url(&payload.kind).is_none() {
        return Err("unsupported kyomu payload".into());
    }
    sync_state::commit(
        tx,
        &Key::new(Dataset::KyomuPage, payload.kind.as_str()),
        Commit {
            account,
            validator: None,
            outcome,
            message,
        },
        Utc::now(),
    )?;
    Ok(())
}

#[cfg(test)]
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
            let missing = Utc::now().to_rfc3339();
            tx.execute("UPDATE registrations SET missing_since=?2,value=json_set(value,'$.missingSince',?2) WHERE id=?1",params![id,missing])?;
        }
    }
    Ok(changed)
}
