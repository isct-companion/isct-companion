use crate::data::{resolve_course_id, versioned::content_hash, DataError};
use crate::school_parser::lms::{id_of, lms_url, AssignState, LmsPayload};
use chrono::{Datelike, Utc};
use rusqlite::{params, OptionalExtension, Transaction};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("{0}")]
    InvalidPayload(String),
    #[error("{0}")]
    Storage(String),
}

impl From<String> for IngestError {
    fn from(message: String) -> Self {
        Self::InvalidPayload(message)
    }
}

impl From<&str> for IngestError {
    fn from(message: &str) -> Self {
        Self::InvalidPayload(message.to_string())
    }
}

impl From<DataError> for IngestError {
    fn from(error: DataError) -> Self {
        Self::Storage(error.to_string())
    }
}

impl From<rusqlite::Error> for IngestError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

impl From<serde_json::Error> for IngestError {
    fn from(error: serde_json::Error) -> Self {
        Self::InvalidPayload(error.to_string())
    }
}

fn split_title(fullname: &str, language: &str) -> Value {
    if let Some((ja, en)) = fullname.split_once(" / ") {
        return json!({"ja": ja.trim(), "en": en.trim()});
    }
    if language.to_ascii_lowercase().starts_with("ja") {
        json!({"ja": fullname.trim()})
    } else {
        json!({"en": fullname.trim()})
    }
}

fn year_of(shortname: &str) -> Option<u32> {
    let open = shortname.rfind('[')?;
    shortname
        .get(open + 1..shortname.len().checked_sub(1)?)?
        .parse()
        .ok()
}

/// The LMS category name is the quarter the course starts in (verified
/// 2026-10-03 for 2025 and 2026 courses); anything else is unplaced.
fn quarter_of(_year: Option<u32>, category: u64) -> Vec<String> {
    match category {
        1..=4 => vec![format!("{category}Q")],
        _ => Vec::new(),
    }
}

fn upsert_versioned(
    tx: &Transaction<'_>,
    table: &str,
    id: &str,
    value: &Value,
    columns: &[(&str, rusqlite::types::Value)],
) -> Result<bool, IngestError> {
    let hash = content_hash(value)?;
    let existing: Option<(String, Option<String>)> = tx
        .query_row(
            &format!("SELECT hash, missing_since FROM {table} WHERE id=?1"),
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(IngestError::from)?;
    let timestamp = Utc::now().to_rfc3339();
    if existing.as_ref().is_some_and(|(stored, _)| stored == &hash) {
        let changed = existing
            .as_ref()
            .is_some_and(|(_, missing)| missing.is_some());
        tx.execute(
            &format!("UPDATE {table} SET last_seen_at=?2, missing_since=NULL, deleted_at=NULL,
                value=json_remove(json_set(value,'$.lastSeenAt',?2),'$.missingSince','$.deletedAt') WHERE id=?1"),
            params![id, timestamp],
        ).map_err(IngestError::from)?;
        return Ok(changed);
    }
    let mut names = vec![
        "id",
        "value",
        "hash",
        "updated_at",
        "updated_by",
        "rev",
        "source",
        "deleted_at",
        "last_seen_at",
        "missing_since",
    ];
    names.extend(columns.iter().map(|(name, _)| *name));
    let marks = (1..=names.len())
        .map(|n| format!("?{n}"))
        .collect::<Vec<_>>()
        .join(",");
    let updates = names
        .iter()
        .skip(1)
        .map(|name| format!("{name}=excluded.{name}"))
        .collect::<Vec<_>>()
        .join(",");
    let old_rev: u64 = tx
        .query_row(&format!("SELECT rev FROM {table} WHERE id=?1"), [id], |r| {
            r.get(0)
        })
        .optional()
        .map_err(IngestError::from)?
        .unwrap_or(0);
    let rev = old_rev + 1;
    let mut stored = value.clone();
    if let Some(object) = stored.as_object_mut() {
        object.insert("rev".into(), rev.into());
        object.insert("updatedAt".into(), timestamp.clone().into());
        object.insert("updatedBy".into(), "lms".into());
        object.insert("source".into(), json!({"provider":"lms"}));
        object.insert("lastSeenAt".into(), timestamp.clone().into());
        object.remove("missingSince");
        object.remove("deletedAt");
    }
    let json = serde_json::to_string(&stored).map_err(IngestError::from)?;
    let mut values = vec![
        id.to_string().into(),
        json.into(),
        hash.into(),
        timestamp.clone().into(),
        String::from("lms").into(),
        (rev as i64).into(),
        String::from("lms").into(),
        rusqlite::types::Value::Null,
        timestamp.clone().into(),
        rusqlite::types::Value::Null,
    ];
    values.extend(columns.iter().map(|(_, value)| value.clone()));
    tx.execute(
        &format!(
            "INSERT INTO {table} ({}) VALUES ({marks}) ON CONFLICT(id) DO UPDATE SET {updates}",
            names.join(",")
        ),
        rusqlite::params_from_iter(values),
    )
    .map_err(IngestError::from)?;
    Ok(true)
}

struct AssignmentSource<'a> {
    cmid: u64,
    lms_course: u64,
    course: &'a str,
    title: &'a str,
    url: &'a str,
}

/// What the stored row already says, for a pull that could not read the page.
struct PriorAssignment {
    submission: String,
    due_at: Option<String>,
    grade: Option<String>,
    accepts_submission: Option<bool>,
}

/// One assignment row. `open` while Moodle lists it as work to do; a
/// previously open one that left that list before its deadline is submitted.
fn write_assignment(
    tx: &Transaction<'_>,
    source: &AssignmentSource<'_>,
    due: Option<i64>,
    listed_open: bool,
    page_state: Option<&AssignState>,
) -> Result<bool, IngestError> {
    let cmid = source.cmid;
    let prior = tx
        .query_row(
            "SELECT submission, due_at, json_extract(value,'$.grade'), json_extract(value,'$.acceptsSubmission') FROM assignments WHERE lms_cmid=?1 AND missing_since IS NULL",
            [cmid],
            |r| {
                Ok(PriorAssignment {
                    submission: r.get(0)?,
                    due_at: r.get(1)?,
                    grade: r.get(2)?,
                    accepts_submission: r.get(3)?,
                })
            },
        )
        .optional()
        .map_err(IngestError::from)?;
    let effective_due = due.or_else(|| {
        prior
            .as_ref()
            .and_then(|value| value.due_at.as_deref())
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.timestamp())
    });
    // The assignment page is authoritative; the event list is only a hint.
    // Keep the last known due date when Moodle temporarily omits its event.
    let submission = if let Some(state) = page_state {
        state.submission.as_str()
    } else if listed_open {
        "open"
    } else if prior.as_ref().map(|value| value.submission.as_str()) == Some("open")
        && effective_due.is_some_and(|d| d > Utc::now().timestamp())
    {
        "submitted"
    } else {
        // A pull that could not read the page keeps finished work finished.
        prior
            .as_ref()
            .map(|value| value.submission.as_str())
            .filter(|p| matches!(*p, "submitted" | "graded"))
            .unwrap_or("unknown")
    };
    let due_at = due
        .and_then(|d| chrono::DateTime::from_timestamp(d, 0))
        .map(|d| d.to_rfc3339())
        .or_else(|| prior.as_ref().and_then(|value| value.due_at.clone()));
    // The page's grade when it was read; otherwise the last one seen, while
    // the work still counts as graded.
    let grade = match page_state {
        Some(state) => state.grade.clone(),
        None => prior
            .as_ref()
            .and_then(|value| value.grade.clone())
            .filter(|_| submission == "graded"),
    };
    let mut assignment = json!({"id":format!("lms-assignment:{cmid}"),"sourceKey":format!("lms:default:assignment:{cmid}"),"kind":"assignment","courseId":source.course,
        "lmsCmid":cmid,"lmsCourseId":source.lms_course,"title":source.title,"details":"","sourceUrl":source.url,
        "dueAt":due_at,"submission":submission});
    if let Some(grade) = grade {
        assignment["grade"] = grade.into();
    }
    // Whether LMS still takes work past the deadline; the last page read wins.
    let accepts = match page_state {
        Some(state) => state.accepts_submission,
        None => prior.as_ref().and_then(|value| value.accepts_submission),
    };
    if let Some(accepts) = accepts {
        assignment["acceptsSubmission"] = accepts.into();
    }
    upsert_versioned(
        tx,
        "assignments",
        &format!("lms-assignment:{cmid}"),
        &assignment,
        &[
            ("title", source.title.to_string().into()),
            (
                "due_at",
                due_at
                    .map(Into::into)
                    .unwrap_or(rusqlite::types::Value::Null),
            ),
            ("source_url", source.url.to_string().into()),
            ("lms_cmid", (cmid as i64).into()),
            ("lms_course_id", (source.lms_course as i64).into()),
            ("submission", submission.to_string().into()),
        ],
    )
}

/// Projects one LMS payload inside the caller's transaction; the run
/// commits or rolls back as a whole.
pub fn apply_lms_payload(tx: &Transaction<'_>, payload: &LmsPayload) -> Result<usize, IngestError> {
    let mut changed = 0;
    let run_id = Uuid::new_v4().to_string();
    let started = Utc::now().to_rfc3339();
    let scope = if payload.full_scan
        && payload.courses_complete
        && payload.timetable_errors.is_empty()
        && payload
            .structures
            .iter()
            .all(|structure| structure.complete)
    {
        "all"
    } else {
        "summary"
    };
    tx.execute("INSERT INTO import_runs(id,provider,scope,state,started_at) VALUES(?1,'lms',?2,'running',?3)", params![run_id, scope, started]).map_err(IngestError::from)?;
    let action_ids: HashSet<u64> = payload
        .action_events
        .iter()
        .filter_map(|v| id_of(v.get("instance")))
        .collect();
    let dated: HashMap<u64, i64> = payload
        .dated_events
        .iter()
        .filter(|v| {
            matches!(
                v.get("eventtype").and_then(Value::as_str),
                Some("due" | "close")
            )
        })
        .filter_map(|v| Some((id_of(v.get("instance"))?, v.get("timesort")?.as_i64()?)))
        .collect();
    // Due dates: the calendar's due event, else the open-work event's time.
    let action_due: HashMap<u64, i64> = payload
        .action_events
        .iter()
        .filter_map(|v| Some((id_of(v.get("instance"))?, v.get("timesort")?.as_i64()?)))
        .collect();
    let due_of = |cmid: u64| dated.get(&cmid).or_else(|| action_due.get(&cmid)).copied();
    let mut written = HashSet::<u64>::new();
    let mut course_ids = HashMap::<u64, String>::new();
    for course in &payload.courses {
        let year = year_of(&course.shortname);
        let offering = (course.idnumber.len() == 9
            && course.idnumber.bytes().all(|b| b.is_ascii_digit()))
        .then(|| course.idnumber.clone());
        let lms_course_id = format!("lms-course:{}", course.id);
        // Weekly meetings by term from the dashboard grid; terms this pull did
        // not show keep what an earlier pull saw.
        let mut timetables = tx
            .query_row(
                "SELECT value FROM lms_courses WHERE id=?1",
                [&lms_course_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(IngestError::from)?
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .and_then(|stored| stored.get("timetables").cloned())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        for grid in payload.timetables.iter().chain(payload.timetable.iter()) {
            let shown = grid
                .meetings
                .get(&course.id)
                .cloned()
                .or_else(|| grid.unscheduled.contains(&course.id).then(Vec::new));
            if year.is_some_and(|year| grid.term.starts_with(&format!("{year}-"))) {
                if let Some(map) = timetables.as_object_mut() {
                    map.insert(
                        grid.term.clone(),
                        shown.map(Value::Array).unwrap_or(Value::Null),
                    );
                }
            }
        }
        let prior_sections: Option<Value> = tx
            .query_row(
                "SELECT value FROM lms_courses WHERE id=?1",
                [&lms_course_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(IngestError::from)?
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .and_then(|stored| stored.get("sections").cloned());
        let mut sections = payload
            .structures
            .iter()
            .find(|s| s.course_id == course.id && s.complete)
            .and_then(|s| s.state.as_ref())
            .and_then(|s| s.get("section"))
            .cloned()
            .or_else(|| prior_sections.clone())
            .unwrap_or_else(|| json!([]));
        if let Some(current) = sections.as_array_mut() {
            for section in current {
                if section.get("bodyHtml").is_none() {
                    if let Some(body) = prior_sections
                        .as_ref()
                        .and_then(Value::as_array)
                        .and_then(|old| {
                            old.iter()
                                .find(|s| id_of(s.get("id")) == id_of(section.get("id")))
                        })
                        .and_then(|s| s.get("bodyHtml"))
                    {
                        if let Some(section) = section.as_object_mut() {
                            section.insert("bodyHtml".into(), body.clone());
                        }
                    }
                }
            }
        }
        let value = json!({"id":lms_course_id,"sourceKey":format!("lms:default:course:{}",course.id),"lmsId":course.id,"offeringId":offering,
            "shortname":course.shortname,"title":split_title(&course.fullname,&payload.language),"year":year,
            "quarters":quarter_of(year,course.coursecategory),"url":course.viewurl,"hidden":course.hidden,
            "timetables":timetables,"sections":sections});
        changed += usize::from(upsert_versioned(
            tx,
            "lms_courses",
            &lms_course_id,
            &value,
            &[("lms_id", (course.id as i64).into())],
        )?);
        let offering_id = value.get("offeringId").and_then(Value::as_str);
        let lms_id = course.id.to_string();
        let academic_year = year.unwrap_or_else(|| Utc::now().year() as u32) as i32;
        let course_id = resolve_course_id(tx, "lms", &lms_id, academic_year, offering_id)?;
        if let Some(offering_id) = offering_id {
            resolve_course_id(
                tx,
                "syllabus",
                offering_id,
                academic_year,
                Some(offering_id),
            )?;
        }
        course_ids.insert(course.id, course_id);
    }
    if payload.courses_complete {
        let live: HashSet<String> = payload
            .courses
            .iter()
            .map(|c| format!("lms-course:{}", c.id))
            .collect();
        let mut statement = tx
            .prepare("SELECT id FROM lms_courses WHERE missing_since IS NULL")
            .map_err(IngestError::from)?;
        let old = statement
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(IngestError::from)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(IngestError::from)?;
        drop(statement);
        for id in old.into_iter().filter(|id| !live.contains(id)) {
            let missing = Utc::now().to_rfc3339();
            changed += tx.execute("UPDATE lms_courses SET missing_since=?2,rev=rev+1,value=json_set(value,'$.missingSince',?2,'$.rev',rev+1,'$.updatedAt',?2) WHERE id=?1 AND missing_since IS NULL",params![id,missing]).map_err(IngestError::from)?;
        }
    }
    for structure in payload.structures.iter().filter(|s| s.complete) {
        let mut live_assignments = HashSet::new();
        let state = structure
            .state
            .as_ref()
            .ok_or("complete LMS structure has no state")?;
        let sections: HashMap<u64, (&str, i64)> = state
            .get("section")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|s| {
                Some((
                    id_of(s.get("id"))?,
                    (
                        s.get("title").and_then(Value::as_str).unwrap_or(""),
                        s.get("number").and_then(Value::as_i64).unwrap_or(0),
                    ),
                ))
            })
            .collect();
        let cms = state
            .get("cm")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut live = HashSet::new();
        for cm in cms {
            let Some(cmid) = id_of(cm.get("id")) else {
                continue;
            };
            let module = cm.get("module").and_then(Value::as_str).unwrap_or("");
            let url = cm.get("url").and_then(Value::as_str).unwrap_or("");
            if !url.is_empty() && !lms_url(url) {
                return Err("LMS item URL failed validation".into());
            }
            let section = id_of(cm.get("sectionid"))
                .and_then(|id| sections.get(&id).copied())
                .unwrap_or(("", 0));
            let item_id = format!("lms-item:{cmid}");
            live.insert(item_id.clone());
            let linked_course = course_ids
                .get(&structure.course_id)
                .cloned()
                .ok_or_else(|| "LMS structure referenced an unknown course".to_string())?;
            let name = cm.get("name").and_then(Value::as_str).unwrap_or("");
            let previous: Option<Value> = tx
                .query_row("SELECT value FROM lms_items WHERE id=?1", [&item_id], |r| {
                    r.get::<_, String>(0)
                })
                .optional()
                .map_err(IngestError::from)?
                .and_then(|raw| serde_json::from_str(&raw).ok());
            let body = cm
                .get("bodyHtml")
                .or_else(|| previous.as_ref().and_then(|v| v.get("bodyHtml")));
            let resource_url = cm
                .get("resourceUrl")
                .or_else(|| previous.as_ref().and_then(|v| v.get("resourceUrl")));
            // A folder's files come from opening it, not from the crawl; keep them.
            let folder_files = previous.as_ref().and_then(|v| v.get("folderFiles"));
            let item = json!({"id":item_id,"sourceKey":format!("lms:default:material:{cmid}"),"kind":"material","courseId":linked_course,"cmid":cmid,"lmsCourseId":structure.course_id,"sectionNumber":section.1,
                "sectionId":id_of(cm.get("sectionid")),"bodyHtml":body,"resourceUrl":resource_url,"folderFiles":folder_files,"sectionTitle":section.0,"module":module,"name":cm.get("name").and_then(Value::as_str).unwrap_or(""),
                "session":section.1,"title":name,"type":"link","url":url,"visible":cm.get("visible").and_then(Value::as_bool).unwrap_or(false) && cm.get("uservisible").and_then(Value::as_bool).unwrap_or(false)});
            changed += usize::from(upsert_versioned(
                tx,
                "lms_items",
                &item_id,
                &item,
                &[("cmid", (cmid as i64).into())],
            )?);
            if matches!(module, "assign" | "quiz") {
                let title = cm.get("name").and_then(Value::as_str).unwrap_or("");
                changed += usize::from(write_assignment(
                    tx,
                    &AssignmentSource {
                        cmid,
                        lms_course: structure.course_id,
                        course: &linked_course,
                        title,
                        url,
                    },
                    due_of(cmid),
                    action_ids.contains(&cmid),
                    payload.assign_status.get(&cmid),
                )?);
                written.insert(cmid);
                live_assignments.insert(format!("lms-assignment:{cmid}"));
            }
        }
        let mut statement = tx.prepare("SELECT id FROM lms_items WHERE json_extract(value,'$.lmsCourseId')=?1 AND missing_since IS NULL").map_err(IngestError::from)?;
        let old = statement
            .query_map([structure.course_id], |r| r.get::<_, String>(0))
            .map_err(IngestError::from)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(IngestError::from)?;
        drop(statement);
        for id in old.into_iter().filter(|id| !live.contains(id)) {
            let missing = Utc::now().to_rfc3339();
            changed += tx.execute("UPDATE lms_items SET missing_since=?2,rev=rev+1,value=json_set(value,'$.missingSince',?2,'$.rev',rev+1,'$.updatedAt',?2) WHERE id=?1 AND missing_since IS NULL",params![id,missing]).map_err(IngestError::from)?;
        }
        let mut statement = tx
            .prepare("SELECT id FROM assignments WHERE lms_course_id=?1 AND missing_since IS NULL")
            .map_err(IngestError::from)?;
        let old = statement
            .query_map([structure.course_id], |r| r.get::<_, String>(0))
            .map_err(IngestError::from)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(IngestError::from)?;
        drop(statement);
        for id in old.into_iter().filter(|id| !live_assignments.contains(id)) {
            let missing = Utc::now().to_rfc3339();
            changed += tx.execute("UPDATE assignments SET missing_since=?2,rev=rev+1,value=json_set(value,'$.missingSince',?2,'$.rev',rev+1,'$.updatedAt',?2) WHERE id=?1 AND missing_since IS NULL",params![id,missing]).map_err(IngestError::from)?;
        }
    }
    // Open work in courses whose structure was not loaded (other quarters).
    for event in &payload.action_events {
        let Some(cmid) = id_of(event.get("instance")) else {
            continue;
        };
        let module = event
            .get("modulename")
            .and_then(Value::as_str)
            .unwrap_or("");
        let url = event.get("url").and_then(Value::as_str).unwrap_or("");
        if written.contains(&cmid)
            || !matches!(module, "assign" | "quiz")
            || (!url.is_empty() && !lms_url(url))
        {
            continue;
        }
        let Some(lms_course) = id_of(event.get("course").and_then(|course| course.get("id")))
        else {
            continue;
        };
        let Some(course) = course_ids.get(&lms_course).cloned() else {
            continue;
        };
        let title = event
            .get("activityname")
            .or_else(|| event.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("");
        changed += usize::from(write_assignment(
            tx,
            &AssignmentSource {
                cmid,
                lms_course,
                course: &course,
                title,
                url,
            },
            due_of(cmid),
            true,
            payload.assign_status.get(&cmid),
        )?);
        written.insert(cmid);
    }
    tx.execute(
        "UPDATE import_runs SET state='complete',finished_at=?2,message=?3 WHERE id=?1",
        params![
            run_id,
            Utc::now().to_rfc3339(),
            format!("{changed} rows changed")
        ],
    )
    .map_err(IngestError::from)?;
    Ok(changed)
}

#[cfg(test)]
#[path = "tests/lms_ingest.rs"]
mod tests;
