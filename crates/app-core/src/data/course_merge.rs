//! Merged courses (`approach/derived-rows-in-write-transaction`).
//!
//! A course merges its LMS rows, its catalog offering, 教務 rooms and the
//! person's own course record. The merge runs inside the transaction that
//! writes one of those sources, for the courses that write touched only, and
//! its rows land in `cache.merged_rows`; reads take them as they are. A
//! course's rows are its merged record, the memberships and terms its LMS
//! quarters imply, each keyed so that reading them in key order gives the
//! order the read-time projection produced: timetable courses, then the
//! person's courses, then LMS-only courses, by LMS row.
use super::*;
use rusqlite::hooks::Action;
use std::collections::{BTreeMap, HashMap};

use crate::schedule::{array, text_or_empty as text};

/// A row a transaction inserted, updated or deleted, as the update hook
/// reported it.
pub(super) type Touched = (Action, String, i64);

/// Tables a course merges.
const SOURCES: &[&str] = &[
    "lms_courses",
    "offerings",
    "registrations",
    "timetable_courses",
    "manual_courses",
    "memberships",
    "manual_memberships",
    "plan_items",
    "course_keys",
    "settings",
];

/// Settings that shape every merged course: content language and teaching
/// periods.
const MERGE_SETTINGS: &[&str] = &[
    "settings",
    "userPreferences",
    "devicePreferences",
    "academicProfile",
];

/// Merges again the courses `touched` rows feed, in the writing transaction.
pub(super) fn refresh(transaction: &Transaction<'_>, touched: &[Touched]) -> Result<(), DataError> {
    if !built(transaction)? {
        return rebuild(transaction);
    }
    let mut ids = BTreeSet::new();
    let mut rows: BTreeMap<&str, Vec<i64>> = BTreeMap::new();
    for (action, table, rowid) in touched {
        if !SOURCES.contains(&table.as_str()) {
            continue;
        }
        // A deleted row cannot be read back to name its course, and a key
        // that moved may move many: merge everything.
        if *action == Action::SQLITE_DELETE
            || (table == "course_keys" && *action == Action::SQLITE_UPDATE)
        {
            return rebuild(transaction);
        }
        rows.entry(table.as_str()).or_default().push(*rowid);
    }
    for (table, rowids) in rows {
        for rowid in rowids {
            if table == "settings" {
                let key: Option<String> = transaction
                    .query_row("SELECT key FROM settings WHERE rowid=?1", [rowid], |row| {
                        row.get(0)
                    })
                    .optional()?;
                if key.is_some_and(|key| MERGE_SETTINGS.contains(&key.as_str())) {
                    return rebuild(transaction);
                }
                continue;
            }
            if table == "course_keys" {
                let course: Option<String> = transaction
                    .query_row(
                        "SELECT course_id FROM course_keys WHERE rowid=?1",
                        [rowid],
                        |row| row.get(0),
                    )
                    .optional()?;
                ids.extend(course);
                continue;
            }
            let raw: Option<String> = transaction
                .query_row(
                    &format!("SELECT value FROM {table} WHERE rowid=?1"),
                    [rowid],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(value) = raw.and_then(|raw| serde_json::from_str::<Value>(&raw).ok()) else {
                continue;
            };
            ids.extend(courses_fed_by(transaction, table, &value)?);
        }
    }
    if ids.is_empty() {
        return Ok(());
    }
    merge(transaction, Some(&ids))
}

/// Merges every course again, e.g. after the caches were emptied.
pub(super) fn rebuild(transaction: &Transaction<'_>) -> Result<(), DataError> {
    merge(transaction, None)?;
    transaction.execute(
        "INSERT INTO cache.merge_state(built) SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM cache.merge_state)",
        [],
    )?;
    Ok(())
}

fn built(connection: &Connection) -> Result<bool, DataError> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM cache.merge_state)",
        [],
        |row| row.get(0),
    )?)
}

/// The internal ids of the courses a source row feeds.
fn courses_fed_by(
    transaction: &Transaction<'_>,
    table: &str,
    value: &Value,
) -> Result<Vec<String>, DataError> {
    let keyed = |key: Option<String>| -> Result<Option<String>, DataError> {
        key.map_or(Ok(None), |key| course_keys::lookup(transaction, &key))
    };
    let year_of = |value: &Value, field: &str| -> Option<i32> {
        value
            .get(field)
            .and_then(Value::as_i64)
            .map(|year| year as i32)
            .or_else(|| {
                text(value, "termId")
                    .get(..4)
                    .and_then(|year| year.parse().ok())
            })
    };
    let mut ids = Vec::new();
    match table {
        "lms_courses" => ids.extend(lms_course_id(transaction, value)?),
        "offerings" | "registrations" => {
            let offering = if table == "offerings" {
                text(value, "id")
            } else {
                text(value, "offeringId")
            };
            let year = year_of(value, "year")
                .or_else(|| offering.get(..4).and_then(|year| year.parse().ok()));
            ids.extend(keyed(
                year.and_then(|year| course_keys::offering_key(year, offering)),
            )?);
        }
        "timetable_courses" | "manual_courses" => {
            ids.push(text(value, "id").to_owned());
            ids.extend(keyed(year_of(value, "year").and_then(|year| {
                course_keys::offering_key(year, text(value, "offeringId"))
            }))?);
        }
        _ => ids.push(text(value, "courseId").to_owned()),
    }
    ids.retain(|id| !id.is_empty());
    Ok(ids)
}

/// The course an LMS row is: the one its LMS key names, given a key when it
/// has none yet (rows stored before course keys).
fn lms_course_id(transaction: &Transaction<'_>, lms: &Value) -> Result<Option<String>, DataError> {
    let Some(lms_id) = lms["lmsId"].as_u64() else {
        return Ok(None);
    };
    let year = lms["year"]
        .as_i64()
        .map(|year| year as i32)
        .unwrap_or_else(|| Utc::now().year());
    resolve_course_id(
        transaction,
        "lms",
        &lms_id.to_string(),
        year,
        lms["offeringId"].as_str(),
    )
    .map(Some)
}

/// One output of the merge, with the LMS row it came from.
enum Pushed {
    Course(String, String),
    Membership(String, Value),
    Term(String, String, String),
}

/// Merges the courses `ids` names, or every course, and replaces their rows.
fn merge(transaction: &Transaction<'_>, ids: Option<&BTreeSet<String>>) -> Result<(), DataError> {
    let wanted = |id: &str| ids.is_none_or(|ids| ids.contains(id));
    let timetable: Vec<Value> = read_visible_rows(transaction, "timetable_courses")?
        .into_iter()
        .filter(|row| wanted(text(row, "id")))
        .collect();
    let manual: Vec<Value> = read_rows(transaction, "manual_courses")?
        .into_iter()
        .filter(|row| wanted(text(row, "id")))
        .collect();
    let mut lms_courses = Vec::new();
    for lms in read_visible_rows(transaction, "lms_courses")? {
        if lms_course_id(transaction, &lms)?.is_some_and(|id| wanted(&id)) {
            lms_courses.push(lms);
        }
    }
    let lms_courses = &lms_courses[..];
    let user_courses: HashSet<String> = manual
        .iter()
        .map(|row| text(row, "id").to_string())
        .collect();
    let mut workspace = empty_workspace();
    DataStore::settings_into(transaction, &mut workspace)?;
    let placed = |table: &str| -> Result<Vec<Value>, DataError> {
        Ok(read_visible_rows(transaction, table)?
            .into_iter()
            .filter(|row| wanted(text(row, "courseId")))
            .collect())
    };
    let mut memberships = placed("memberships")?;
    memberships.extend(placed("manual_memberships")?);
    workspace["memberships"] = Value::Array(memberships);
    workspace["planItems"] = Value::Array(placed("plan_items")?);
    workspace["terms"] = json!([]);
    workspace["courses"] = Value::Array(timetable.iter().chain(&manual).cloned().collect());
    let mut pushed = Vec::new();
    let teaching = crate::schedule::teaching_periods(&workspace)
        .map_err(|e| DataError::InvalidInput(e.to_string()))?;
    let mut lms_links = HashMap::new();
    for lms in lms_courses {
        if let (Some(lms_id), Some(id)) = (lms["lmsId"].as_u64(), lms_course_id(transaction, lms)?)
        {
            lms_links.insert(lms_id, id);
        }
    }
    let registrations = read_visible_rows(transaction, "registrations")?;
    let mut offering_ids: BTreeSet<&str> = lms_courses
        .iter()
        .map(|lms| text(lms, "offeringId"))
        .collect();
    offering_ids.extend(
        array(&workspace, "courses")
            .iter()
            .map(|course| text(course, "offeringId")),
    );
    offering_ids.remove("");
    let catalog = offerings_by_id(transaction, offering_ids)?;
    project_lms_timetable(
        &mut workspace,
        &catalog,
        lms_courses,
        &registrations,
        &user_courses,
        &teaching,
        (&mut lms_links, &mut pushed),
    )?;
    for (lms_id, id) in &lms_links {
        if let Some(course) = workspace["courses"]
            .as_array_mut()
            .into_iter()
            .flatten()
            .find(|course| text(course, "id") == id)
        {
            if course["sourceKey"].is_null() {
                course["sourceKey"] = format!("lms:default:course:{lms_id}").into();
            }
            if course["lmsCourseId"]
                .as_u64()
                .is_none_or(|current| *lms_id > current)
            {
                course["lmsCourseId"] = json!(lms_id);
            }
        }
    }
    // Every quarter uses the same cached bilingual metadata. Display language
    // is a read projection, never a mutation of the school's canonical title.
    for course in workspace["courses"].as_array_mut().into_iter().flatten() {
        if course["source"]["provider"] == "manual" {
            continue;
        }
        if let Some(offering) = catalog.get(text(course, "offeringId")) {
            if !user_courses.contains(text(course, "id")) {
                if text(course, "code").is_empty() {
                    if let Some(code) = offering["code"].as_str().filter(|code| !code.is_empty()) {
                        course["code"] = code.into();
                    }
                }
                if let Some(language) = offering["instructionLanguage"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                {
                    course["instructionLanguage"] = language.into();
                }
                if ["lecture", "exercise", "lab"]
                    .iter()
                    .all(|key| offering["credits"][key].is_number())
                {
                    course["creditSplit"] = offering["credits"].clone();
                    course["credits"] = json!(["lecture", "exercise", "lab"]
                        .iter()
                        .filter_map(|key| offering["credits"][key].as_f64())
                        .sum::<f64>());
                }
            }
            course["names"]["instructor"] = crate::workspace::catalog_instructor_names(offering);
            if offering["title"].is_object() {
                let mut names = course["names"]["title"]
                    .as_object()
                    .cloned()
                    .unwrap_or_default();
                for (language, title) in offering["title"].as_object().into_iter().flatten() {
                    names.insert(language.clone(), title.clone());
                }
                course["names"]["title"] = Value::Object(names);
            }
        }
        if let Some(lms) = lms_courses
            .iter()
            .find(|l| l["lmsId"] == course["lmsCourseId"] && l["lmsId"].is_number())
        {
            for language in ["ja", "en"] {
                if course["names"]["title"][language]
                    .as_str()
                    .is_none_or(str::is_empty)
                {
                    if let Some(title) = lms["title"][language].as_str().filter(|s| !s.is_empty()) {
                        course["names"]["title"][language] = title.into();
                    }
                }
            }
        }
        if course["names"].is_object() && course["names"]["instructor"].is_null() {
            course["names"]["instructor"] = json!({});
        }
    }

    let origin: HashMap<&str, &str> = pushed
        .iter()
        .filter_map(|pushed| match pushed {
            Pushed::Course(lms, id) => Some((id.as_str(), lms.as_str())),
            _ => None,
        })
        .collect();
    let in_timetable: HashSet<&str> = timetable.iter().map(|row| text(row, "id")).collect();
    let mut rows: Vec<(&str, String, String, String)> = Vec::new();
    for course in array(&workspace, "courses") {
        let id = text(course, "id");
        let order = if in_timetable.contains(id) {
            format!("0:{id}")
        } else if user_courses.contains(id) {
            format!("1:{id}")
        } else {
            format!("2:{}", origin.get(id).copied().unwrap_or(id))
        };
        rows.push((
            "course",
            order,
            id.to_owned(),
            serde_json::to_string(course)?,
        ));
    }
    for (index, pushed) in pushed.iter().enumerate() {
        match pushed {
            Pushed::Membership(lms, membership) => rows.push((
                "membership",
                format!("{lms}\u{0}{index:08}"),
                text(membership, "courseId").to_owned(),
                serde_json::to_string(membership)?,
            )),
            Pushed::Term(lms, term, id) => rows.push((
                "term",
                format!("{lms}\u{0}{index:08}"),
                id.clone(),
                serde_json::to_string(term)?,
            )),
            Pushed::Course(..) => {}
        }
    }
    // Only rows that changed are written, so a merge that finds the same
    // courses commits nothing and announces nothing.
    let mut stored: HashMap<(String, String), (String, String)> = HashMap::new();
    {
        let mut statement = transaction.prepare_cached(match ids {
            None => "SELECT kind, ord, course_id, value FROM cache.merged_rows WHERE ?1 IS NULL",
            Some(_) => {
                "SELECT kind, ord, course_id, value FROM cache.merged_rows WHERE course_id=?1"
            }
        })?;
        let mut keep = |course: Option<&str>| -> Result<(), DataError> {
            let found = statement
                .query_map([course], |row| {
                    Ok(((row.get(0)?, row.get(1)?), (row.get(2)?, row.get(3)?)))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            stored.extend(found);
            Ok(())
        };
        match ids {
            None => keep(None)?,
            Some(ids) => {
                for id in ids {
                    keep(Some(id))?;
                }
            }
        }
    }
    let mut insert = transaction.prepare_cached(
        "INSERT OR REPLACE INTO cache.merged_rows(kind, ord, course_id, value) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (kind, order, id, value) in rows {
        let key = (kind.to_owned(), order);
        if stored
            .remove(&key)
            .is_some_and(|old| old == (id.clone(), value.clone()))
        {
            continue;
        }
        insert.execute(params![kind, key.1, id, value])?;
    }
    let mut delete =
        transaction.prepare_cached("DELETE FROM cache.merged_rows WHERE kind=?1 AND ord=?2")?;
    for ((kind, order), _) in stored {
        delete.execute(params![kind, order])?;
    }
    Ok(())
}

/// The merged rows of one kind, in merge order.
pub(super) fn read(connection: &Connection, kind: &str) -> Result<Vec<Value>, DataError> {
    let mut statement = connection
        .prepare_cached("SELECT value FROM cache.merged_rows WHERE kind=?1 ORDER BY ord")?;
    let rows = statement
        .query_map([kind], |row| row.get::<_, String>(0))?
        .map(|raw| {
            serde_json::from_str(&raw?)
                .map_err(|_| DataError::storage("merged_rows contains invalid JSON"))
        })
        .collect();
    rows
}

/// The visible catalog offerings `ids` name, by id.
pub(super) fn offerings_by_id<'a>(
    connection: &Connection,
    ids: impl IntoIterator<Item = &'a str>,
) -> Result<HashMap<String, Value>, DataError> {
    let mut statement = connection
        .prepare_cached("SELECT value FROM offerings WHERE id = ?1 AND missing_since IS NULL")?;
    let mut catalog = HashMap::new();
    for id in ids {
        let Some(raw) = statement
            .query_row([id], |row| row.get::<_, String>(0))
            .optional()?
        else {
            continue;
        };
        let mut offering: Value = serde_json::from_str(&raw)
            .map_err(|_| DataError::storage("offerings contains invalid JSON"))?;
        normalize_optional_nulls(&mut offering);
        catalog.insert(id.to_owned(), offering);
    }
    Ok(catalog)
}

/// Project cached LMS quarter grids without rewriting user records.
fn project_lms_timetable(
    workspace: &mut Value,
    offerings: &HashMap<String, Value>,
    lms_courses: &[Value],
    registrations: &[Value],
    user_courses: &HashSet<String>,
    teaching: &[crate::schedule::TeachingPeriod],
    (links, pushed): (&mut HashMap<u64, String>, &mut Vec<Pushed>),
) -> Result<(), DataError> {
    for lms in lms_courses
        .iter()
        .filter(|row| row["hidden"].as_bool() != Some(true))
    {
        let (Some(lms_id), Some(year)) = (lms["lmsId"].as_u64(), lms["year"].as_i64()) else {
            continue;
        };
        if !(2000..2100).contains(&year) {
            continue;
        }
        let offering_id = text(lms, "offeringId");
        let catalog = offerings
            .get(offering_id)
            .filter(|_| !offering_id.is_empty())
            .cloned();
        let mut terms = lms_terms(lms);
        terms.retain(|term| term.starts_with(&format!("{year}-")));
        terms.sort();
        let Some(first_term) = terms.first() else {
            continue;
        };
        let id = links.get(&lms_id).cloned().unwrap_or_else(|| {
            if offering_id.len() == 9 && offering_id.bytes().all(|b| b.is_ascii_digit()) {
                format!("{year}-{offering_id}")
            } else {
                format!("{year}-lms-{lms_id}")
            }
        });
        let existing = array(workspace, "courses")
            .iter()
            .position(|c| text(c, "id") == id);
        let mut course = if let Some(index) = existing {
            workspace["courses"][index].clone()
        } else if let Some(offering) = &catalog {
            crate::workspace::planned_catalog_course(
                offering,
                &id,
                first_term,
                crate::workspace::workspace_content_locale(workspace),
                teaching,
            )?
        } else {
            json!({"id":id,"kind":"course","termId":first_term,
                "code":lms_course_code(text(lms,"shortname")),
                "title":crate::schedule::localized(Some(&lms["title"]), "ja", ""),
                "names":{"title":lms["title"],"instructor":{}},"instructor":"",
                "credits":0,"creditSplit":{"lecture":0,"exercise":0,"lab":0},
                "instructionLanguage":"other","color":"#526e91","meetings":[],"url":lms["url"]})
        };
        if existing.is_none() {
            course["sourceKey"] = lms["sourceKey"].clone();
            course["source"] = json!({"provider":"lms","externalId":lms_id.to_string()});
            for field in ["rev", "updatedAt", "updatedBy"] {
                course[field] = lms[field].clone();
            }
            if !offering_id.is_empty() {
                course["offeringId"] = offering_id.into();
            }
        }
        // The catalog code is often only the base code; the linked LMS
        // shortname retains the school's numeric class suffix.
        let same_offering = !offering_id.is_empty() && text(&course, "offeringId") == offering_id;
        let own_lms_record = course["source"]["provider"] == "lms"
            && text(&course["source"], "externalId") == lms_id.to_string();
        if !user_courses.contains(&id)
            && course["source"]["provider"] != "manual"
            && (same_offering || own_lms_record)
        {
            let code = lms_course_code(text(lms, "shortname"));
            if !code.is_empty() {
                course["code"] = code.into();
            }
        }
        // A user-owned course retains its personal clocks. Cached school
        // clocks update only for quarters successfully captured by LMS.
        if existing.is_none() {
            course["meetings"] = json!([]);
        }
        for term in &terms {
            if !user_courses.contains(&id) && course["source"]["provider"] != "manual" {
                if let Some(grid) = lms["timetables"].get(term).filter(|v| v.is_array()) {
                    let metadata = catalog
                        .as_ref()
                        .map(|offering| {
                            crate::workspace::planned_catalog_course(
                                offering,
                                &id,
                                term,
                                crate::workspace::workspace_content_locale(workspace),
                                teaching,
                            )
                        })
                        .transpose()?
                        .map(|c| c["meetings"].clone())
                        .unwrap_or_else(|| course["meetings"].clone());
                    let mut meetings =
                        crate::workspace::meetings_from_grid(grid, &metadata, teaching)?;
                    let offering = Some(offering_id)
                        .filter(|id| !id.is_empty())
                        .unwrap_or_else(|| text(&course, "offeringId"));
                    for meeting in meetings.as_array_mut().into_iter().flatten() {
                        meeting["termIds"] = json!([term]);
                        if text(meeting, "room").is_empty() {
                            if let Some(room) =
                                registration_room(registrations, offering, term, meeting)
                            {
                                meeting["room"] = room.into();
                            }
                        }
                    }
                    let clocks = course["meetings"]
                        .as_array_mut()
                        .ok_or_else(|| DataError::storage("Missing meetings."))?;
                    clocks.retain(|m| {
                        m["termIds"]
                            .as_array()
                            .is_some_and(|ids| !ids.contains(&json!(term)))
                    });
                    clocks.extend(meetings.as_array().into_iter().flatten().cloned());
                }
            }
            // Existing local placement (including a tombstone) wins over inference.
            if !array(workspace, "memberships")
                .iter()
                .chain(array(workspace, "planItems"))
                .any(|m| text(m, "courseId") == id && text(m, "termId") == term)
            {
                let membership = json!({"id":format!("lms-membership:{lms_id}:{term}"),"kind":"membership",
                    "courseId":id,"termId":term,"status":"lms-member",
                    "evidence":{"provider":"lms","externalId":lms_id.to_string()},
                    "rev":lms["rev"],"updatedAt":lms["updatedAt"],"updatedBy":"lms"});
                pushed.push(Pushed::Membership(
                    text(lms, "id").to_owned(),
                    membership.clone(),
                ));
                workspace["memberships"]
                    .as_array_mut()
                    .ok_or_else(|| DataError::storage("Missing memberships."))?
                    .push(membership);
            }
            if !array(workspace, "terms").contains(&json!(term)) {
                pushed.push(Pushed::Term(
                    text(lms, "id").to_owned(),
                    term.clone(),
                    id.clone(),
                ));
                workspace["terms"]
                    .as_array_mut()
                    .ok_or_else(|| DataError::storage("Missing terms."))?
                    .push(json!(term));
            }
        }
        if let Some(index) = existing {
            workspace["courses"][index] = course;
        } else {
            pushed.push(Pushed::Course(text(lms, "id").to_owned(), id.clone()));
            workspace["courses"]
                .as_array_mut()
                .ok_or_else(|| DataError::storage("Missing courses."))?
                .push(course);
        }
        links.insert(lms_id, id);
    }
    Ok(())
}

/// The 教務 registration room for an LMS slot without a syllabus room. Only
/// the same offering, quarter and weekday with overlapping periods counts, so
/// a disagreeing official schedule never lends its room to another slot.
fn registration_room<'a>(
    registrations: &'a [Value],
    offering_id: &str,
    term: &str,
    meeting: &Value,
) -> Option<&'a str> {
    const DAYS: [&str; 7] = ["日", "月", "火", "水", "木", "金", "土"];
    let quarter = term.split_once('-')?.1;
    let day = DAYS.get(usize::try_from(array(meeting, "days").first()?.as_u64()?).ok()?)?;
    let span = |row: &Value| {
        let periods = array(row, "periods").iter().filter_map(Value::as_u64);
        periods.clone().min().zip(periods.max())
    };
    let (from, to) = span(meeting)?;
    if offering_id.is_empty() {
        return None;
    }
    registrations
        .iter()
        .filter(|row| text(row, "offeringId") == offering_id)
        .flat_map(|row| array(row, "schedule"))
        .find(|slot| {
            text(slot, "quarter") == quarter
                && text(slot, "day") == *day
                && span(slot).is_some_and(|(start, end)| start <= to && from <= end)
        })
        .map(|slot| text(slot, "room"))
        .filter(|room| !room.is_empty())
}

/// Quarters an LMS course is on. Category metadata bootstraps old caches; a
/// captured grid then owns its quarter: an array means present (including
/// non-weekly), null means confirmed absent.
fn lms_terms(lms: &Value) -> Vec<String> {
    let timetables = &lms["timetables"];
    let captured = timetables
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, grid)| grid.is_array())
        .map(|(term, _)| term.clone());
    let category = lms["year"].as_i64().into_iter().flat_map(|year| {
        array(lms, "quarters")
            .iter()
            .filter_map(Value::as_str)
            .map(move |quarter| format!("{year}-{quarter}"))
    });
    let mut terms: Vec<String> = captured
        .chain(category)
        .filter(|term| {
            crate::schedule::Term::parse(term).is_ok()
                && !timetables.get(term.as_str()).is_some_and(Value::is_null)
        })
        .collect();
    terms.sort();
    terms.dedup();
    terms
}
