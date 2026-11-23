//! Personal catalog plans and their offline descriptions. The descriptions
//! are input to draft projection only, even when a school course shares
//! their offering key.
use super::*;
use crate::schedule::text_or_empty as text;

pub(crate) fn is_description(row: &Value) -> bool {
    row["source"]["provider"] == "kyomu" && row["offeringId"].is_string()
}

pub(crate) fn description<'a>(rows: &'a [Value], offering: &str) -> Option<&'a Value> {
    rows.iter()
        .filter(|row| is_description(row) && row["offeringId"] == offering)
        .max_by_key(|row| {
            (
                row["deletedAt"].is_null(),
                text(row, "updatedAt"),
                text(row, "id"),
            )
        })
}

/// The academic year is part of the school's offering number. Catalog-only
/// identifiers use the year of the saved quarter.
pub(super) fn offering_key(row: &Value) -> Option<String> {
    let offering = row["offeringId"].as_str().filter(|id| !id.is_empty())?;
    let year = offering
        .get(..4)
        .and_then(|year| year.parse().ok())
        .or_else(|| text(row, "termId").get(..4)?.parse().ok())?;
    Some(course_keys::catalog_key(year, offering))
}

/// The device's course for a plan or a saved description. The row's own ID
/// is independent of this mapping and remains the sync and undo identity.
pub(super) fn course_id(connection: &Connection, row: &Value) -> Result<Option<String>, DataError> {
    let id = match offering_key(row) {
        Some(key) => course_keys::lookup(connection, &key)?,
        None => None,
    };
    Ok(id.or_else(|| row["courseId"].as_str().map(str::to_owned)))
}

pub(super) fn project_plan_courses(
    connection: &Connection,
    plans: &mut [Value],
) -> Result<(), DataError> {
    for plan in plans {
        if let Some(id) = course_id(connection, plan)? {
            plan["courseId"] = id.into();
        }
    }
    Ok(())
}

/// Adds the courses or quarters that have an active personal plan. School
/// placements retain their own time and rooms, including quarters where
/// LMS listed the course without a fixed schedule or the user hid it.
pub(super) fn project_courses(
    workspace: &mut Value,
    descriptions: &[Value],
    plans: &[Value],
    mut school: HashSet<(String, String)>,
    catalog: &HashMap<String, Value>,
    teaching: &[crate::schedule::TeachingPeriod],
) -> Result<HashSet<String>, DataError> {
    use crate::schedule::array;
    let language = crate::locale::ContentLanguage::of(workspace);
    school.extend(
        array(workspace, "memberships")
            .iter()
            .filter(|row| row["deletedAt"].is_null() && row["status"] != "withdrawn")
            .map(|row| {
                (
                    text(row, "courseId").to_owned(),
                    text(row, "termId").to_owned(),
                )
            }),
    );
    let mut projected = HashSet::new();
    let courses = workspace["courses"]
        .as_array_mut()
        .ok_or_else(|| DataError::storage("Missing courses."))?;
    // Older devices could save separate records for the same placement.
    // Keep their IDs for sync and project their shared course only once.
    let mut used = HashSet::new();
    for plan in plans {
        let (id, term, offering) = (
            text(plan, "courseId"),
            text(plan, "termId"),
            text(plan, "offeringId"),
        );
        if id.is_empty() || offering.is_empty() || !used.insert((id, term)) {
            continue;
        }
        let mut draft = match description(descriptions, offering) {
            Some(row) => {
                let mut course = row.clone();
                if let Some(offering) = catalog.get(offering) {
                    course["meetings"] =
                        crate::workspace::catalog_meetings(offering, language, teaching)?;
                }
                course
            }
            None => {
                let Some(offering) = catalog.get(offering) else {
                    continue;
                };
                let mut draft = crate::workspace::planned_catalog_course(
                    offering, id, term, language, teaching,
                )?;
                draft["rev"] = plan["rev"].clone();
                draft["updatedAt"] = plan["updatedAt"].clone();
                draft["updatedBy"] = plan["updatedBy"].clone();
                draft
            }
        };
        draft["id"] = id.into();
        if let Some(object) = draft.as_object_mut() {
            // Older versions deleted the description with one of its plans.
            // Its content is still the offline description of a live plan.
            object.remove("deletedAt");
        }
        let meetings = array(&draft, "meetings")
            .iter()
            .cloned()
            .map(|mut meeting| {
                meeting["termIds"] = json!([term]);
                meeting
            })
            .collect::<Vec<_>>();
        if let Some(course) = courses.iter_mut().find(|course| course["id"] == id) {
            if school.contains(&(id.into(), term.into()))
                || course["source"]["provider"] == "manual"
            {
                continue;
            }
            let clocks = course["meetings"]
                .as_array_mut()
                .ok_or_else(|| DataError::storage("Missing meetings."))?;
            clocks.retain(|meeting| !array(meeting, "termIds").iter().any(|id| id == term));
            clocks.extend(meetings);
        } else {
            draft["meetings"] = Value::Array(meetings);
            courses.push(draft);
            projected.insert(id.to_owned());
        }
    }
    Ok(projected)
}
