//! LMS rows, one freshness key at a time. Each `apply_lms_*` runs in the
//! caller's transaction, writes its key's rows, marks what its inventory no
//! longer lists as missing and records the key fresh, so a key lands whole
//! or not at all.
use crate::data::{
    resolve_course_id,
    versioned::{content_hash, keep_unchanged},
    DataError,
};
use crate::school_parser::lms::{
    id_of, lms_url, LmsAssignStates, LmsCourseContent, LmsDeadlines, LmsEnrolment, LmsHistoryMonth,
    StoredModule,
};
use crate::sync_state::{self, Commit, Dataset, Due, Key, KeyState, Outcome, Term};
use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("{0}")]
    InvalidPayload(String),
    #[error("{0}")]
    Storage(String),
    /// The stored enrolment list does not hold this course (yet, or any
    /// more): its content waits for the list instead of failing.
    #[error("LMS course {0} is not enrolled")]
    NotEnrolled(u64),
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
) -> Result<bool, IngestError> {
    let hash = content_hash(value)?;
    let existing: Option<(String, bool)> = tx
        .query_row(
            &format!(
                "SELECT hash, missing_since IS NOT NULL OR deleted_at IS NOT NULL
                 FROM {table} WHERE id=?1"
            ),
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(IngestError::from)?;
    let timestamp = Utc::now().to_rfc3339();
    if let Some((stored, gone)) = &existing {
        if stored == &hash {
            return keep_unchanged(tx, table, id, *gone, &timestamp).map_err(IngestError::from);
        }
    }
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
    tx.execute(
        &format!(
            "INSERT INTO {table}
             (id,value,hash,updated_at,updated_by,rev,source,deleted_at,last_seen_at,missing_since)
             VALUES (?1,?2,?3,?4,'lms',?5,'lms',NULL,?4,NULL)
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

/// The fields `upsert_versioned` adds to a stored value; the content hash is
/// computed without them.
const STORED_META: [&str; 7] = [
    "rev",
    "updatedAt",
    "updatedBy",
    "source",
    "lastSeenAt",
    "missingSince",
    "deletedAt",
];

/// A stored value as it was before `upsert_versioned` added its metadata,
/// so writing it back unchanged keeps its hash.
fn content_of(mut stored: Value) -> Value {
    if let Some(object) = stored.as_object_mut() {
        for key in STORED_META {
            object.remove(key);
        }
    }
    stored
}

fn stored_value(tx: &Transaction<'_>, table: &str, id: &str) -> Result<Option<Value>, IngestError> {
    Ok(tx
        .query_row(
            &format!("SELECT value FROM {table} WHERE id=?1"),
            [id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(IngestError::from)?
        .and_then(|raw| serde_json::from_str(&raw).ok()))
}

/// Marks the rows `sql` selects (ids of rows still present) that `live`
/// does not list as missing: the key's inventory no longer has them.
fn mark_missing(
    tx: &Transaction<'_>,
    table: &str,
    sql: &str,
    scope: impl rusqlite::Params,
    live: &HashSet<String>,
) -> Result<usize, IngestError> {
    let mut statement = tx.prepare(sql).map_err(IngestError::from)?;
    let old = statement
        .query_map(scope, |row| row.get::<_, String>(0))
        .map_err(IngestError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(IngestError::from)?;
    drop(statement);
    let missing = Utc::now().to_rfc3339();
    let mut changed = 0;
    for id in old.into_iter().filter(|id| !live.contains(id)) {
        changed += tx
            .execute(
                &format!("UPDATE {table} SET missing_since=?2,rev=rev+1,value=json_set(value,'$.missingSince',?2,'$.rev',rev+1,'$.updatedAt',?2) WHERE id=?1 AND missing_since IS NULL"),
                params![id, missing],
            )
            .map_err(IngestError::from)?;
    }
    Ok(changed)
}

/// Records `key` fresh in the transaction that wrote its rows.
fn commit_key(
    tx: &Transaction<'_>,
    dataset: Dataset,
    key: impl Into<String>,
    account: &str,
    now: DateTime<Utc>,
) -> Result<(), IngestError> {
    sync_state::commit(
        tx,
        &Key::new(dataset, key),
        Commit {
            account,
            validator: None,
            outcome: Outcome::Complete,
            message: None,
        },
        now,
    )?;
    Ok(())
}

/// The internal course id of an enrolled LMS course, from its stored row.
fn linked_course(
    tx: &Transaction<'_>,
    lms_course: u64,
    stored: &Value,
) -> Result<String, IngestError> {
    let offering = stored.get("offeringId").and_then(Value::as_str);
    let year = stored
        .get("year")
        .and_then(Value::as_i64)
        .map(|year| year as i32)
        .unwrap_or_else(|| Utc::now().year());
    Ok(resolve_course_id(
        tx,
        "lms",
        &lms_course.to_string(),
        year,
        offering,
    )?)
}

/// The stored row of an enrolled (not missing) LMS course.
fn enrolled_course(tx: &Transaction<'_>, lms_course: u64) -> Result<Option<Value>, IngestError> {
    Ok(tx
        .query_row(
            "SELECT value FROM lms_courses WHERE id=?1 AND missing_since IS NULL",
            [format!("lms-course:{lms_course}")],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(IngestError::from)?
        .and_then(|raw| serde_json::from_str(&raw).ok()))
}

/// One assignment row. Three keys share it, and each writes only what it
/// owns, merged with the stored row: `lms.course` its existence, title and
/// link; `lms.deadlines` (and `lms.history`) the due date and the open-work
/// hint; `lms.assign` submission, grade and `acceptsSubmission`.
#[derive(Clone, Debug)]
struct Assignment {
    cmid: u64,
    lms_course: u64,
    course: String,
    title: String,
    url: String,
    due_at: Option<String>,
    submission: String,
    grade: Option<String>,
    accepts_submission: Option<bool>,
}

impl Assignment {
    fn new(cmid: u64, lms_course: u64, course: String, title: &str, url: &str) -> Self {
        Self {
            cmid,
            lms_course,
            course,
            title: title.to_string(),
            url: url.to_string(),
            due_at: None,
            submission: "unknown".into(),
            grade: None,
            accepts_submission: None,
        }
    }

    /// The present (not missing) row of `cmid`.
    fn load(tx: &Transaction<'_>, cmid: u64) -> Result<Option<Self>, IngestError> {
        let raw: Option<String> = tx
            .query_row(
                "SELECT value FROM assignments WHERE lms_cmid=?1 AND missing_since IS NULL",
                [cmid],
                |row| row.get(0),
            )
            .optional()
            .map_err(IngestError::from)?;
        let Some(value) = raw.and_then(|raw| serde_json::from_str::<Value>(&raw).ok()) else {
            return Ok(None);
        };
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
        Ok(Some(Self {
            cmid,
            lms_course: id_of(value.get("lmsCourseId")).unwrap_or(0),
            course: text("courseId").unwrap_or_default(),
            title: text("title").unwrap_or_default(),
            url: text("sourceUrl").unwrap_or_default(),
            due_at: text("dueAt"),
            submission: text("submission").unwrap_or_else(|| "unknown".into()),
            grade: text("grade"),
            accepts_submission: value.get("acceptsSubmission").and_then(Value::as_bool),
        }))
    }

    fn due_timestamp(&self) -> Option<i64> {
        self.due_at
            .as_deref()
            .and_then(|due| DateTime::parse_from_rfc3339(due).ok())
            .map(|due| due.timestamp())
    }

    fn set_due(&mut self, timestamp: i64) {
        if let Some(due) = DateTime::from_timestamp(timestamp, 0) {
            self.due_at = Some(due.to_rfc3339());
        }
    }

    fn write(&self, tx: &Transaction<'_>) -> Result<bool, IngestError> {
        let cmid = self.cmid;
        let mut assignment = json!({"id":format!("lms-assignment:{cmid}"),"sourceKey":format!("lms:default:assignment:{cmid}"),"kind":"assignment","courseId":self.course,
            "lmsCmid":cmid,"lmsCourseId":self.lms_course,"title":self.title,"details":"","sourceUrl":self.url,
            "dueAt":self.due_at,"submission":self.submission});
        if let Some(grade) = &self.grade {
            assignment["grade"] = grade.clone().into();
        }
        if let Some(accepts) = self.accepts_submission {
            assignment["acceptsSubmission"] = accepts.into();
        }
        upsert_versioned(
            tx,
            "assignments",
            &format!("lms-assignment:{cmid}"),
            &assignment,
        )
    }
}

/// Commits `lms.enrolment/all`: one row per enrolled course with its
/// weekly meetings by term, and every course the list no longer has marked
/// missing. Sections belong to `lms.course` and are kept.
pub fn apply_lms_enrolment(
    tx: &Transaction<'_>,
    enrolment: &LmsEnrolment,
    account: &str,
    now: DateTime<Utc>,
) -> Result<usize, IngestError> {
    let mut changed = 0;
    let mut live = HashSet::new();
    for course in &enrolment.courses {
        let year = year_of(&course.shortname);
        let offering = (course.idnumber.len() == 9
            && course.idnumber.bytes().all(|b| b.is_ascii_digit()))
        .then(|| course.idnumber.clone());
        let lms_course_id = format!("lms-course:{}", course.id);
        live.insert(lms_course_id.clone());
        let stored = stored_value(tx, "lms_courses", &lms_course_id)?;
        // Weekly meetings by term from the dashboard grids; terms this read
        // did not show keep what an earlier read saw.
        let mut timetables = stored
            .as_ref()
            .and_then(|stored| stored.get("timetables").cloned())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        for grid in &enrolment.timetables {
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
        let sections = stored
            .as_ref()
            .and_then(|stored| stored.get("sections").cloned())
            .unwrap_or_else(|| json!([]));
        let value = json!({"id":lms_course_id,"sourceKey":format!("lms:default:course:{}",course.id),"lmsId":course.id,"offeringId":offering,
            "shortname":course.shortname,"title":split_title(&course.fullname,&enrolment.language),"year":year,
            "quarters":quarter_of(year,course.coursecategory),"url":course.viewurl,"hidden":course.hidden,
            "timetables":timetables,"sections":sections});
        changed += usize::from(upsert_versioned(tx, "lms_courses", &lms_course_id, &value)?);
        let lms_id = course.id.to_string();
        let academic_year = year.unwrap_or_else(|| Utc::now().year() as u32) as i32;
        resolve_course_id(tx, "lms", &lms_id, academic_year, offering.as_deref())?;
    }
    changed += mark_missing(
        tx,
        "lms_courses",
        "SELECT id FROM lms_courses WHERE missing_since IS NULL",
        [],
        &live,
    )?;
    commit_key(tx, Dataset::LmsEnrolment, "all", account, now)?;
    Ok(changed)
}

/// Commits `lms.course/{id}`: the course's sections and every module as one
/// unit. Modules the outline no longer lists, and their assignment rows, are
/// marked missing in the same transaction.
pub fn apply_lms_course(
    tx: &Transaction<'_>,
    content: &LmsCourseContent,
    account: &str,
    now: DateTime<Utc>,
) -> Result<usize, IngestError> {
    let course_id = content.course_id;
    let lms_course_id = format!("lms-course:{course_id}");
    let stored = enrolled_course(tx, course_id)?.ok_or(IngestError::NotEnrolled(course_id))?;
    let linked = linked_course(tx, course_id, &stored)?;
    let mut changed = 0;

    // Sections: the new outline; a section read without a body keeps its
    // stored one.
    let prior_sections = stored.get("sections").cloned();
    let mut sections = content.sections.clone();
    for section in &mut sections {
        if section.get("bodyHtml").is_some() {
            continue;
        }
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
    let mut course = content_of(stored);
    course["sections"] = Value::Array(sections.clone());
    changed += usize::from(upsert_versioned(
        tx,
        "lms_courses",
        &lms_course_id,
        &course,
    )?);

    let section_of: HashMap<u64, (&str, i64)> = sections
        .iter()
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
    let mut live = HashSet::new();
    let mut live_assignments = HashSet::new();
    let mut new_assignment = false;
    for cm in &content.modules {
        let Some(cmid) = id_of(cm.get("id")) else {
            continue;
        };
        let module = cm.get("module").and_then(Value::as_str).unwrap_or("");
        let url = cm.get("url").and_then(Value::as_str).unwrap_or("");
        if !url.is_empty() && !lms_url(url) {
            return Err("LMS item URL failed validation".into());
        }
        let section = id_of(cm.get("sectionid"))
            .and_then(|id| section_of.get(&id).copied())
            .unwrap_or(("", 0));
        let item_id = format!("lms-item:{cmid}");
        live.insert(item_id.clone());
        let name = cm.get("name").and_then(Value::as_str).unwrap_or("");
        let previous = stored_value(tx, "lms_items", &item_id)?;
        let body = cm
            .get("bodyHtml")
            .or_else(|| previous.as_ref().and_then(|v| v.get("bodyHtml")));
        let resource_url = cm
            .get("resourceUrl")
            .or_else(|| previous.as_ref().and_then(|v| v.get("resourceUrl")));
        // A folder's files come from opening it, not from the crawl; keep them.
        let folder_files = previous.as_ref().and_then(|v| v.get("folderFiles"));
        let item = json!({"id":item_id,"sourceKey":format!("lms:default:material:{cmid}"),"kind":"material","courseId":linked,"cmid":cmid,"lmsCourseId":course_id,"sectionNumber":section.1,
            "sectionId":id_of(cm.get("sectionid")),"bodyHtml":body,"resourceUrl":resource_url,"folderFiles":folder_files,"sectionTitle":section.0,"module":module,"name":name,
            "session":section.1,"title":name,"type":"link","url":url,"modified":cm.get("modified"),
            "visible":cm.get("visible").and_then(Value::as_bool).unwrap_or(false) && cm.get("uservisible").and_then(Value::as_bool).unwrap_or(false)});
        changed += usize::from(upsert_versioned(tx, "lms_items", &item_id, &item)?);
        if matches!(module, "assign" | "quiz") {
            let row = match Assignment::load(tx, cmid)? {
                Some(mut row) => {
                    row.lms_course = course_id;
                    row.course = linked.clone();
                    row.title = name.to_string();
                    row.url = url.to_string();
                    row
                }
                None => {
                    new_assignment = true;
                    Assignment::new(cmid, course_id, linked.clone(), name, url)
                }
            };
            changed += usize::from(row.write(tx)?);
            live_assignments.insert(format!("lms-assignment:{cmid}"));
        }
    }
    changed += mark_missing(
        tx,
        "lms_items",
        "SELECT id FROM lms_items WHERE lms_course_id=?1 AND missing_since IS NULL",
        [course_id],
        &live,
    )?;
    changed += mark_missing(
        tx,
        "assignments",
        "SELECT id FROM assignments WHERE lms_course_id=?1 AND missing_since IS NULL",
        [course_id],
        &live_assignments,
    )?;
    // A new assignment has no state yet: the course's `lms.assign` key no
    // longer covers it, so it reads as missing and is read next.
    if new_assignment {
        sync_state::invalidate(tx, &Key::new(Dataset::LmsAssign, course_id.to_string()))?;
    }
    commit_key(tx, Dataset::LmsCourse, course_id.to_string(), account, now)?;
    Ok(changed)
}

/// Whether `lms.assign/{lms_course}` is stored for `account`, so it owns
/// the course's submission states. Cached per apply.
fn assign_owns(
    tx: &Transaction<'_>,
    lms_course: u64,
    account: &str,
    cache: &mut HashMap<u64, bool>,
) -> Result<bool, IngestError> {
    if let Some(known) = cache.get(&lms_course) {
        return Ok(*known);
    }
    let known = sync_state::get(tx, &Key::new(Dataset::LmsAssign, lms_course.to_string()))?
        .is_some_and(|state| state.fetched_at.is_some() && state.account == account);
    cache.insert(lms_course, known);
    Ok(known)
}

/// Commits `lms.deadlines/all`: due dates of open and upcoming work, and
/// the open-work hint. A course whose `lms.assign` key is stored owns its
/// submission states; for it the hint only moves open work that left the
/// list before its deadline to submitted. Open work of an enrolled course
/// whose outline has not been read yet gets its row here.
pub fn apply_lms_deadlines(
    tx: &Transaction<'_>,
    deadlines: &LmsDeadlines,
    account: &str,
    now: DateTime<Utc>,
) -> Result<usize, IngestError> {
    let mut changed = 0;
    // Due dates: the calendar's due event, else the open-work event's time.
    let dated: HashMap<u64, i64> = deadlines
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
    let mut listed: HashMap<u64, &Value> = HashMap::new();
    for event in &deadlines.action_events {
        let module = event
            .get("modulename")
            .and_then(Value::as_str)
            .unwrap_or("");
        let url = event.get("url").and_then(Value::as_str).unwrap_or("");
        if !matches!(module, "assign" | "quiz") || (!url.is_empty() && !lms_url(url)) {
            continue;
        }
        if let Some(cmid) = id_of(event.get("instance")) {
            listed.insert(cmid, event);
        }
    }
    let due_of = |cmid: u64| {
        dated.get(&cmid).copied().or_else(|| {
            listed
                .get(&cmid)
                .and_then(|event| event.get("timesort"))
                .and_then(Value::as_i64)
        })
    };
    let mut assign_owned: HashMap<u64, bool> = HashMap::new();
    // Rows this read says something about, and every row still open.
    let mut cmids: HashSet<u64> = listed.keys().chain(dated.keys()).copied().collect();
    let mut statement = tx
        .prepare("SELECT lms_cmid FROM assignments WHERE submission='open' AND missing_since IS NULL AND lms_cmid IS NOT NULL")
        .map_err(IngestError::from)?;
    let open = statement
        .query_map([], |row| row.get::<_, i64>(0))
        .map_err(IngestError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(IngestError::from)?;
    drop(statement);
    cmids.extend(open.into_iter().map(|cmid| cmid as u64));

    let mut cmids: Vec<u64> = cmids.into_iter().collect();
    cmids.sort_unstable();
    // Courses that got an assignment row here, before its state was read.
    let mut stateless = BTreeSet::new();
    for cmid in cmids {
        let event = listed.get(&cmid).copied();
        let mut row = match Assignment::load(tx, cmid)? {
            Some(row) => row,
            None => {
                let Some(event) = event else {
                    continue;
                };
                let Some(lms_course) =
                    id_of(event.get("course").and_then(|course| course.get("id")))
                else {
                    continue;
                };
                let Some(stored) = enrolled_course(tx, lms_course)? else {
                    continue;
                };
                let title = event
                    .get("activityname")
                    .or_else(|| event.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let url = event.get("url").and_then(Value::as_str).unwrap_or("");
                stateless.insert(lms_course);
                Assignment::new(
                    cmid,
                    lms_course,
                    linked_course(tx, lms_course, &stored)?,
                    title,
                    url,
                )
            }
        };
        if let Some(due) = due_of(cmid) {
            row.set_due(due);
        }
        let owned = assign_owns(tx, row.lms_course, account, &mut assign_owned)?;
        if event.is_some() {
            if !owned {
                row.submission = "open".into();
            }
        } else if row.submission == "open" {
            if row.due_timestamp().is_some_and(|due| due > now.timestamp()) {
                row.submission = "submitted".into();
            } else if !owned {
                // Open work that left the list after its deadline: unknown.
                row.submission = "unknown".into();
            }
        }
        changed += usize::from(row.write(tx)?);
    }
    for lms_course in stateless {
        sync_state::invalidate(tx, &Key::new(Dataset::LmsAssign, lms_course.to_string()))?;
    }
    commit_key(tx, Dataset::LmsDeadlines, "all", account, now)?;
    Ok(changed)
}

/// Commits `lms.assign/{id}`: one's own state of each assignment and quiz
/// the course's outline has a row for. Due dates, titles and links stay.
pub fn apply_lms_assign(
    tx: &Transaction<'_>,
    states: &LmsAssignStates,
    account: &str,
    now: DateTime<Utc>,
) -> Result<usize, IngestError> {
    let mut changed = 0;
    let mut cmids: Vec<&u64> = states.states.keys().collect();
    cmids.sort_unstable();
    for cmid in cmids {
        let Some(mut row) = Assignment::load(tx, *cmid)? else {
            continue;
        };
        if row.lms_course != states.course_id {
            continue;
        }
        let state = &states.states[cmid];
        row.submission = state.submission.clone();
        row.grade = state.grade.clone();
        row.accepts_submission = state.accepts_submission;
        changed += usize::from(row.write(tx)?);
    }
    commit_key(
        tx,
        Dataset::LmsAssign,
        states.course_id.to_string(),
        account,
        now,
    )?;
    Ok(changed)
}

/// Commits `lms.history/{YYYY-MM}`: the month's due dates for assignments
/// that have no due date or one that has passed. A future due date belongs
/// to `lms.deadlines`.
pub fn apply_lms_history(
    tx: &Transaction<'_>,
    month: &LmsHistoryMonth,
    account: &str,
    now: DateTime<Utc>,
) -> Result<usize, IngestError> {
    let mut changed = 0;
    for event in &month.events {
        if !matches!(
            event.get("eventtype").and_then(Value::as_str),
            Some("due" | "close")
        ) {
            continue;
        }
        let (Some(cmid), Some(due)) = (
            id_of(event.get("instance")),
            event.get("timesort").and_then(Value::as_i64),
        ) else {
            continue;
        };
        let Some(mut row) = Assignment::load(tx, cmid)? else {
            continue;
        };
        if row
            .due_timestamp()
            .is_some_and(|stored| stored > now.timestamp())
        {
            continue;
        }
        row.set_due(due);
        changed += usize::from(row.write(tx)?);
    }
    commit_key(tx, Dataset::LmsHistory, month.month.clone(), account, now)?;
    Ok(changed)
}

/// What the store holds of a course's modules, for [`StoredModule`].
pub fn stored_modules(
    connection: &Connection,
    lms_course: u64,
) -> Result<HashMap<u64, StoredModule>, IngestError> {
    let mut statement = connection
        .prepare("SELECT value FROM lms_items WHERE lms_course_id=?1 AND missing_since IS NULL")
        .map_err(IngestError::from)?;
    let rows = statement
        .query_map([lms_course], |row| row.get::<_, String>(0))
        .map_err(IngestError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(IngestError::from)?;
    Ok(rows
        .iter()
        .filter_map(|raw| serde_json::from_str::<Value>(raw).ok())
        .filter_map(|item| {
            Some((
                id_of(item.get("cmid"))?,
                StoredModule {
                    modified: item.get("modified").and_then(Value::as_i64),
                    has_body: item.get("bodyHtml").is_some_and(|body| !body.is_null()),
                },
            ))
        })
        .collect())
}

/// An enrolled LMS course as stored, for planning its keys.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredLmsCourse {
    pub id: u64,
    pub title: String,
    /// Academic year from the shortname.
    pub year: Option<i32>,
    /// The quarter the course starts in.
    pub quarter: Option<u8>,
    /// Terms (`2026-3Q`) whose timetable grid lists the course.
    pub shown_in: Vec<String>,
}

/// Every enrolled (not missing) LMS course.
pub fn stored_courses(connection: &Connection) -> Result<Vec<StoredLmsCourse>, IngestError> {
    let mut statement = connection
        .prepare("SELECT value FROM lms_courses WHERE missing_since IS NULL ORDER BY id")
        .map_err(IngestError::from)?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(IngestError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(IngestError::from)?;
    Ok(rows
        .iter()
        .filter_map(|raw| serde_json::from_str::<Value>(raw).ok())
        .filter_map(|course| {
            let title = ["ja", "en"]
                .iter()
                .find_map(|lang| {
                    course
                        .pointer(&format!("/title/{lang}"))
                        .and_then(Value::as_str)
                })
                .or_else(|| course.get("shortname").and_then(Value::as_str))
                .unwrap_or("")
                .to_string();
            Some(StoredLmsCourse {
                id: id_of(course.get("lmsId"))?,
                title,
                year: course
                    .get("year")
                    .and_then(Value::as_i64)
                    .map(|year| year as i32),
                quarter: course
                    .pointer("/quarters/0")
                    .and_then(Value::as_str)
                    .and_then(|quarter| quarter.strip_suffix('Q'))
                    .and_then(|quarter| quarter.parse().ok()),
                shown_in: course
                    .get("timetables")
                    .and_then(Value::as_object)
                    .map(|terms| {
                        terms
                            .iter()
                            .filter(|(_, meetings)| meetings.is_array())
                            .map(|(term, _)| term.clone())
                            .collect()
                    })
                    .unwrap_or_default(),
            })
        })
        .collect())
}

fn japan_today(now: DateTime<Utc>) -> NaiveDate {
    FixedOffset::east_opt(9 * 3600)
        .map(|offset| now.with_timezone(&offset).date_naive())
        .unwrap_or_else(|| now.date_naive())
}

fn parse_term(term: &str) -> Option<(i32, u8)> {
    let (year, quarter) = term.split_once('-')?;
    let quarter = quarter
        .strip_suffix('Q')?
        .parse::<u8>()
        .ok()
        .filter(|quarter| (1..=4).contains(quarter))?;
    Some((year.parse().ok()?, quarter))
}

/// The LMS quarter now, as `(academic year, quarter)`: the term of the
/// dashboard grid when this pull read it, else the month in Japan.
pub fn current_lms_term(grid_term: Option<&str>, now: DateTime<Utc>) -> (i32, u8) {
    if let Some(term) = grid_term.and_then(parse_term) {
        return term;
    }
    let today = japan_today(now);
    let year = today.year();
    match today.month() {
        4 | 5 => (year, 1),
        6..=9 => (year, 2),
        10 | 11 => (year, 3),
        12 => (year, 4),
        _ => (year - 1, 4),
    }
}

/// The term a course's keys age by. A course the current grid lists, or
/// one starting in the current quarter, is current; one of an earlier
/// academic year is frozen; anything else (an earlier or later quarter, an
/// unknown year) is past.
pub fn lms_course_term(course: &StoredLmsCourse, current: (i32, u8)) -> Term {
    let label = format!("{}-{}Q", current.0, current.1);
    if course.shown_in.contains(&label) {
        return Term::Current;
    }
    match course.year {
        Some(year) if year < current.0 => Term::Frozen,
        Some(year) if year == current.0 && course.quarter == Some(current.1) => Term::Current,
        _ => Term::Past,
    }
}

/// The keys a pull reads after `lms.enrolment` and `lms.deadlines`, in
/// order: `lms.course` keys, then `lms.assign` keys, then `lms.history`
/// months. A manual pull reads every current-term course; otherwise a key
/// is read when it is due, missing before stale. A manual pull ignores
/// backoff.
pub fn plan_lms_keys<'c>(
    courses: &'c [StoredLmsCourse],
    states: &HashMap<Key, KeyState>,
    account: &str,
    manual: bool,
    grid_term: Option<&str>,
    now: DateTime<Utc>,
) -> Vec<(Key, Option<&'c StoredLmsCourse>)> {
    let current = current_lms_term(grid_term, now);
    let due = |key: &Key, term: Term| {
        sync_state::due(
            states.get(key),
            sync_state::ttl(key.dataset, term),
            account,
            now,
        )
    };
    let rank = |due: Due| match due {
        Due::Missing => Some(1u8),
        Due::Stale => Some(2),
        Due::BackingOff(_) if manual => Some(2),
        _ => None,
    };
    let mut planned = Vec::new();
    for dataset in [Dataset::LmsCourse, Dataset::LmsAssign] {
        let mut keys: Vec<(u8, &StoredLmsCourse, Key)> = courses
            .iter()
            .filter_map(|course| {
                let term = lms_course_term(course, current);
                let key = Key::new(dataset, course.id.to_string());
                let order = if manual && term == Term::Current {
                    0
                } else {
                    rank(due(&key, term))?
                };
                Some((order, course, key))
            })
            .collect();
        keys.sort_by_key(|(order, course, _)| (*order, course.id));
        planned.extend(keys.into_iter().map(|(_, course, key)| (key, Some(course))));
    }
    let today = japan_today(now);
    let mut years: Vec<i32> = courses.iter().filter_map(|course| course.year).collect();
    years.sort_unstable_by(|a, b| b.cmp(a));
    let mut months: Vec<(u8, Key)> = crate::net::moodle::months_to_read(&years, today)
        .into_iter()
        .filter_map(|(year, month)| {
            let key = Key::new(Dataset::LmsHistory, format!("{year:04}-{month:02}"));
            let term = if (year, month) < (today.year(), today.month()) {
                Term::Frozen
            } else {
                Term::Current
            };
            Some((rank(due(&key, term))?, key))
        })
        .collect();
    months.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.key.cmp(&b.1.key)));
    planned.extend(months.into_iter().map(|(_, key)| (key, None)));
    planned
}

#[cfg(test)]
#[path = "tests/lms_ingest.rs"]
mod tests;
