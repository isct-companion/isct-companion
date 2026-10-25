use super::*;

pub(super) fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub(super) fn quarter_for_month(month: u32) -> &'static str {
    match month {
        12 | 1..=3 => "4Q",
        10 | 11 => "3Q",
        6..=9 => "2Q",
        _ => "1Q",
    }
}

/// One authoritative resolver for every native consumer. User-configured
/// starts intentionally override subscribed calendar boundaries. When no
/// usable boundary exists, keep the user's selected term instead of guessing
/// from hard-coded month ranges.
pub(super) fn current_term_for(workspace: &Value, date: chrono::NaiveDate) -> Option<String> {
    let mut starts: Vec<(chrono::NaiveDate, String, u8)> = Vec::new();
    let mut configured = HashSet::new();
    if let Some(years) = workspace
        .pointer("/academicProfile/years")
        .and_then(Value::as_array)
    {
        for year in years {
            let Some(academic_year) = year.get("academicYear").and_then(Value::as_i64) else {
                continue;
            };
            for quarter in ["1Q", "2Q", "3Q", "4Q"] {
                if let Some(start) = year
                    .pointer(&format!("/quarterStarts/{quarter}"))
                    .and_then(Value::as_str)
                    .and_then(|value| chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
                {
                    configured.insert((academic_year, quarter));
                    starts.push((start, format!("{academic_year}-{quarter}"), 2));
                }
            }
        }
    }
    if let Some(days) = workspace.get("academicCalendar").and_then(Value::as_array) {
        for day in days {
            if day.get("deletedAt").is_some() || day.get("missingSince").is_some() {
                continue;
            }
            if day.get("type").and_then(Value::as_str) != Some("quarter-start") {
                continue;
            }
            if let (Some(start), Some(year), Some(quarter)) = (
                day.get("date")
                    .and_then(Value::as_str)
                    .and_then(|value| chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()),
                day.get("academicYear").and_then(Value::as_i64),
                day.get("quarter").and_then(Value::as_str),
            ) {
                if configured.contains(&(year, quarter)) {
                    continue;
                }
                starts.push((start, format!("{year}-{quarter}"), 1));
            }
        }
    }
    starts
        .into_iter()
        .filter(|(start, _, _)| start <= &date)
        .max_by_key(|(start, _, priority)| (*start, *priority))
        .map(|(_, term, _)| term)
        .or_else(|| {
            workspace
                .get("selectedTerm")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
}

/// Build the same first-launch workspace used by persistence and preview fixtures.
pub fn initial_workspace(
    date: chrono::NaiveDate,
    device_id: &str,
    updated_at: &str,
) -> Result<Value, DataError> {
    let mut workspace = initial_workspace_value(date, device_id, updated_at);
    let _: crate::contracts::Workspace = serde_json::from_value(workspace.clone())
        .map_err(|error| DataError::InvalidInput(format!("invalid initial workspace: {error}")))?;
    normalize_optional_nulls(&mut workspace);
    Ok(workspace)
}

pub(super) fn empty_workspace() -> Value {
    initial_workspace_value(
        (Utc::now() + chrono::Duration::hours(9)).date_naive(),
        &Uuid::new_v4().to_string(),
        &now(),
    )
}

fn initial_workspace_value(date: chrono::NaiveDate, device_id: &str, updated_at: &str) -> Value {
    let year = if date.month() < 4 {
        date.year() - 1
    } else {
        date.year()
    };
    let term = format!("{year}-{}", quarter_for_month(date.month()));
    let mut workspace = json!({
        "schemaVersion": 3,
        "deviceId": device_id,
        "selectedTerm": term,
        "terms": [term],
        "courses": [],
        "memberships": [],
        "assignments": [],
        "materials": [],
        "events": [],
        "planItems": [],
        "exceptions": [],
        "intensiveSchedules": [],
        "plans": [],
        "academicCalendar": [],
        "gpaSummary": [],
        "departments": [],
        "recommendedCourses": [],
        "importantDates": [],
        "exams": [],
        "scheduleOverrides": [],
        "subscriptions": [],
        "scheduledTasks": [reference_refresh_task(device_id), lms_background_refresh_task(device_id)],
        "settings": {
            "appearance": "system",
            "agentAccess": {"enabled": false, "lastWrites": []},
            "density": "comfortable",
            "locale": "en",
            "notifications": {
                "classes": true,
                "exams": true,
                "examLeadMinutes": 60,
                "assignments": true,
                "localPermission": "default"
            },
            "cloud": {
                "provider": "none",
                "clientId": "",
                "redirectUri": "http://127.0.0.1:42813/oauth/callback",
                "conflict": "resolve-manually"
            },
            "customization": {
                "background": "",
                "sites": {
                    "portal": {"css": "", "js": "", "enabled": false},
                    "lms": {"css": "", "js": "", "enabled": false},
                    "syllabus": {"css": "", "js": "", "enabled": false}
                },
                "hooks": []
            }
        },
        "academicProfile": {"name": "Personal program", "gradePath": [], "years": []},
        "importStatus": {"school": "not-configured"},
        "syncStatus": {
            "state": "needs-configuration",
            "message": "Cloud sync is off."
        }
    });
    workspace["currentTerm"] = Value::String(term);
    workspace["classSchedules"] = json!([]);
    merge_object(
        &mut workspace["settings"],
        json!({
            "restoreSchoolPage": false,
            "schoolWebZoom": 1,
            "showGrades": false,
            "siteConsents": {"portal": false, "lms": false, "syllabus": false},
            "subscriptionRefresh": {"frequency": "daily", "allowCellular": false}
        }),
    );
    for task in workspace["scheduledTasks"]
        .as_array_mut()
        .into_iter()
        .flatten()
    {
        task["updatedAt"] = json!(updated_at);
    }
    workspace
}

pub(super) fn reference_refresh_task(device_id: &str) -> Value {
    json!({
        "id": "builtin:reference-subscriptions",
        "rev": 1,
        "updatedAt": now(),
        "updatedBy": device_id,
        "name": "Reference subscriptions",
        "action": "reference-refresh",
        "schedule": "RRULE:FREQ=HOURLY;INTERVAL=1;BYMINUTE=0",
        "enabled": true
    })
}

pub(super) fn lms_background_refresh_task(device_id: &str) -> Value {
    json!({
        "id": "builtin:lms-background-refresh",
        "rev": 1,
        "updatedAt": now(),
        "updatedBy": device_id,
        "name": "LMS background refresh",
        "action": "lms-refresh",
        "schedule": "RRULE:FREQ=HOURLY;INTERVAL=4;BYMINUTE=0",
        "enabled": false
    })
}
