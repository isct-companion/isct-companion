use super::*;

pub(super) fn settings_parts(settings: &Value) -> (Value, Value, Value) {
    let user = json!({
        "departmentId": settings.get("departmentId"),
        "notifications": {
            "classes": settings.pointer("/notifications/classes"),
            "exams": settings.pointer("/notifications/exams"),
            "examLeadMinutes": settings.pointer("/notifications/examLeadMinutes"),
            "assignments": settings.pointer("/notifications/assignments"),
            "classLeadMinutes": settings.pointer("/notifications/classLeadMinutes"),
            "assignmentLeadMinutes": settings.pointer("/notifications/assignmentLeadMinutes")
        },
        "customization": settings.get("customization")
    });
    let device = json!({
        "lmsBilingualFilter": settings.get("lmsBilingualFilter"),
        "autoDownloadLimitMb": settings.get("autoDownloadLimitMb"),
        "autoCleanDays": settings.get("autoCleanDays"),
        "agentAccess": settings.get("agentAccess"),
        "appearance": settings.get("appearance"),
        "density": settings.get("density"),
        "textSize": settings.get("textSize"),
        "automaticAppUpdates": settings.get("automaticAppUpdates"),
        "restoreSchoolPage": settings.get("restoreSchoolPage"),
        "activeSchoolSite": settings.get("activeSchoolSite"),
        "schoolWebZoom": settings.get("schoolWebZoom"),
        "showGrades": settings.get("showGrades"),
        "siteConsents": settings.get("siteConsents"),
        "locale": settings.get("locale"),
        "contentLanguage": settings.get("contentLanguage"),
        "subscriptionRefresh": settings.get("subscriptionRefresh"),
        "notifications": {"localPermission": settings.pointer("/notifications/localPermission")}
    });
    let cloud = settings.get("cloud").cloned().unwrap_or_else(|| json!({}));
    (user, device, cloud)
}

/// Enumerated preference values. Settings forms render their own option lists;
/// Core is the one owner of which values may be stored.
const PREFERENCE_CHOICES: &[(&str, &[&str])] = &[
    ("/appearance", &["system", "light", "dark"]),
    ("/density", &["comfortable", "compact"]),
    ("/textSize", &["small", "medium", "large"]),
    ("/locale", &["en", "ja", "zh"]),
    ("/contentLanguage", &["auto", "ja", "en"]),
    (
        "/notifications/localPermission",
        &["default", "denied", "granted"],
    ),
    ("/subscriptionRefresh/frequency", &["daily"]),
];
const CLOUD_CHOICES: &[(&str, &[&str])] = &[
    ("/provider", &["none", "google-drive", "onedrive", "icloud"]),
    (
        "/conflict",
        &["keep-current", "use-latest", "resolve-manually"],
    ),
];
/// Study-guide cohorts are four-digit years from the 2000 curriculum onwards.
pub(crate) const ADMISSION_YEARS: std::ops::RangeInclusive<i64> = 2000..=9999;

pub(super) fn validate_choices(
    value: &Value,
    choices: &[(&str, &[&str])],
) -> Result<(), DataError> {
    for (pointer, allowed) in choices {
        let Some(stored) = value.pointer(pointer).filter(|value| !value.is_null()) else {
            continue;
        };
        if !stored
            .as_str()
            .is_some_and(|stored| allowed.contains(&stored))
        {
            return Err(DataError::InvalidInput(format!(
                "setting {} must be one of {}",
                &pointer[1..],
                allowed.join(", ")
            )));
        }
    }
    Ok(())
}

/// Checks the values present in whole settings or in one stored part of them.
pub(super) fn validate_preference_values(settings: &Value) -> Result<(), DataError> {
    crate::schedule::assignment_lead_minutes(settings.get("notifications").unwrap_or(&Value::Null))
        .map_err(|error| DataError::InvalidInput(error.to_string()))?;

    if let Some(lead) = settings
        .pointer("/notifications/examLeadMinutes")
        .filter(|value| !value.is_null())
    {
        if !lead
            .as_u64()
            .is_some_and(|lead| (1..=10080).contains(&lead))
        {
            return Err(DataError::InvalidInput(
                "Exam reminder time must be between 1 minute and 7 days.".into(),
            ));
        }
    }
    validate_choices(settings, PREFERENCE_CHOICES)?;
    settings
        .get("cloud")
        .map_or(Ok(()), |cloud| validate_choices(cloud, CLOUD_CHOICES))
}

pub(super) fn validate_settings(settings: &Value) -> Result<(), DataError> {
    validate_contract::<crate::contracts::Settings>("settings", settings)?;
    validate_preference_values(settings)
}

pub(crate) fn validate_admission_year(profile: &Value) -> Result<(), DataError> {
    match profile
        .get("admissionYear")
        .filter(|value| !value.is_null())
    {
        None => Ok(()),
        Some(year)
            if year
                .as_i64()
                .is_some_and(|year| ADMISSION_YEARS.contains(&year)) =>
        {
            Ok(())
        }
        Some(_) => Err(DataError::InvalidInput(format!(
            "admission year must be between {} and {}",
            ADMISSION_YEARS.start(),
            ADMISSION_YEARS.end()
        ))),
    }
}

pub(super) fn validate_study_enrollments(profile: &Value) -> Result<(), DataError> {
    let Some(value) = profile.get("studyEnrollments") else {
        return Ok(());
    };
    let mut enrollments: Vec<crate::study_profile::StudyEnrollment> =
        serde_json::from_value(value.clone()).map_err(|error| {
            DataError::InvalidInput(format!("study stages are invalid: {error}"))
        })?;
    let original_terms = enrollments
        .iter()
        .map(|enrollment| enrollment.admitted_term.clone())
        .collect::<Vec<_>>();
    crate::study_profile::canonicalize_study_enrollments(&mut enrollments)
        .map_err(|error| DataError::InvalidInput(error.to_string()))?;
    if enrollments
        .iter()
        .map(|enrollment| &enrollment.admitted_term)
        .ne(original_terms.iter())
    {
        return Err(DataError::InvalidInput(
            "study stages must be ordered by admission term".into(),
        ));
    }
    Ok(())
}

/// Every writer of a settings row meets the same value rules.
pub(super) fn validate_setting(key: &str, value: &Value) -> Result<(), DataError> {
    match key {
        "userPreferences" | "devicePreferences" => validate_preference_values(value),
        "cloud" => validate_choices(value, CLOUD_CHOICES),
        "classGroups" => crate::schedule::validate_class_groups(value)
            .map_err(|error| DataError::InvalidInput(error.to_string())),
        "academicProfile" => {
            validate_admission_year(value)?;
            validate_study_enrollments(value)?;
            if let Some(periods) = value
                .get("teachingPeriods")
                .filter(|value| !value.is_null())
            {
                let periods: Vec<crate::schedule::TeachingPeriod> =
                    serde_json::from_value(periods.clone()).map_err(|error| {
                        DataError::InvalidInput(format!("teaching periods are invalid: {error}"))
                    })?;
                crate::schedule::validate_teaching_periods(&periods)
                    .map_err(|error| DataError::InvalidInput(error.to_string()))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Stored preference parts are complete snapshots of their fields: a null
/// field was cleared and removes any default or legacy value beneath it.
pub(super) fn apply_preferences(target: &mut Value, part: Value) {
    let (Some(target), Value::Object(part)) = (target.as_object_mut(), part) else {
        return;
    };
    for (key, value) in part {
        if value.is_null() {
            target.remove(&key);
        } else if value.is_object() && target.get(&key).is_some_and(Value::is_object) {
            if let Some(child) = target.get_mut(&key) {
                apply_preferences(child, value);
            }
        } else {
            target.insert(key, value);
        }
    }
}
