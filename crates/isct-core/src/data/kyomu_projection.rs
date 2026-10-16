//! One read model for official enrollment and clocks. School caches never
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
    let registrations = read_visible_rows(connection, "registrations")?;
    let states = read_visible_rows(connection, "registration_states")?;
    let covered: HashSet<String> = states
        .iter()
        .filter_map(|s| {
            s.get("academicYear")
                .and_then(Value::as_i64)
                .map(|year| (s, year))
        })
        .flat_map(|(s, year)| {
            array(s, "quarters")
                .iter()
                .filter_map(move |q| q.as_str().map(|q| format!("{year}-{q}")))
        })
        .collect();
    // LMS enrollment can include old, optional or staff-created courses. It
    // remains available in Lecture Desk but cannot add courses to an official term.
    if let Some(memberships) = workspace["memberships"].as_array_mut() {
        memberships.retain(|row| {
            !(text(row, "status") == "lms-member" && covered.contains(text(row, "termId")))
        });
    }
    let teaching = crate::schedule::teaching_periods(workspace)
        .map_err(|e| DataError::InvalidInput(e.to_string()))?;
    let mut aliases = HashMap::new();
    for registration in registrations.iter().filter(|r| {
        text(r, "id").starts_with("kyomu-registration:") && text(r, "courseState") != "rejected"
    }) {
        let offering_id = text(registration, "offeringId");
        let year: i32 = offering_id
            .get(..4)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| DataError::InvalidInput("Invalid official course year.".into()))?;
        let canonical = canonical_course_id(year, offering_id)?;
        let schedule = array(registration, "schedule");
        let quarters = crate::kyomu_ingest::registration_quarters(registration);
        if quarters.is_empty() {
            continue;
        }
        let catalog = array(workspace, "offerings")
            .iter()
            .find(|row| text(row, "id") == offering_id)
            .cloned();
        let courses = workspace["courses"]
            .as_array_mut()
            .ok_or_else(|| DataError::storage("Missing courses."))?;
        let index = courses.iter().position(|course| {
            text(course, "offeringId") == offering_id || text(course, "id") == canonical
        });
        let mut course = index.map(|i| courses[i].clone()).unwrap_or_else(|| json!({
            "id":canonical,"kind":"course","color":"#526e91","rev":1,
            "updatedAt":registration["updatedAt"],"updatedBy":"kyomu", "instructionLanguage":"other",
            "credits":0,"creditSplit":{"lecture":0,"exercise":0,"lab":0}
        }));
        if let Some(object) = course.as_object_mut() {
            object.remove("deletedAt");
            object.remove("missingSince");
        }
        if let Some(offering) = &catalog {
            course["names"] = json!({"title":offering["title"],"instructor":crate::workspace::catalog_instructor_names(offering)});
            course["instructionLanguage"] = offering["instructionLanguage"].clone();
        }
        let id = text(&course, "id").to_string();
        aliases.insert(canonical, id.clone());
        course["offeringId"] = offering_id.into();
        course["termId"] = format!("{year}-{}", quarters[0]).into();
        course["code"] = text(registration, "code").into();
        course["title"] = text(registration, "title").into();
        course["instructor"] = text(registration, "instructor").into();
        let mut names = course.get("names").cloned().unwrap_or_else(|| json!({}));
        names["title"]["ja"] = registration["title"].clone();
        names["instructor"]["ja"] = registration["instructor"].clone();
        course["names"] = names;
        course["url"] = format!("https://syllabus.s.isct.ac.jp/courses?jwc={offering_id}").into();
        course["source"] = json!({"provider":"kyomu","externalId":offering_id});
        let declared = registrations
            .iter()
            .find(|r| text(r, "offeringId") == offering_id && text(r, "kind") == "declared");
        if let Some(credits) = registration
            .get("credits")
            .filter(|c| c.is_object())
            .or_else(|| {
                declared
                    .and_then(|r| r.get("credits"))
                    .filter(|c| c.is_object())
            })
        {
            course["creditSplit"] = credits.clone();
            course["credits"] = ["lecture", "exercise", "lab"]
                .iter()
                .filter_map(|k| credits[*k].as_f64())
                .sum::<f64>()
                .into();
        }
        if let Some(total) = declared
            .and_then(|row| row.get("effectiveCredits"))
            .and_then(Value::as_f64)
        {
            let raw = course["credits"].as_f64().unwrap_or_default();
            if raw > 0.0 {
                for key in ["lecture", "exercise", "lab"] {
                    let split = course["creditSplit"][key].as_f64().unwrap_or_default();
                    course["creditSplit"][key] = json!(split * total / raw);
                }
            }
            course["credits"] = json!(total);
        }
        let mut meetings = Vec::new();
        for slot in schedule
            .iter()
            .filter(|slot| quarters.iter().any(|q| q == text(slot, "quarter")))
        {
            let Some(day) = "月火水木金土日"
                .chars()
                .position(|d| text(slot, "day").starts_with(d))
                .map(|i| i + 1)
            else {
                continue;
            };
            let periods: Vec<u8> = array(slot, "periods")
                .iter()
                .filter_map(Value::as_u64)
                .filter_map(|v| u8::try_from(v).ok())
                .collect();
            let (Some(first), Some(last)) = (periods.first(), periods.last()) else {
                continue;
            };
            let (start, end) = crate::schedule::clock_for_periods(&teaching, [*first, *last])
                .map_err(|e| DataError::InvalidInput(e.to_string()))?;
            meetings.push(
                json!({"days":[day],"periods":periods,"startTime":start,"endTime":end,
                "room":text(slot,"room"),"termIds":[format!("{year}-{}",text(slot,"quarter"))]}),
            );
        }
        course["meetings"] = json!(meetings);
        if let Some(index) = index {
            courses[index] = course;
        } else {
            courses.push(course);
        }
        for quarter in quarters {
            let term = format!("{year}-{quarter}");
            if !array(workspace, "terms")
                .iter()
                .any(|t| t.as_str() == Some(&term))
            {
                workspace["terms"]
                    .as_array_mut()
                    .ok_or_else(|| DataError::storage("Missing terms."))?
                    .push(term.clone().into());
            }
            let membership = json!({"id":format!("kyomu-membership:{offering_id}:{quarter}"),"kind":"membership", "courseId":id,"termId":term,
                "status":if states.iter().any(|s| s["academicYear"].as_i64() == Some(i64::from(year)) && array(s,"quarters").contains(&json!(quarter)) && text(s,"state") == "registered") {"registered"} else {"declared"},
                "evidence":{"provider":"kyomu","externalId":offering_id},"rev":registration["rev"],"updatedAt":registration["updatedAt"],"updatedBy":"kyomu"});
            workspace["memberships"]
                .as_array_mut()
                .ok_or_else(|| DataError::storage("Missing memberships."))?
                .push(membership);
        }
    }
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
                text(course, "code").eq_ignore_ascii_case(&code)
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
    for (lms_id, id) in &lms_links {
        if let Some(course) = workspace["courses"]
            .as_array_mut()
            .into_iter()
            .flatten()
            .find(|course| text(course, "id") == id)
        {
            if course["lmsCourseId"]
                .as_u64()
                .is_none_or(|current| *lms_id > current)
            {
                course["lmsCourseId"] = json!(lms_id);
            }
        }
    }
    for field in ["assignments", "materials"] {
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
        if let Some(offering) = offerings
            .iter()
            .find(|o| text(o, "id") == text(course, "offeringId"))
        {
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
