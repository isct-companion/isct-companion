use super::*;

/// Adds catalog offerings to the timetable of `term_id`, the quarter on
/// screen.  An offering running over several quarters (3Q–4Q) that is added
/// in one of them is planned into each of them; added outside its quarters
/// it stays in `term_id`.  A local draft (plan item) in `term_id` whose
/// periods clash leaves the timetable; school-registered courses stay.
pub(super) fn catalog_plan_terms(
    offering: &Value,
    term_id: &str,
) -> Result<Vec<String>, DataError> {
    validate_term(term_id)?;
    let (plan_year, quarter) = (&term_id[..4], &term_id[5..]);
    let quarters = offering
        .get("quarters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    if !quarters.contains(&quarter) {
        return Ok(vec![term_id.to_string()]);
    }
    let terms = quarters
        .iter()
        .map(|quarter| format!("{plan_year}-{quarter}"))
        .filter(|term| validate_term(term).is_ok())
        .collect::<Vec<_>>();
    if terms.is_empty() {
        return Err(DataError::InvalidInput(
            "catalog offering has no valid teaching quarter".into(),
        ));
    }
    Ok(terms)
}

pub(super) fn add_catalog_courses(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    term_id: &str,
    offerings: Vec<Value>,
) -> Result<CatalogAddReceipt, DataError> {
    validate_term(term_id)?;
    let teaching_periods = crate::schedule::teaching_periods(current)
        .map_err(|error| DataError::InvalidInput(error.to_string()))?;
    let offerings = offerings
        .iter()
        .map(|submitted| {
            let id = string(submitted, "id")?;
            write.catalog_row(id)?.ok_or_else(|| {
                DataError::InvalidInput(format!("catalog offering {id} is no longer available"))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    for offering in &offerings {
        for plan_term in catalog_plan_terms(offering, term_id)? {
            let eligibility =
                crate::study_profile::catalog_course_eligibility(current, &plan_term, offering);
            if let Some(message) =
                crate::study_profile::ineligibility_message(offering, eligibility)
            {
                return Err(DataError::InvalidInput(message));
            }
        }
    }
    let locale = workspace_content_locale(current);
    let plan_year = &term_id[..4];
    let mut offering_ids = offerings
        .iter()
        .map(|offering| string(offering, "id").map(str::to_string))
        .collect::<Result<Vec<_>, _>>()?;
    offering_ids.sort();
    offering_ids.dedup();
    let mut receipt = CatalogAddReceipt {
        term_id: term_id.to_string(),
        offering_ids,
        ..CatalogAddReceipt::default()
    };
    // Working copies, so later offerings see what earlier ones changed.
    let mut courses: Vec<Value> = rows(current, "courses")?.cloned().collect();
    let mut plans: Vec<Value> = rows(current, "planItems")?.cloned().collect();
    let members: Vec<&Value> = rows(current, "memberships")?
        .filter(|row| is_live(row))
        .collect();
    for offering in offerings {
        let offering_id = string(&offering, "id")?;
        let year = offering
            .get("year")
            .and_then(Value::as_i64)
            .or_else(|| plan_year.parse().ok())
            .ok_or_else(|| "catalog offering is missing academic year".to_string())?;
        let course_id = write.offering_course_id(
            i32::try_from(year).map_err(|_| "academic year is invalid")?,
            offering_id,
        )?;
        let placed_here: HashSet<&str> = members
            .iter()
            .copied()
            .chain(plans.iter().filter(|row| is_live(row)))
            .filter(|row| row.get("termId").and_then(Value::as_str) == Some(term_id))
            .filter_map(|row| row.get("courseId").and_then(Value::as_str))
            .collect();
        let on_timetable: Vec<Value> = courses
            .iter()
            .filter(|course| {
                is_live(course)
                    && course
                        .get("id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| placed_here.contains(id))
            })
            .cloned()
            .collect();
        for victim in crate::course_query::clashing_courses(&offering, &on_timetable) {
            let Some(plan) = plans.iter_mut().find(|row| {
                is_live(row)
                    && row.get("courseId") == victim.get("id")
                    && row.get("termId").and_then(Value::as_str) == Some(term_id)
            }) else {
                continue;
            };
            let id = string(plan, "id")?.to_string();
            write.delete("plan_items", &id, revision(plan))?;
            plan["deletedAt"] = Value::String("replaced".into());
            receipt.replaced.push(id);
        }
        let live_course = courses.iter().find(|course| {
            is_live(course)
                && (course.get("id").and_then(Value::as_str) == Some(&course_id)
                    || course.get("offeringId").and_then(Value::as_str) == Some(offering_id))
        });
        // A course already in the workspace stays as it is (and in its own
        // quarter); only its plan items change.
        let course_id = match live_course {
            Some(course) => string(course, "id")?.to_string(),
            None => {
                let tombstone = courses
                    .iter()
                    .find(|course| course.get("id").and_then(Value::as_str) == Some(&course_id));
                let course = write.put(
                    "manual_courses",
                    planned_catalog_course(
                        &offering,
                        &course_id,
                        term_id,
                        locale,
                        &teaching_periods,
                    )?,
                    tombstone.and_then(revision),
                )?;
                let id = string(&course, "id")?.to_string();
                courses.retain(|row| row.get("id").and_then(Value::as_str) != Some(&id));
                courses.push(course);
                id
            }
        };
        let plan_terms = catalog_plan_terms(&offering, term_id)?;
        for plan_term in plan_terms {
            let mine = |row: &&Value| {
                row.get("courseId").and_then(Value::as_str) == Some(&course_id)
                    && row.get("termId").and_then(Value::as_str) == Some(&plan_term)
            };
            if plans.iter().filter(mine).any(is_live) {
                continue;
            }
            // Bring back the latest removed draft before writing a new one.
            let removed = plans
                .iter()
                .filter(mine)
                .max_by(|a, b| {
                    a.get("deletedAt")
                        .and_then(Value::as_str)
                        .cmp(&b.get("deletedAt").and_then(Value::as_str))
                })
                .cloned();
            let plan = match removed {
                Some(row) => {
                    let rev = revision(&row);
                    write.put("plan_items", row, rev)?
                }
                None => write.put(
                    "plan_items",
                    json!({"kind":"plan-item","termId":plan_term,"courseId":course_id,"offeringId":offering_id,"state":"planned"}),
                    None,
                )?,
            };
            let id = string(&plan, "id")?.to_string();
            plans.retain(|row| row.get("id").and_then(Value::as_str) != Some(&id));
            plans.push(plan);
            receipt.placed.push(id);
        }
    }
    Ok(receipt)
}

/// Meetings from the LMS timetable grid, with the syllabus room for the same
/// weekday when known.
pub(crate) fn meetings_from_grid(
    grid: &Value,
    syllabus: &Value,
    teaching_periods: &[crate::schedule::TeachingPeriod],
) -> Result<Value, DataError> {
    let mut meetings = Vec::new();
    for row in grid.as_array().into_iter().flatten() {
        let day = row
            .get("day")
            .and_then(Value::as_u64)
            .ok_or_else(|| "LMS timetable slot is missing its day".to_string())?;
        let bounds = row.get("periods").and_then(Value::as_array);
        let start = bounds
            .and_then(|bounds| bounds.first())
            .and_then(Value::as_u64)
            .ok_or_else(|| "LMS timetable slot is missing its periods".to_string())?;
        let end = bounds
            .and_then(|bounds| bounds.get(1))
            .and_then(Value::as_u64)
            .unwrap_or(start);
        let period = |value: u64| {
            u8::try_from(value)
                .ok()
                .filter(|value| (1..=12).contains(value))
                .ok_or_else(|| DataError::from("LMS timetable periods must be within 1..12"))
        };
        let (from, to) = (period(start)?, period(end)?);
        if from > to {
            return Err("LMS timetable periods are out of order".into());
        }
        let (start_time, end_time) =
            crate::schedule::clock_for_periods(teaching_periods, [from, to])
                .map_err(|error| DataError::from(error.to_string()))?;
        let twin = syllabus.as_array().into_iter().flatten().find(|meeting| {
            meeting
                .get("days")
                .and_then(Value::as_array)
                .is_some_and(|days| days.iter().any(|value| value.as_u64() == Some(day)))
        });
        let mut meeting = json!({
            "days":[day], "periods":(from..=to).collect::<Vec<_>>(),
            "startTime":start_time, "endTime":end_time,
            "room":twin.and_then(|twin| twin.get("room")).cloned().unwrap_or_else(|| json!(""))
        });
        if let Some(names) = twin
            .and_then(|twin| twin.get("roomNames"))
            .filter(|names| !names.is_null())
        {
            meeting["roomNames"] = names.clone();
        }
        meetings.push(meeting);
    }
    Ok(Value::Array(meetings))
}

/// Reverses the last `AddCatalogCourses` of this session: removes only the
/// plan items it placed and brings back the ones it replaced.
pub(super) fn undo_catalog_courses(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    term_id: &str,
    mut offering_ids: Vec<String>,
    receipt: Option<CatalogAddReceipt>,
) -> Result<(), DataError> {
    validate_term(term_id)?;
    offering_ids.sort();
    offering_ids.dedup();
    let receipt = receipt
        .filter(|receipt| receipt.term_id == term_id && receipt.offering_ids == offering_ids)
        .ok_or_else(|| "this add can no longer be undone".to_string())?;
    for plan in rows(current, "planItems")? {
        let Some(id) = plan.get("id").and_then(Value::as_str) else {
            continue;
        };
        if is_live(plan) && receipt.placed.iter().any(|placed| placed == id) {
            write.delete("plan_items", id, revision(plan))?;
        } else if !is_live(plan) && receipt.replaced.iter().any(|replaced| replaced == id) {
            write.put("plan_items", plan.clone(), revision(plan))?;
        }
    }
    Ok(())
}

/// The manual course a catalog offering is planned as, shared by
/// `AddCatalogCourses` and the agent's `plan_add`.
///
/// The teaching table is passed in rather than read here: one builder serves
/// both the intent path and the MCP path, and both resolve the table once.
pub(crate) fn catalog_instructor_names(offering: &Value) -> Value {
    let mut names = serde_json::Map::new();
    for language in ["ja", "en"] {
        let value = offering["instructors"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|name| name[language].as_str())
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>()
            .join(" / ");
        if !value.is_empty() {
            names.insert(language.into(), value.into());
        }
    }
    Value::Object(names)
}

pub(crate) fn planned_catalog_course(
    offering: &Value,
    course_id: &str,
    term_id: &str,
    locale: &str,
    teaching_periods: &[crate::schedule::TeachingPeriod],
) -> Result<Value, DataError> {
    let offering_id = string(offering, "id")?;
    let credits = offering
        .get("credits")
        .cloned()
        .unwrap_or_else(|| json!({"lecture":0,"exercise":0,"lab":0}));
    let total = ["lecture", "exercise", "lab"]
        .into_iter()
        .filter_map(|key| credits.get(key).and_then(Value::as_f64))
        .sum::<f64>();
    let instructor = offering
        .get("instructors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|value| localized(Some(value), locale))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" / ");
    Ok(json!({
        "id":course_id, "kind":"course", "termId":term_id, "offeringId":offering_id,
        "names":{"title":offering.get("title"),"instructor":catalog_instructor_names(offering)},
        "code":offering.get("code").and_then(Value::as_str).unwrap_or_default(),
        "title":localized(offering.get("title"), locale), "instructor":instructor, "credits":total,
        "creditSplit":credits,
        "instructionLanguage":normalize_language(offering.get("instructionLanguage").and_then(Value::as_str)),
        "color":"#556b8d", "meetings":catalog_meetings(offering, locale, teaching_periods)?,
        "url":offering.get("syllabusUrl"),
        "source":{"provider":"kyomu","externalId":offering_id}
    }))
}

pub(super) fn catalog_meetings(
    offering: &Value,
    locale: &str,
    teaching_periods: &[crate::schedule::TeachingPeriod],
) -> Result<Value, DataError> {
    let mut meetings = Vec::new();
    for entry in offering
        .get("schedule")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if entry.get("kind").and_then(Value::as_str) != Some("weekly") {
            continue;
        }
        let day = entry
            .get("day")
            .and_then(Value::as_u64)
            .ok_or_else(|| "weekly schedule is missing day".to_string())?;
        let bounds = entry
            .get("periods")
            .and_then(Value::as_array)
            .ok_or_else(|| "weekly schedule is missing periods".to_string())?;
        let start = bounds
            .first()
            .and_then(Value::as_u64)
            .ok_or_else(|| "weekly schedule start is invalid".to_string())?;
        let end = bounds
            .get(1)
            .and_then(Value::as_u64)
            .ok_or_else(|| "weekly schedule end is invalid".to_string())?;
        if start == 0 || end > 12 || start > end {
            return Err("weekly periods must be within 1..12".into());
        }
        let periods: Vec<_> = (start..=end).map(Value::from).collect();
        let bounds = [
            u8::try_from(start).map_err(|_| DataError::from("period is outside 1..12"))?,
            u8::try_from(end).map_err(|_| DataError::from("period is outside 1..12"))?,
        ];
        let (start_time, end_time) = crate::schedule::clock_for_periods(teaching_periods, bounds)
            .map_err(|error| DataError::from(error.to_string()))?;
        let room_names = entry.get("room").cloned();
        let room = localized(room_names.as_ref(), locale);
        meetings.push(json!({"days":[day],"periods":periods,"startTime":start_time,"endTime":end_time,"room":room,"roomNames":room_names}));
    }
    Ok(Value::Array(meetings))
}
