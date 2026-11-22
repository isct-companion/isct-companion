use super::*;
use crate::data::lms_visibility::{current_lms_placements, restores_lms_timetable};

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
        if let Some(on) = draft.get("onTimetable").and_then(Value::as_bool) {
            set_course_on_timetable(write, current, string(existing, "id")?, on)?;
        }
        return Ok(());
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

/// Delete manually authored courses, including the personal placements that refer to them.
pub(super) fn delete_course_batch(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    course_ids: Vec<String>,
) -> Result<(), DataError> {
    let mut owned: Vec<_> = course_ids
        .into_iter()
        .filter(|id| {
            visible_row(current, "courses", id)
                .is_some_and(|course| course["source"]["provider"] == "manual")
        })
        .collect();
    owned.sort_unstable();
    owned.dedup();
    remove_course_plans(write, current, &owned)?;
    for id in owned {
        write.delete("manual_courses", &id, None)?;
        remove_manual_memberships(write, current, &id)?;
    }
    Ok(())
}

pub(super) fn remove_course_plans(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    course_ids: &[String],
) -> Result<(), DataError> {
    let ids: HashSet<_> = course_ids.iter().map(String::as_str).collect();
    remove_catalog_plans(
        write,
        rows(current, "planItems")?
            .filter(|row| row["courseId"].as_str().is_some_and(|id| ids.contains(id))),
    )
    .map(|_| ())
}

fn remove_manual_memberships(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    course_id: &str,
) -> Result<(), DataError> {
    for row in rows(current, "memberships")?.filter(|row| {
        is_live(row) && row["courseId"] == course_id && row["evidence"]["provider"] == "manual"
    }) {
        write.delete("manual_memberships", string(row, "id")?, revision(row))?;
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
    let saved = write.rows("manual_courses")?;
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
        let original = saved.iter().find(|row| {
            is_live(row) && row["id"] == id && !crate::data::catalog_drafts::is_description(row)
        });
        let editing_draft =
            rows(current, "planItems")?.any(|plan| is_live(plan) && plan["courseId"] == id);
        let original = original.or_else(|| {
            let offering = course["offeringId"].as_str()?;
            editing_draft
                .then(|| crate::data::catalog_drafts::description(&saved, offering))
                .flatten()
        });
        let original = original.ok_or_else(|| {
            DataError::InvalidInput(
                "credits and language of a school timetable course cannot be edited".into(),
            )
        })?;
        let mut updated = original.clone();
        let object = object_mut(&mut updated, "course")?;
        if let Some(language) = language {
            object.insert("instructionLanguage".into(), Value::from(language));
        }
        if let Some(instructor) = instructor.filter(|_| manual) {
            object.insert("instructor".into(), Value::from(instructor));
        }
        if let Some(credits) = changes.credits {
            object.insert("credits".into(), Value::from(credits));
        }
        write.put("manual_courses", updated, revision(original))?;
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
    if course["source"]["provider"] == "manual" {
        if !on {
            return remove_manual_memberships(write, current, course_id);
        }
        let placed = rows(current, "memberships")?.any(|row| {
            is_live(row) && row["courseId"] == course_id && row["evidence"]["provider"] == "manual"
        });
        if !placed {
            let term = string(course, "termId")?;
            write.put(
                "manual_memberships",
                json!({"kind":"membership","termId":term,"courseId":course_id,
                "status":"declared","evidence":{"provider":"manual"}}),
                None,
            )?;
        }
        return Ok(());
    }

    // The LMS visibility preference is per source course and quarter. The
    // timetable toggle applies to every known quarter of the selected course.
    let choices = write.rows("manual_memberships")?;
    let lms_placements = current_lms_placements(current);
    for row in rows(current, "memberships")?.filter(|row| {
        row["courseId"] == course_id
            && is_live(row) != on
            && row["id"]
                .as_str()
                .is_some_and(|id| lms_placements.contains(id))
    }) {
        let id = string(row, "id")?;
        if on {
            write.put("manual_memberships", row.clone(), revision(row))?;
        } else {
            if let Some(prior_hide) = choices.iter().find(|choice| choice["id"] == id) {
                for legacy in choices
                    .iter()
                    .filter(|choice| restores_lms_timetable(choice, prior_hide))
                {
                    write.delete(
                        "manual_memberships",
                        string(legacy, "id")?,
                        revision(legacy),
                    )?;
                }
            }
            // LMS supplies the live row. Save the preference under the same
            // identity so new school evidence cannot change the selection.
            let saved = write.put("manual_memberships", row.clone(), None)?;
            write.delete("manual_memberships", id, revision(&saved))?;
        }
    }
    Ok(())
}
