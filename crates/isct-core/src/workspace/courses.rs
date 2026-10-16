use super::*;

pub(super) fn validate_term(term: &str) -> Result<(), DataError> {
    let bytes = term.as_bytes();
    let valid = bytes.len() == 7
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && matches!(&bytes[5..], b"1Q" | b"2Q" | b"3Q" | b"4Q");
    if valid {
        Ok(())
    } else {
        Err("term must use YYYY-1Q..4Q".into())
    }
}

pub(super) fn edit_terms(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    term_id: String,
    add: bool,
) -> Result<(), DataError> {
    validate_term(&term_id)?;
    let mut terms = current
        .get("terms")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    terms.retain(|term| term.as_str() != Some(&term_id));
    if add {
        terms.push(Value::String(term_id.clone()));
        terms.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
        write.save_setting("selectedTerm", &Value::String(term_id))?;
    } else if current.get("selectedTerm").and_then(Value::as_str) == Some(&term_id) {
        return Err("the selected term cannot be removed".into());
    }
    write.save_setting("terms", &Value::Array(terms))
}

pub(super) fn save_course(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    draft: Value,
    term_id: String,
    existing_id: Option<String>,
) -> Result<(), DataError> {
    validate_term(&term_id)?;
    let draft = require_object(&draft, "course draft")?;
    let existing = existing_id
        .as_deref()
        .and_then(|id| visible_row(current, "courses", id));
    let existing_rev = existing
        .and_then(|row| row.get("rev"))
        .and_then(Value::as_u64);
    if let Some(existing) = existing
        .filter(|row| row.pointer("/source/provider").and_then(Value::as_str) != Some("manual"))
    {
        put_course_override(
            write,
            existing,
            ["title", "instructor", "color"]
                .map(|key| (key, draft.get(key).cloned().unwrap_or(Value::Null))),
        )?;
        return set_course_on_timetable(
            write,
            current,
            string(existing, "id")?,
            draft
                .get("onTimetable")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        );
    }
    let mut course = existing.cloned().unwrap_or_else(|| {
        json!({
            "id": Uuid::now_v7().to_string(), "kind":"course", "rev":0,
            "updatedAt":"", "updatedBy":"", "termId":term_id,
            "code":"", "title":"", "instructor":"", "credits":0,
            "creditSplit":{"lecture":0,"exercise":0,"lab":0},
            "instructionLanguage":"other", "color":"#526e91", "meetings":[],
            "source":{"provider":"manual"}
        })
    });
    let object = object_mut(&mut course, "course")?;
    for key in [
        "code",
        "title",
        "instructor",
        "credits",
        "instructionLanguage",
        "color",
        "meetings",
        "url",
    ] {
        if let Some(value) = draft.get(key) {
            object.insert(key.into(), value.clone());
        }
    }
    object.insert("termId".into(), Value::String(term_id.clone()));
    let credits = object.get("credits").and_then(Value::as_f64).unwrap_or(0.0);
    object.insert(
        "creditSplit".into(),
        json!({"lecture":credits,"exercise":0,"lab":0}),
    );
    let course_id = string(&course, "id")?.to_string();
    write.put("manual_courses", course, existing_rev)?;
    let on_timetable = draft
        .get("onTimetable")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if visible_row(current, "courses", &course_id).is_some() {
        set_course_on_timetable(write, current, &course_id, on_timetable)?;
    } else if on_timetable {
        write.put(
            "manual_memberships",
            json!({"id":Uuid::now_v7().to_string(),"kind":"membership","rev":0,"updatedAt":"","updatedBy":"","termId":term_id,"courseId":course_id,"status":"declared","evidence":{"provider":"manual"}}),
            None,
        )?;
    }
    Ok(())
}

/// Deletes the selected courses, whatever added them, with every quarter
/// they are placed in.
pub(super) fn delete_course_batch(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    course_ids: Vec<String>,
) -> Result<(), DataError> {
    let ids: HashSet<_> = course_ids.into_iter().collect();
    for course in rows(current, "courses")?.filter(|row| {
        is_live(row)
            && row
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| ids.contains(id))
    }) {
        let id = string(course, "id")?;
        // Courses the school cache lists are not user rows; delete reports
        // false for them and their placements still go.
        write.delete("manual_courses", id, None)?;
        remove_placements(write, current, id, None)?;
    }
    Ok(())
}

/// Tombstones the live memberships and plan items that put a course on the
/// timetable: in `term`, or in every quarter it runs in.
pub(super) fn remove_placements(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    course_id: &str,
    term: Option<&str>,
) -> Result<(), DataError> {
    if rows(current, "memberships")?.any(|row| {
        is_live(row)
            && row.get("courseId").and_then(Value::as_str) == Some(course_id)
            && term.is_none_or(|term| row.get("termId").and_then(Value::as_str) == Some(term))
            && row.pointer("/evidence/provider").and_then(Value::as_str) == Some("kyomu")
    }) {
        return Err(DataError::InvalidInput(
            "This course is on your official timetable. Change enrollment in 教務Web and refresh the timetable.".into(),
        ));
    }
    for (field, table) in [
        ("memberships", "manual_memberships"),
        ("planItems", "plan_items"),
    ] {
        for row in rows(current, field)?.filter(|row| {
            is_live(row)
                && row.get("courseId").and_then(Value::as_str) == Some(course_id)
                && term.is_none_or(|term| row.get("termId").and_then(Value::as_str) == Some(term))
        }) {
            write.delete(table, string(row, "id")?, None)?;
        }
    }
    Ok(())
}

pub(super) fn is_live(row: &Value) -> bool {
    row.get("deletedAt").is_none_or(Value::is_null)
}

pub(super) fn edit_course_batch(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    course_ids: Vec<String>,
    changes: CourseBatchChanges,
) -> Result<(), DataError> {
    if changes
        .credits
        .is_some_and(|credits| !credits.is_finite() || !(0.0..=20.0).contains(&credits))
    {
        return Err("credits must be between 0 and 20".into());
    }
    let language = changes
        .language
        .as_deref()
        .map(str::trim)
        .filter(|language| !language.is_empty());
    if language.is_some_and(|language| !matches!(language, "ja" | "en" | "ja-en" | "other")) {
        return Err("instruction language is invalid".into());
    }
    let instructor = changes
        .instructor
        .as_deref()
        .map(str::trim)
        .filter(|instructor| !instructor.is_empty());
    let ids: HashSet<_> = course_ids.into_iter().collect();
    for course in rows(current, "courses")?.filter(|row| {
        row.get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| ids.contains(id) && row.get("deletedAt").is_none_or(Value::is_null))
    }) {
        let id = string(course, "id")?;
        let manual = course.pointer("/source/provider").and_then(Value::as_str) == Some("manual");
        // Catalog and LMS courses keep their source instructor; the person's
        // name for it is an overlay like the course editor writes.
        if let Some(instructor) = instructor.filter(|_| !manual) {
            put_course_override(write, course, [("instructor", Value::from(instructor))])?;
        }
        if language.is_none() && changes.credits.is_none() && (instructor.is_none() || !manual) {
            continue;
        }
        if !write.has_user_row("manual_courses", id)? {
            return Err(
                "credits and language of a school timetable course cannot be edited".into(),
            );
        }
        let mut updated = course.clone();
        let object = object_mut(&mut updated, "course")?;
        // The read model embeds the overlay; the stored row never contains it.
        object.remove("overrides");
        if let Some(language) = language {
            object.insert("instructionLanguage".into(), Value::from(language));
        }
        if let Some(instructor) = instructor.filter(|_| manual) {
            object.insert("instructor".into(), Value::from(instructor));
        }
        if let Some(credits) = changes.credits {
            object.insert("credits".into(), Value::from(credits));
        }
        write.put("manual_courses", updated, revision(course))?;
    }
    Ok(())
}

/// One overlay row holds every personal change to a non-manual course. Each
/// edit replaces only its own fields and keeps the others.
pub(super) fn put_course_override<const N: usize>(
    write: &WorkspaceWrite<'_, '_>,
    course: &Value,
    changes: [(&str, Value); N],
) -> Result<(), DataError> {
    let id = string(course, "id")?;
    let mut overlay = course
        .get("overrides")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (key, value) in changes {
        overlay.insert(key.into(), value);
    }
    overlay.insert(
        "sourceKey".into(),
        Value::from(
            course
                .get("sourceKey")
                .and_then(Value::as_str)
                .unwrap_or(id),
        ),
    );
    overlay.insert("courseId".into(), Value::from(id));
    write
        .put("course_overrides", Value::Object(overlay), None)
        .map(|_| ())
}

pub(super) fn set_course_on_timetable(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    course_id: &str,
    on: bool,
) -> Result<(), DataError> {
    let course = visible_row(current, "courses", course_id)
        .ok_or_else(|| "course was not found".to_string())?;
    if !on {
        // Off takes the course out of every quarter it runs in (3Q–4Q
        // alike), whoever placed it there.
        return remove_placements(write, current, course_id, None);
    }
    let placed = rows(current, "memberships")?
        .chain(rows(current, "planItems")?)
        .any(|row| is_live(row) && row.get("courseId").and_then(Value::as_str) == Some(course_id));
    if !placed {
        let term = string(course, "termId")?;
        write.put("manual_memberships", json!({"id":Uuid::now_v7().to_string(),"kind":"membership","rev":0,"updatedAt":"","updatedBy":"","termId":term,"courseId":course_id,"status":"declared","evidence":{"provider":"manual"}}), None)?;
    }
    Ok(())
}
