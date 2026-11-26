use super::{
    feeds::object_mut, AcademicCalendar, ClassSchedules, Departments, ImportantDates,
    PublishedExamFeed, SubscriptionError,
};
use crate::data::{offering_course_id, versioned::content_hash};
use rusqlite::{OptionalExtension, Transaction};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

fn apply_row(
    tx: &Transaction<'_>,
    table: &str,
    id: &str,
    value: &Value,
    source_hash: &str,
) -> Result<bool, SubscriptionError> {
    let hash = content_hash(value)?;
    let old: Option<(String, u64, bool)> = tx
        .query_row(
            &format!("SELECT hash,rev,missing_since IS NOT NULL OR deleted_at IS NOT NULL FROM {table} WHERE id=?1"),
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(SubscriptionError::from)?;
    let timestamp = crate::clock::now().to_rfc3339();
    if let Some((stored, _, gone)) = &old {
        if stored == &hash {
            return crate::data::versioned::keep_unchanged(tx, table, id, *gone, &timestamp)
                .map_err(SubscriptionError::from);
        }
    }
    let rev = old.map_or(0, |x| x.1) + 1;
    let mut stored = value.clone();
    if let Some(object) = stored.as_object_mut() {
        object.insert("rev".into(), rev.into());
        object.insert("updatedAt".into(), timestamp.clone().into());
        object.insert("updatedBy".into(), "subscription".into());
        object.insert(
            "source".into(),
            json!({"provider":"subscription","hash":source_hash}),
        );
        object
            .entry("sourceKey")
            .or_insert_with(|| format!("reference:{table}:{id}").into());
        object.insert("lastSeenAt".into(), timestamp.clone().into());
        object.remove("missingSince");
        object.remove("deletedAt");
    }
    let json = serde_json::to_string(&stored).map_err(SubscriptionError::from)?;
    tx.execute(
        &format!(
            "INSERT INTO {table}
             (id,value,hash,updated_at,updated_by,rev,source,deleted_at,last_seen_at,missing_since)
             VALUES (?1,?2,?3,?4,'subscription',?5,?6,NULL,?4,NULL)
             ON CONFLICT(id) DO UPDATE SET value=excluded.value,hash=excluded.hash,
               updated_at=excluded.updated_at,updated_by=excluded.updated_by,rev=excluded.rev,
               source=excluded.source,deleted_at=NULL,last_seen_at=excluded.last_seen_at,
               missing_since=NULL"
        ),
        rusqlite::params![id, json, hash, timestamp, rev as i64, source_hash],
    )
    .map_err(SubscriptionError::from)?;
    Ok(true)
}

pub fn apply_reference(
    tx: &Transaction<'_>,
    selected_calendar: Option<&AcademicCalendar>,
    selected_departments: Option<&Departments>,
) -> Result<usize, SubscriptionError> {
    let empty_calendar = AcademicCalendar {
        schema_version: 1,
        academic_year: 0,
        source: vec![],
        quarters: BTreeMap::new(),
        days: vec![],
        teaching_ranges: BTreeMap::new(),
        exam_ranges: BTreeMap::new(),
    };
    let empty_departments = Departments {
        schema_version: 1,
        admission_year: 0,
        source: vec![],
        departments: vec![],
    };
    let calendar = selected_calendar.unwrap_or(&empty_calendar);
    let departments = selected_departments.unwrap_or(&empty_departments);
    let calendar_value = serde_json::to_value(calendar).map_err(SubscriptionError::from)?;
    let departments_value = serde_json::to_value(departments).map_err(SubscriptionError::from)?;
    let calendar_hash = content_hash(&calendar_value)?;
    let guide_hash = content_hash(&departments_value)?;
    let mut changed = 0;
    let mut calendar_ids = HashSet::new();
    for (quarter, range) in &calendar.quarters {
        for (kind, date) in [("quarter-start", &range.start), ("quarter-end", &range.end)] {
            let id = format!("calendar:{}:{kind}:{quarter}", calendar.academic_year);
            calendar_ids.insert(id.clone());
            let label = json!({"ja":format!("{quarter} {}",if kind=="quarter-start"{"開始"}else{"終了"}),"en":format!("{quarter} {}",if kind=="quarter-start"{"starts"}else{"ends"})});
            let mut value = json!({"id":id,"academicYear":calendar.academic_year,"date":date,"type":kind,"quarter":quarter,"label":label,"provenance":{"source":calendar.source}});
            if kind == "quarter-start" {
                if let Some(ranges) = calendar.teaching_ranges.get(quarter) {
                    value["teachingRanges"] = json!(ranges);
                }
                if let Some(ranges) = calendar.exam_ranges.get(quarter) {
                    value["examRanges"] = json!(ranges);
                }
            }
            changed += usize::from(apply_row(
                tx,
                "academic_calendar",
                &id,
                &value,
                &calendar_hash,
            )?);
        }
    }
    for day in &calendar.days {
        let id = format!("calendar:{}:day:{}", calendar.academic_year, day.date);
        calendar_ids.insert(id.clone());
        let mut value = serde_json::to_value(day).map_err(SubscriptionError::from)?;
        let object = object_mut(&mut value, "calendar day")?;
        object.insert("id".into(), id.clone().into());
        object.insert("academicYear".into(), calendar.academic_year.into());
        object.insert("provenance".into(), json!({"source":calendar.source}));
        changed += usize::from(apply_row(
            tx,
            "academic_calendar",
            &id,
            &value,
            &calendar_hash,
        )?);
    }
    let mut department_ids = HashSet::new();
    let mut recommended_ids = HashSet::new();
    for department in &departments.departments {
        let id = format!(
            "cohort:{}:department:{}",
            departments.admission_year, department.id
        );
        department_ids.insert(id.clone());
        let mut value = serde_json::to_value(department).map_err(SubscriptionError::from)?;
        let object = object_mut(&mut value, "department")?;
        object.insert("id".into(), id.clone().into());
        object.insert("admissionYear".into(), departments.admission_year.into());
        object.insert("cohortYear".into(), departments.admission_year.into());
        object.insert("provenance".into(), json!({"source":departments.source}));
        changed += usize::from(apply_row(tx, "departments", &id, &value, &guide_hash)?);
        for course in &department.recommended {
            let id = format!(
                "cohort:{}:recommended:{}:{}:{}",
                departments.admission_year, department.id, course.code, course.group
            );
            recommended_ids.insert(id.clone());
            let mut value = serde_json::to_value(course).map_err(SubscriptionError::from)?;
            let object = object_mut(&mut value, "recommended course")?;
            object.insert("id".into(), id.clone().into());
            object.insert("admissionYear".into(), departments.admission_year.into());
            object.insert("cohortYear".into(), departments.admission_year.into());
            object.insert("departmentId".into(), department.id.clone().into());
            object.insert(
                "departmentRowId".into(),
                format!(
                    "cohort:{}:department:{}",
                    departments.admission_year, department.id
                )
                .into(),
            );
            object.insert(
                "provenance".into(),
                json!({"source":departments.source,"guidePdf":department.guide_pdf}),
            );
            changed += usize::from(apply_row(
                tx,
                "recommended_courses",
                &id,
                &value,
                &guide_hash,
            )?);
        }
    }
    for (table, prefix, live) in [
        (
            "academic_calendar",
            format!("calendar:{}:%", calendar.academic_year),
            calendar_ids,
        ),
        (
            "departments",
            format!("cohort:{}:department:%", departments.admission_year),
            department_ids,
        ),
        (
            "recommended_courses",
            format!("cohort:{}:recommended:%", departments.admission_year),
            recommended_ids,
        ),
    ] {
        if (table == "academic_calendar" && selected_calendar.is_none())
            || (table != "academic_calendar" && selected_departments.is_none())
        {
            continue;
        }
        changed += mark_missing(tx, table, &prefix, &live)?;
    }
    Ok(changed)
}

fn mark_missing(
    tx: &Transaction<'_>,
    table: &str,
    prefix: &str,
    live: &HashSet<String>,
) -> Result<usize, SubscriptionError> {
    let mut statement = tx
        .prepare(&format!(
            "SELECT id FROM {table} WHERE id LIKE ?1 AND missing_since IS NULL"
        ))
        .map_err(SubscriptionError::from)?;
    let old = statement
        .query_map([prefix], |row| row.get::<_, String>(0))
        .map_err(SubscriptionError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(SubscriptionError::from)?;
    drop(statement);
    crate::data::versioned::mark_missing_rows(tx, table, old, live, || crate::clock::now().to_rfc3339())
        .map_err(SubscriptionError::from)
}

pub fn apply_important_dates(
    tx: &Transaction<'_>,
    feed: &ImportantDates,
) -> Result<usize, SubscriptionError> {
    let feed_value = serde_json::to_value(feed).map_err(SubscriptionError::from)?;
    let source_hash = content_hash(&feed_value)?;
    let mut changed = 0;
    let mut live = HashSet::new();
    for entry in &feed.entries {
        let id = format!("important-date:{}:{}", feed.academic_year, entry.id);
        live.insert(id.clone());
        let mut value = serde_json::to_value(entry).map_err(SubscriptionError::from)?;
        let object = object_mut(&mut value, "important date")?;
        object.insert("id".into(), id.clone().into());
        object.insert("academicYear".into(), feed.academic_year.into());
        object.insert(
            "sourceKey".into(),
            format!(
                "reference:important-dates:{}:{}",
                feed.academic_year, entry.id
            )
            .into(),
        );
        object.insert("provenance".into(), json!({"source":feed.source}));
        changed += usize::from(apply_row(tx, "important_dates", &id, &value, &source_hash)?);
    }
    changed += mark_missing(
        tx,
        "important_dates",
        &format!("important-date:{}:%", feed.academic_year),
        &live,
    )?;
    Ok(changed)
}

pub fn apply_class_schedules(
    tx: &Transaction<'_>,
    feed: &ClassSchedules,
) -> Result<usize, SubscriptionError> {
    let feed_value = serde_json::to_value(feed).map_err(SubscriptionError::from)?;
    let source_hash = content_hash(&feed_value)?;
    let mut changed = 0;
    let mut live = HashSet::new();
    for entry in &feed.entries {
        let id = format!("class-schedule:{}:{}", feed.academic_year, entry.id);
        live.insert(id.clone());
        let mut value = serde_json::to_value(entry).map_err(SubscriptionError::from)?;
        let object = object_mut(&mut value, "class schedule")?;
        object.insert("id".into(), id.clone().into());
        object.insert("academicYear".into(), feed.academic_year.into());
        object.insert(
            "sourceKey".into(),
            format!(
                "reference:class-schedules:{}:{}",
                feed.academic_year, entry.id
            )
            .into(),
        );
        object.insert("provenance".into(), json!({"source":feed.source}));
        changed += usize::from(apply_row(tx, "class_schedules", &id, &value, &source_hash)?);
    }
    changed += mark_missing(
        tx,
        "class_schedules",
        &format!("class-schedule:{}:%", feed.academic_year),
        &live,
    )?;
    Ok(changed)
}

pub fn apply_exam_feed(
    tx: &Transaction<'_>,
    published: &PublishedExamFeed,
) -> Result<usize, SubscriptionError> {
    let feed_value = serde_json::to_value(&published.feed).map_err(SubscriptionError::from)?;
    let source_hash = content_hash(&feed_value)?;
    let mut changed = 0;
    let mut live = HashSet::new();
    for entry in &published.feed.entries {
        let id = format!(
            "{}:{}:{}:{}:{}",
            published.kind.row_prefix(),
            published.feed.academic_year,
            published.level,
            published.quarter,
            entry.id
        );
        live.insert(id.clone());
        let mut value = serde_json::to_value(entry).map_err(SubscriptionError::from)?;
        let object = object_mut(&mut value, "exam")?;
        object.insert("id".into(), id.clone().into());
        object.insert("academicYear".into(), published.feed.academic_year.into());
        object.insert("level".into(), published.level.clone().into());
        if let Some(offering_id) = entry.syllabus_course_id.as_deref() {
            let course_id = offering_course_id(tx, published.feed.academic_year, offering_id)?;
            object.insert("courseId".into(), course_id.into());
        }
        object.insert(
            "sourceKey".into(),
            format!(
                "{}:{}:{}:{}:{}",
                published.kind.source_key_prefix(),
                published.feed.academic_year,
                published.level,
                published.quarter,
                entry.id
            )
            .into(),
        );
        object.insert(
            "provenance".into(),
            json!({"source":published.feed.source,"dataUrl":published.url}),
        );
        changed += usize::from(apply_row(tx, "exams", &id, &value, &source_hash)?);
    }
    let prefix = format!(
        "{}:{}:{}:{}:%",
        published.kind.row_prefix(),
        published.feed.academic_year,
        published.level,
        published.quarter
    );
    changed += mark_missing(tx, "exams", &prefix, &live)?;
    Ok(changed)
}
