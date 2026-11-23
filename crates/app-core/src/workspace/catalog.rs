use super::*;
use crate::locale::{pick, ContentLanguage};

#[derive(Clone)]
pub(crate) struct CatalogAddReceipt {
    term_id: String,
    offering_ids: Vec<String>,
    changes: Vec<PlanChange>,
    #[cfg(feature = "agent")]
    pub(crate) placements: Vec<CatalogPlacement>,
}

#[cfg(feature = "agent")]
#[derive(Clone)]
pub(crate) struct CatalogPlacement {
    pub(crate) course_id: String,
    pub(crate) plan_id: String,
    pub(crate) term_id: String,
}

#[derive(Clone)]
struct PlanChange {
    after: Value,
    was_live: bool,
}

/// Adds catalog offerings to the timetable of `term_id`, the quarter on
/// screen.  An offering running over several quarters (3Q–4Q) that is added
/// in one of them is planned into each of them; added outside its quarters
/// it stays in `term_id`.  A local draft (plan item) in `term_id` whose
/// periods clash leaves the timetable; school-registered courses stay.
fn catalog_plan_terms(offering: &Value, term_id: &str) -> Result<Vec<String>, DataError> {
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
    let offerings = offerings
        .iter()
        .map(|submitted| {
            let id = string(submitted, "id")?;
            write.catalog_row(id)?.ok_or_else(|| {
                DataError::InvalidInput(format!("catalog offering {id} is no longer available"))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    plan_catalog_courses(write, current, term_id, offerings, true)
}

pub(crate) fn plan_catalog_courses(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    term_id: &str,
    offerings: Vec<Value>,
    replace_conflicts: bool,
) -> Result<CatalogAddReceipt, DataError> {
    validate_term(term_id)?;
    let teaching_periods = crate::schedule::teaching_periods(current)?;
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
    let language = ContentLanguage::of(current);
    let plan_year = &term_id[..4];
    let mut offering_ids = offerings
        .iter()
        .map(|offering| string(offering, "id").map(str::to_string))
        .collect::<Result<Vec<_>, _>>()?;
    offering_ids.sort();
    offering_ids.dedup();
    // Work with persisted plan rows so receipt checks keep the record's own
    // identity and revision even when its course is projected to another id.
    let mut placed_courses = if replace_conflicts {
        crate::schedule::scheduled_courses(current, term_id)
    } else {
        Vec::new()
    };
    let mut descriptions = write.rows("manual_courses")?;
    let mut plans = write.rows("plan_items")?;
    let previously_planned: HashSet<_> = plans
        .iter()
        .filter(|row| is_live(row))
        .filter_map(|row| row["id"].as_str().map(str::to_owned))
        .collect();
    #[cfg(feature = "agent")]
    let mut placements = Vec::new();
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
        if replace_conflicts {
            let replaceable = crate::course_query::replaceable_drafts(
                &placed_courses,
                &plans,
                crate::schedule::array(current, "memberships"),
                term_id,
            );
            let victims = crate::course_query::clashing_courses(&offering, &replaceable);
            for plan in plans.iter_mut().filter(|row| {
                is_live(row)
                    && row["termId"] == term_id
                    && victims
                        .iter()
                        .any(|victim| row["offeringId"] == victim["offeringId"])
            }) {
                *plan = tombstone_plan(write, plan)?;
            }
        }
        let descriptor = crate::data::catalog_drafts::description(&descriptions, offering_id);
        let draft = match descriptor {
            Some(previous) => {
                let mut refreshed = previous.clone();
                refreshed["meetings"] = catalog_meetings(&offering, language, &teaching_periods)?;
                if refreshed["meetings"] != previous["meetings"] {
                    write.put("manual_courses", refreshed, revision(previous))?
                } else {
                    refreshed
                }
            }
            None => {
                let id = format!("catalog-description:{year}:{offering_id}");
                let row =
                    planned_catalog_course(&offering, &id, term_id, language, &teaching_periods)?;
                write.put("manual_courses", row, Some(0))?
            }
        };
        descriptions.retain(|row| row["id"] != draft["id"]);
        descriptions.push(draft.clone());
        // A later entry in the batch can replace this one using its current catalog slots.
        if replace_conflicts
            && !placed_courses
                .iter()
                .any(|course| course["id"] == course_id)
        {
            let mut placed = draft;
            placed["id"] = course_id.clone().into();
            placed_courses.push(placed);
        }
        for plan_term in catalog_plan_terms(&offering, term_id)? {
            let existing = plans.iter().find(|row| {
                is_live(row) && row["termId"] == plan_term && row["offeringId"] == offering_id
            });
            let plan_id = match existing {
                Some(row) => string(row, "id")?.to_owned(),
                None => format!("catalog-plan:{plan_term}:{offering_id}"),
            };
            if existing.is_none() {
                match plans.iter().position(|row| row["id"] == plan_id) {
                    Some(index) => {
                        let mut revived = plans[index].clone();
                        revived["courseId"] = json!(course_id);
                        plans[index] = write.put("plan_items", revived, revision(&plans[index]))?;
                    }
                    None => plans.push(write.put(
                        "plan_items",
                        json!({"id":plan_id,"kind":"plan-item","termId":plan_term,
                            "courseId":course_id,"offeringId":offering_id,"state":"planned"}),
                        Some(0),
                    )?),
                }
            }
            #[cfg(feature = "agent")]
            placements.push(CatalogPlacement {
                course_id: course_id.clone(),
                plan_id,
                term_id: plan_term,
            });
        }
    }
    // Within a batch, one offering can replace another just added by
    // that batch. Undo only the difference from before the whole batch.
    let changes = plans
        .into_iter()
        .filter_map(|after| {
            let was_live = previously_planned.contains(after["id"].as_str()?);
            (was_live != is_live(&after)).then_some(PlanChange { after, was_live })
        })
        .collect();
    Ok(CatalogAddReceipt {
        term_id: term_id.into(),
        offering_ids,
        changes,
        #[cfg(feature = "agent")]
        placements,
    })
}

/// Delete every observed draft named by this action, including copies from
/// another device. Their own IDs and revisions are retained for sync.
pub(crate) fn remove_catalog_plans<'a>(
    write: &WorkspaceWrite<'_, '_>,
    plans: impl IntoIterator<Item = &'a Value>,
) -> Result<usize, DataError> {
    let mut removed = 0;
    for plan in plans {
        if is_live(plan) {
            tombstone_plan(write, plan)?;
            removed += 1;
        }
    }
    Ok(removed)
}

fn tombstone_plan(write: &WorkspaceWrite<'_, '_>, plan: &Value) -> Result<Value, DataError> {
    write
        .delete("plan_items", string(plan, "id")?, revision(plan))?
        .ok_or_else(|| DataError::storage("the catalog plan was not found"))
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

/// Reverses the last `AddCatalogCourses` of this session, provided its
/// affected plans still match the add's result.
pub(super) fn undo_catalog_courses(
    write: &WorkspaceWrite<'_, '_>,
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
    let plans = write.rows("plan_items")?;
    for change in receipt.changes {
        if !plans.contains(&change.after) {
            return Err("this add can no longer be undone".into());
        }
        let expected = revision(&change.after);
        if change.was_live {
            write.put("plan_items", change.after, expected)?;
        } else {
            write.delete("plan_items", string(&change.after, "id")?, expected)?;
        }
    }
    Ok(())
}

/// Localized instructor names shared by the catalog description and school course projection.
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
    language: ContentLanguage,
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
        .filter_map(|value| pick(Some(value), language))
        .collect::<Vec<_>>()
        .join(" / ");
    Ok(json!({
        "id":course_id, "kind":"course", "termId":term_id, "offeringId":offering_id,
        "names":{"title":offering.get("title"),"instructor":catalog_instructor_names(offering)},
        "code":offering.get("code").and_then(Value::as_str).unwrap_or_default(),
        "title":pick(offering.get("title"), language).unwrap_or_default(), "instructor":instructor, "credits":total,
        "creditSplit":credits,
        "instructionLanguage":normalize_language(offering.get("instructionLanguage").and_then(Value::as_str)),
        "color":"#556b8d", "meetings":catalog_meetings(offering, language, teaching_periods)?,
        "url":offering.get("syllabusUrl"),
        "source":{"provider":"kyomu","externalId":offering_id}
    }))
}

pub(crate) fn catalog_meetings(
    offering: &Value,
    language: ContentLanguage,
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
        let room = pick(room_names.as_ref(), language).unwrap_or_default();
        meetings.push(json!({"days":[day],"periods":periods,"startTime":start_time,"endTime":end_time,"room":room,"roomNames":room_names}));
    }
    Ok(Value::Array(meetings))
}
