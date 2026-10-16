use super::*;

pub(super) fn apply_lms_merge(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    school_records: &Value,
    terms: Vec<String>,
    add: Vec<String>,
    keep: Vec<String>,
    retime: Vec<String>,
) -> Result<(), DataError> {
    for term in &terms {
        validate_term(term)?;
    }
    let add: HashSet<_> = add.into_iter().collect();
    let keep: HashSet<_> = keep.into_iter().collect();
    let retime: HashSet<_> = retime.into_iter().collect();
    let offerings = write
        .catalog_rows()?
        .into_iter()
        .filter_map(crate::course_query::normalize_offering)
        .collect::<Vec<_>>();
    let locale = workspace_content_locale(current);
    let teaching_periods = crate::schedule::teaching_periods(current)
        .map_err(|error| DataError::InvalidInput(error.to_string()))?;
    let diff = lms_diff(current, school_records, &terms, &offerings, locale)?;
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let lms_id_of = |entry: &Value| {
        entry
            .pointer("/lms/lmsId")
            .and_then(Value::as_u64)
            .ok_or_else(|| DataError::from("LMS course is missing lmsId"))
    };
    let mut linked = HashMap::<u64, String>::new();
    for pair in &diff.common {
        linked.insert(
            lms_id_of(&pair["lms"])?,
            string(&pair["local"], "id")?.to_string(),
        );
    }
    let stored_courses: Vec<&Value> = rows(current, "courses")?.collect();
    // Course rows this merge changes, written once each at the end.
    let mut edited = HashMap::<String, (Value, Option<u64>)>::new();
    let mut created = HashSet::<String>::new();
    let memberships: Vec<&Value> = rows(current, "memberships")?.collect();
    let mut live_members: HashSet<(String, String)> = memberships
        .iter()
        .filter(|row| is_live(row))
        .filter_map(|row| {
            Some((
                row.get("courseId")?.as_str()?.to_string(),
                row.get("termId")?.as_str()?.to_string(),
            ))
        })
        .collect();

    for entry in diff.lms_only.iter().filter(|entry| {
        entry
            .get("key")
            .and_then(Value::as_str)
            .is_some_and(|key| add.contains(key))
    }) {
        let term = string(entry, "termId")?;
        let lms = &entry["lms"];
        let lms_id = lms_id_of(entry)?;
        let offering = entry.get("offering").filter(|value| value.is_object());
        let year: i32 = term[..4]
            .parse()
            .map_err(|_| DataError::from("term year is invalid"))?;
        let offering_id = lms.get("offeringId").and_then(Value::as_str);
        let mut id = match offering_id {
            Some(offering_id) => offering_course_id(year, offering_id)?,
            None => format!("{year}-lms-{lms_id}"),
        };
        match stored_courses
            .iter()
            .find(|course| course.get("id").and_then(Value::as_str) == Some(&id))
        {
            _ if created.contains(&id) => {}
            Some(course) if is_live(course) => {}
            Some(course) => {
                write.put("manual_courses", stored_course(course), revision(course))?;
            }
            None => {
                let syllabus = offering
                    .map(|offering| catalog_meetings(offering, locale, &teaching_periods))
                    .transpose()?
                    .unwrap_or_else(|| json!([]));
                let meetings = match entry.get("gridMeetings").filter(|grid| !grid.is_null()) {
                    Some(grid) => meetings_from_grid(grid, &syllabus, &teaching_periods)?,
                    None => syllabus,
                };
                let credits = offering
                    .and_then(|offering| offering.get("credits"))
                    .cloned()
                    .unwrap_or_else(|| json!({"lecture":0,"exercise":0,"lab":0}));
                let instructors = offering
                    .and_then(|offering| offering.get("instructors"))
                    .and_then(Value::as_array);
                let mut course = json!({
                    "id":id, "kind":"course", "termId":term,
                    "names":{
                        "title":offering.and_then(|offering| offering.get("title")).or_else(|| lms.get("title")),
                        "instructor":instructors.and_then(|items| items.first()).cloned().unwrap_or_else(|| json!({}))
                    },
                    "code":entry.get("code"), "title":entry.get("title"),
                    "instructor":instructors.into_iter().flatten()
                        .map(|value| localized(Some(value), locale))
                        .filter(|value| !value.is_empty())
                        .collect::<Vec<_>>().join(" / "),
                    "credits":credit_total(&credits), "creditSplit":credits,
                    "instructionLanguage":normalize_language(offering.and_then(|offering| offering.get("instructionLanguage")).and_then(Value::as_str)),
                    "color":"#526e91", "meetings":meetings,
                    "source":{"provider":"lms","externalId":lms_id.to_string(),"fetchedAt":now}
                });
                // The LMS page is reached through the LMS record; `url` is the syllabus.
                let url = offering
                    .and_then(|offering| offering.get("syllabusUrl"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| {
                        offering_id
                            .map(|id| format!("https://syllabus.s.isct.ac.jp/courses/-/-/-/{id}"))
                    });
                let object = object_mut(&mut course, "course")?;
                if let Some(url) = url {
                    object.insert("url".into(), Value::String(url));
                }
                if let Some(offering_id) = offering_id {
                    object.insert("offeringId".into(), Value::String(offering_id.into()));
                }
                let course = write.put("manual_courses", course, None)?;
                id = string(&course, "id")?.to_string();
                created.insert(id.clone());
            }
        }
        // A 3Q–4Q course pulled in 3Q also joins 4Q; quarters inside the
        // reviewed scope are left to their own entry, which the person may
        // have unchecked.
        let mut member_terms = vec![term.to_string()];
        member_terms.extend(
            lms_terms_with_offering(lms, offering)
                .into_iter()
                .filter(|other| !terms.contains(other)),
        );
        for member_term in member_terms {
            if live_members.contains(&(id.clone(), member_term.clone())) {
                continue;
            }
            let membership_id = format!("lms-membership:{lms_id}:{member_term}");
            match memberships
                .iter()
                .find(|row| row.get("id").and_then(Value::as_str) == Some(&membership_id))
            {
                Some(row) if is_live(row) => {}
                Some(row) => {
                    write.put("manual_memberships", (*row).clone(), revision(row))?;
                }
                None => {
                    write.put(
                        "manual_memberships",
                        json!({
                            "id":membership_id, "kind":"membership", "termId":member_term,
                            "courseId":id, "status":"lms-member",
                            "evidence":{"provider":"lms","externalId":lms_id.to_string(),"fetchedAt":now}
                        }),
                        None,
                    )?;
                }
            }
            live_members.insert((id.clone(), member_term));
        }
        linked.entry(lms_id).or_insert(id);
    }

    // The latest edited copy of a live course, else its stored row.
    let draft = |edited: &HashMap<String, (Value, Option<u64>)>, id: &str| {
        edited.get(id).cloned().or_else(|| {
            stored_courses
                .iter()
                .find(|course| {
                    is_live(course) && course.get("id").and_then(Value::as_str) == Some(id)
                })
                .map(|course| (stored_course(course), revision(course)))
        })
    };
    // Courses added from LMS follow its current name and take the syllabus
    // URL and credits once the syllabus gives them.
    for pair in &diff.common {
        let local = &pair["local"];
        if local.pointer("/source/provider").and_then(Value::as_str) != Some("lms") {
            continue;
        }
        let entry = &pair["lms"];
        let offering = entry.get("offering").filter(|value| value.is_object());
        let names = offering
            .and_then(|offering| offering.get("title"))
            .or_else(|| entry.pointer("/lms/title"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let title = entry.get("title").cloned().unwrap_or_else(|| json!(""));
        let mut changed = false;
        let renamed = local
            .pointer("/overrides/title")
            .and_then(Value::as_str)
            .is_some_and(|title| !title.is_empty());
        let id = string(local, "id")?;
        let Some((mut course, rev)) = draft(&edited, id) else {
            continue;
        };
        if !renamed
            && (course.pointer("/names/title").unwrap_or(&json!({})) != &names
                || course.get("title") != Some(&title))
        {
            let instructor = course
                .pointer("/names/instructor")
                .cloned()
                .unwrap_or_else(|| json!({}));
            course["names"] = json!({"title":names,"instructor":instructor});
            course["title"] = title;
            changed = true;
        }
        if let Some(url) = offering
            .and_then(|offering| offering.get("syllabusUrl"))
            .and_then(Value::as_str)
        {
            if !course
                .get("url")
                .and_then(Value::as_str)
                .is_some_and(|url| url.starts_with("https://syllabus.s.isct.ac.jp/"))
            {
                course["url"] = Value::String(url.into());
                changed = true;
            }
        }
        if let Some(credits) = offering.and_then(|offering| offering.get("credits")) {
            let total = credit_total(credits);
            if total > 0.0 && course.get("credits").and_then(Value::as_f64) == Some(0.0) {
                course["credits"] = json!(total);
                course["creditSplit"] = credits.clone();
                changed = true;
            }
        }
        if changed {
            edited.insert(id.to_string(), (course, rev));
        }
    }
    for pair in &diff.common {
        let id = string(&pair["local"], "id")?;
        let grid = pair
            .pointer("/lms/gridMeetings")
            .filter(|grid| !grid.is_null());
        let (Some(grid), true) = (
            grid,
            pair.get("timeDiffers").and_then(Value::as_bool) == Some(true)
                && retime.contains(&format!("local:{id}")),
        ) else {
            continue;
        };
        if let Some((mut course, rev)) = draft(&edited, id) {
            course["meetings"] = meetings_from_grid(
                grid,
                course.get("meetings").unwrap_or(&json!([])),
                &teaching_periods,
            )?;
            edited.insert(id.to_string(), (course, rev));
        }
    }
    let mut edited = edited.into_iter().collect::<Vec<_>>();
    edited.sort_by(|left, right| left.0.cmp(&right.0));
    for (_, (course, rev)) in edited {
        write.put("manual_courses", course, rev)?;
    }

    for entry in &diff.local_only {
        if entry
            .get("key")
            .and_then(Value::as_str)
            .is_some_and(|key| keep.contains(key))
        {
            continue;
        }
        let term = string(entry, "termId")?;
        remove_placements(write, current, string(&entry["course"], "id")?, Some(term))?;
    }

    let mut linked = linked.into_iter().collect::<Vec<_>>();
    linked.sort();
    for (lms_id, course_id) in linked {
        write.relink_lms_records(lms_id, &course_id)?;
    }
    let mut status = current
        .get("importStatus")
        .cloned()
        .unwrap_or_else(|| json!({}));
    object_mut(&mut status, "import status")?.insert(
        "timetableReviewedAt".into(),
        Value::String(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
    );
    write.save_setting("importStatus", &status)
}

/// A course row as stored, without the overlay the snapshot adds.
pub(super) fn stored_course(course: &Value) -> Value {
    let mut course = course.clone();
    if let Some(object) = course.as_object_mut() {
        object.remove("overrides");
    }
    course
}

pub(super) fn credit_total(credits: &Value) -> f64 {
    ["lecture", "exercise", "lab"]
        .into_iter()
        .filter_map(|key| credits.get(key).and_then(Value::as_f64))
        .sum()
}

/// The course id of a catalog offering: `{year}-{offeringId}`.
pub(super) fn offering_course_id(year: i32, offering_id: &str) -> Result<String, DataError> {
    if offering_id.len() == 9 && offering_id.bytes().all(|byte| byte.is_ascii_digit()) {
        crate::data::canonical_course_id(year, offering_id)
    } else {
        Ok(format!("{year}-{offering_id}"))
    }
}

pub(super) fn lms_terms(lms: &Value) -> Vec<String> {
    let mut terms: HashSet<String> = lms
        .get("timetables")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|map| {
            map.keys()
                .filter(|term| validate_term(term).is_ok())
                .cloned()
        })
        .collect();
    if let Some(year) = lms.get("year").and_then(Value::as_i64) {
        for quarter in lms
            .get("quarters")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let term = format!("{year}-{quarter}");
            if validate_term(&term).is_ok() {
                terms.insert(term);
            }
        }
    }
    terms.into_iter().collect()
}

pub(super) fn base_code(value: &str) -> String {
    let mut value = value.trim().to_ascii_uppercase();
    if value.ends_with(']') {
        if let Some(index) = value.rfind('[') {
            value.truncate(index);
        }
    }
    if let Some((head, tail)) = value.rsplit_once('-') {
        if tail.bytes().all(|byte| byte.is_ascii_digit()) {
            value = head.to_string();
        }
    }
    value.trim().to_string()
}

/// Builds the review model for an LMS timetable merge and reports syllabus
/// details that the shell must fetch before asking again.  The returned
/// `conflicts` value is an object keyed by entry id; the React adapter may turn
/// it into a `Map` for the existing dialog.
pub fn lms_merge_preview(
    workspace: &Value,
    school_records: &Value,
    scope: &str,
    selected_term: &str,
    offerings: &[Value],
) -> Result<Value, WorkspaceError> {
    validate_term(selected_term).map_err(WorkspaceError::from)?;
    if scope != "term" && scope != "all" {
        return Err(WorkspaceError::InvalidIntent(
            "LMS merge scope must be term or all".into(),
        ));
    }
    let offerings = offerings
        .iter()
        .filter_map(|offering| crate::course_query::normalize_offering(offering.clone()))
        .collect::<Vec<_>>();
    let offering_by_id = |id: &str| {
        offerings
            .iter()
            .find(|row| row.get("id").and_then(Value::as_str) == Some(id))
    };
    let lms_courses = school_records
        .get("lmsCourses")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let selected_year = selected_term
        .get(..4)
        .and_then(|year| year.parse::<i64>().ok());
    let in_scope = |row: &Value| {
        let terms = lms_terms_with_offering(
            row,
            row.get("offeringId")
                .and_then(Value::as_str)
                .and_then(offering_by_id),
        );
        scope == "all"
            || terms.iter().any(|term| term == selected_term)
            || (terms.is_empty() && row.get("year").and_then(Value::as_i64) == selected_year)
    };
    let detail_ids = lms_courses
        .iter()
        .filter(|row| in_scope(row))
        .filter_map(|row| row.get("offeringId").and_then(Value::as_str))
        .filter(|id| {
            offering_by_id(id)
                .and_then(|row| row.get("detailed"))
                .and_then(Value::as_bool)
                != Some(true)
        })
        .map(str::to_string)
        .collect::<HashSet<_>>();

    let mut terms = if scope == "term" {
        vec![selected_term.to_string()]
    } else {
        let mut values: HashSet<String> = lms_courses
            .iter()
            .flat_map(|row| {
                lms_terms_with_offering(
                    row,
                    row.get("offeringId")
                        .and_then(Value::as_str)
                        .and_then(offering_by_id),
                )
            })
            .collect();
        values.extend(crate::schedule::placement_terms(workspace).map(str::to_string));
        values.extend(
            workspace
                .get("terms")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string),
        );
        if let Some(term) = workspace.get("currentTerm").and_then(Value::as_str) {
            values.insert(term.to_string());
        }
        values.into_iter().collect()
    };
    terms.sort();
    let diff = lms_diff(
        workspace,
        school_records,
        &terms,
        &offerings,
        workspace_content_locale(workspace),
    )
    .map_err(WorkspaceError::from)?;
    let (common, local_only, lms_only) = (diff.common, diff.local_only, diff.lms_only);
    let mut conflicts = Map::new();
    let mut group = 0_u64;
    for remote in &lms_only {
        for local in &local_only {
            let same_code = !remote
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .is_empty()
                && base_code(
                    remote
                        .get("code")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                ) == base_code(
                    local
                        .pointer("/course/code")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                );
            let reason = if same_code && remote.get("termId") != local.get("termId") {
                Some("quarter")
            } else if remote.get("termId") == local.get("termId")
                && !value_string_set(remote.get("slots").unwrap_or(&Value::Null)).is_disjoint(
                    &value_string_set(local.get("slots").unwrap_or(&Value::Null)),
                )
            {
                Some("slot")
            } else {
                None
            };
            if let Some(reason) = reason {
                let remote_key = string(remote, "key").map_err(WorkspaceError::from)?;
                let local_key = string(local, "key").map_err(WorkspaceError::from)?;
                let id = conflicts
                    .get(remote_key)
                    .or_else(|| conflicts.get(local_key))
                    .and_then(|value| value.get("group"))
                    .and_then(Value::as_u64)
                    .unwrap_or_else(|| {
                        let id = group;
                        group += 1;
                        id
                    });
                conflicts.insert(remote_key.into(), json!({"group":id,"reason":reason}));
                conflicts.insert(local_key.into(), json!({"group":id,"reason":reason}));
            }
        }
    }
    let has_differences = !local_only.is_empty()
        || !lms_only.is_empty()
        || common
            .iter()
            .any(|pair| pair.get("timeDiffers").and_then(Value::as_bool) == Some(true));
    let mut detail_ids = detail_ids.into_iter().collect::<Vec<_>>();
    detail_ids.sort();
    Ok(
        json!({"terms":terms,"detailIds":detail_ids,"common":common,"localOnly":local_only,"lmsOnly":lms_only,"conflicts":conflicts,"hasDifferences":has_differences}),
    )
}

/// The LMS review diff of `terms`: matched (`common`), local-only and
/// LMS-only entries.  Shared by the preview and the applied merge so both see
/// the same entries.
pub(super) struct LmsDiff {
    common: Vec<Value>,
    local_only: Vec<Value>,
    lms_only: Vec<Value>,
}

pub(super) fn lms_diff(
    workspace: &Value,
    school_records: &Value,
    terms: &[String],
    offerings: &[Value],
    locale: &str,
) -> Result<LmsDiff, DataError> {
    let lms_courses = school_records
        .get("lmsCourses")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let term_scope: HashSet<_> = terms.iter().cloned().collect();
    let offering_by_id = |id: &str| {
        offerings
            .iter()
            .find(|row| row.get("id").and_then(Value::as_str) == Some(id))
    };
    let mut remote = Vec::<Value>::new();
    for row in lms_courses
        .iter()
        .filter(|row| row.get("hidden").and_then(Value::as_bool) != Some(true))
    {
        let offering = row
            .get("offeringId")
            .and_then(Value::as_str)
            .and_then(offering_by_id);
        for term in lms_terms_with_offering(row, offering)
            .into_iter()
            .filter(|term| term_scope.contains(term))
        {
            let grid = row.pointer(&format!("/timetables/{term}"));
            let slots = grid
                .map(grid_slots)
                .unwrap_or_else(|| offering.map(offering_slots).unwrap_or_default());
            let shortname = row
                .get("shortname")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let title = localized(
                offering
                    .and_then(|row| row.get("title"))
                    .or_else(|| row.get("title")),
                locale,
            );
            let title = crate::school_parser::lms::decode_entities(if title.is_empty() {
                shortname
            } else {
                &title
            });
            let code = offering
                .and_then(|row| row.get("code"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| {
                    shortname
                        .strip_suffix(']')
                        .and_then(|value| value.rsplit_once('[').map(|(head, _)| head.trim()))
                        .unwrap_or(shortname)
                        .to_string()
                });
            remote.push(json!({
                "key":format!("lms:{}:{term}", row.get("lmsId").and_then(Value::as_u64).unwrap_or_default()),
                "termId":term, "lms":row, "offering":offering, "title":title,
                "code":code, "slots":slots, "gridMeetings":grid
            }));
        }
    }
    let mut common = Vec::new();
    let mut local_only = Vec::new();
    let mut matched = HashSet::new();
    for term in terms {
        // The timetable of the quarter: a 3Q–4Q course is compared in each.
        for course in &crate::schedule::scheduled_courses(workspace, term) {
            let found = remote.iter().find(|entry| {
                entry.get("termId").and_then(Value::as_str) == Some(term)
                    && !matched
                        .contains(entry.get("key").and_then(Value::as_str).unwrap_or_default())
                    && ((course.get("offeringId").and_then(Value::as_str).is_some()
                        && course.get("offeringId") == entry.pointer("/lms/offeringId"))
                        || base_code(
                            course
                                .get("code")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                        ) == base_code(
                            entry
                                .get("code")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                        ))
            });
            if let Some(entry) = found {
                matched.insert(string(entry, "key")?.to_string());
                let time_differs = entry
                    .get("gridMeetings")
                    .is_some_and(|grid| !grid.is_null())
                    && course_slots(course)
                        != value_string_set(entry.get("slots").unwrap_or(&Value::Null));
                common.push(
                    json!({"termId":term,"local":course,"lms":entry,"timeDiffers":time_differs}),
                );
            } else {
                local_only.push(json!({"key":format!("local:{}",string(course,"id")?),"termId":term,"course":course,"slots":course_slots(course)}));
            }
        }
    }
    let lms_only = remote
        .into_iter()
        .filter(|entry| {
            !matched.contains(entry.get("key").and_then(Value::as_str).unwrap_or_default())
        })
        .collect::<Vec<_>>();
    Ok(LmsDiff {
        common,
        local_only,
        lms_only,
    })
}

pub(super) fn lms_terms_with_offering(lms: &Value, offering: Option<&Value>) -> Vec<String> {
    let mut terms: HashSet<String> = lms_terms(lms).into_iter().collect();
    let year = lms.get("year").and_then(Value::as_i64).or_else(|| {
        offering
            .and_then(|row| row.get("year"))
            .and_then(Value::as_i64)
    });
    if let Some(year) = year {
        let quarters = offering
            .and_then(|row| row.get("quarters"))
            .and_then(Value::as_array)
            .filter(|quarters| !quarters.is_empty())
            .or_else(|| lms.get("quarters").and_then(Value::as_array));
        for quarter in quarters.into_iter().flatten().filter_map(Value::as_str) {
            let term = format!("{year}-{quarter}");
            if validate_term(&term).is_ok() {
                terms.insert(term);
            }
        }
    }
    terms.into_iter().collect()
}
