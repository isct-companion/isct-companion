//! LMS owns timetable membership and clocks. School caches never
//! rewrite user plans, colours, notes or edits in their backing tables.
use super::*;
use std::collections::HashMap;

fn text<'a>(row: &'a Value, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or_default()
}
fn array<'a>(row: &'a Value, key: &str) -> &'a [Value] {
    row.get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

pub(super) fn project(connection: &Connection, workspace: &mut Value) -> Result<(), DataError> {
    if let Some(identity) = read_setting(connection, "kyomuAcademicIdentity")? {
        if text(&workspace["settings"], "departmentId").is_empty() {
            let departments: Vec<Value> =
                serde_json::from_str(include_str!("../course_query/departments.json"))?;
            if let Some(department) = departments.iter().find(|d| {
                d.pointer("/name/ja").and_then(Value::as_str) == identity["department"].as_str()
            }) {
                workspace["settings"]["departmentId"] = department["id"].clone();
            }
        }
        if let Ok(date) =
            chrono::NaiveDate::parse_from_str(text(&identity, "admittedOn"), "%Y-%m-%d")
        {
            let programme = text(&identity, "programme");
            let stage = if programme.contains("学士") {
                "undergraduate"
            } else if programme.contains("修士") {
                "master"
            } else if programme.contains("博士") {
                "doctoral"
            } else if programme.contains("専門職") {
                "professional"
            } else {
                "unknown"
            };
            // Existing user choices win; a fresh profile uses the official entry date.
            if workspace["academicProfile"]["admissionYear"].is_null() {
                workspace["academicProfile"]["admissionYear"] = date.year().into();
            }
            if array(&workspace["academicProfile"], "studyEnrollments").is_empty()
                && stage != "unknown"
            {
                let quarter = match date.month() {
                    4..=6 => 1,
                    7..=9 => 2,
                    10..=12 => 3,
                    _ => 4,
                };
                let year = date.year() - i32::from(date.month() < 4);
                let category = text(&identity, "entryCategory");
                let kind = if category.contains("編入") {
                    "transfer"
                } else if ["前期日程", "後期日程", "一般", "推薦", "総合型"]
                    .iter()
                    .any(|v| category.contains(v))
                {
                    "regular"
                } else {
                    "unknown"
                };
                let mut enrollment = json!({"stage":stage,"entryKind":kind,"admittedTerm":format!("{year}-{quarter}Q")});
                if kind == "regular" {
                    enrollment["entryGrade"] = json!(1);
                }
                workspace["academicProfile"]["studyEnrollments"] = json!([enrollment]);
            }
        }
    }
    let teaching = crate::schedule::teaching_periods(workspace)
        .map_err(|e| DataError::InvalidInput(e.to_string()))?;
    let user_courses: HashSet<String> = read_rows(connection, "manual_courses")?
        .iter()
        .map(|row| text(row, "id").to_string())
        .collect();
    let mut aliases = HashMap::new();
    let mut lms_links = HashMap::new();
    let lms_courses = read_visible_rows(connection, "lms_courses")?;
    for lms in &lms_courses {
        let lms_id = lms["lmsId"].as_u64().unwrap_or_default();
        let year = lms["year"].as_i64().unwrap_or_default();
        let offering = text(&lms, "offeringId");
        let code = lms_course_code(text(&lms, "shortname"));
        let candidates: Vec<_> = array(workspace, "courses")
            .iter()
            .filter(|course| {
                if !offering.is_empty() {
                    return text(course, "offeringId") == offering;
                }
                !code.is_empty()
                    && text(course, "code").eq_ignore_ascii_case(&code)
                    && text(course, "termId").starts_with(&format!("{year}-"))
                    && array(&lms, "quarters").iter().any(|quarter| {
                        text(course, "termId")
                            == format!("{year}-{}", quarter.as_str().unwrap_or(""))
                            || array(workspace, "memberships").iter().any(|m| {
                                text(m, "courseId") == text(course, "id")
                                    && text(m, "termId")
                                        == format!("{year}-{}", quarter.as_str().unwrap_or(""))
                            })
                    })
            })
            .collect();
        // Exact offering id wins. Without one, only a unique code/year/quarter
        // match is safe; class-section and ambiguous matches stay unlinked.
        if candidates.len() == 1 {
            let id = text(candidates[0], "id").to_string();
            lms_links.insert(lms_id, id.clone());
            aliases.insert(format!("{year}-lms-{lms_id}"), id);
        }
    }
    project_lms_timetable(
        workspace,
        &lms_courses,
        &user_courses,
        &teaching,
        &mut lms_links,
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
    for field in ["assignments", "materials", "memberships", "planItems"] {
        for row in workspace[field].as_array_mut().into_iter().flatten() {
            if let Some(id) = row["lmsCourseId"]
                .as_u64()
                .and_then(|id| lms_links.get(&id))
                .or_else(|| aliases.get(text(row, "courseId")))
            {
                row["courseId"] = id.clone().into();
            }
        }
    }
    // Every quarter uses the same cached bilingual metadata. Display language
    // is a read projection, never a mutation of the school's canonical title.
    let offerings = array(workspace, "offerings").to_vec();
    for course in workspace["courses"].as_array_mut().into_iter().flatten() {
        if course["source"]["provider"] == "manual" {
            continue;
        }
        if let Some(offering) = offerings
            .iter()
            .find(|o| text(o, "id") == text(course, "offeringId"))
        {
            if !user_courses.contains(text(course, "id")) {
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
    Ok(())
}

/// Project cached LMS quarter grids without rewriting user records.
fn project_lms_timetable(
    workspace: &mut Value,
    lms_courses: &[Value],
    user_courses: &HashSet<String>,
    teaching: &[crate::schedule::TeachingPeriod],
    links: &mut HashMap<u64, String>,
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
        let catalog = array(workspace, "offerings")
            .iter()
            .find(|o| !offering_id.is_empty() && text(o, "id") == offering_id)
            .cloned();
        // Category metadata bootstraps old caches. A captured grid then owns
        // this quarter: arrays mean present (including non-weekly), null absent.
        let mut terms = crate::workspace::lms_terms_with_offering(lms, None);
        terms.retain(|term| {
            term.starts_with(&format!("{year}-"))
                && crate::schedule::Term::parse(term).is_ok()
                && !lms["timetables"].get(term).is_some_and(Value::is_null)
        });
        terms.sort();
        terms.dedup();
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
        // Never merge ambiguous code/class matches by manufacturing a new alias.
        if offering_id.is_empty()
            && !links.contains_key(&lms_id)
            && array(workspace, "courses").iter().any(|c| {
                text(c, "code") == lms_course_code(text(lms, "shortname"))
                    && terms.iter().any(|term| text(c, "termId") == term)
            })
        {
            continue;
        }
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
                    for meeting in meetings.as_array_mut().into_iter().flatten() {
                        meeting["termIds"] = json!([term]);
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
                workspace["memberships"]
                    .as_array_mut()
                    .ok_or_else(|| DataError::storage("Missing memberships."))?
                    .push(
                        json!({"id":format!("lms-membership:{lms_id}:{term}"),"kind":"membership",
                        "courseId":id,"termId":term,"status":"lms-member",
                        "evidence":{"provider":"lms","externalId":lms_id.to_string()},
                        "rev":lms["rev"],"updatedAt":lms["updatedAt"],"updatedBy":"lms"}),
                    );
            }
            if !array(workspace, "terms").contains(&json!(term)) {
                workspace["terms"]
                    .as_array_mut()
                    .ok_or_else(|| DataError::storage("Missing terms."))?
                    .push(json!(term));
            }
        }
        if let Some(index) = existing {
            workspace["courses"][index] = course;
        } else {
            workspace["courses"]
                .as_array_mut()
                .ok_or_else(|| DataError::storage("Missing courses."))?
                .push(course);
        }
        links.insert(lms_id, id);
    }
    Ok(())
}
