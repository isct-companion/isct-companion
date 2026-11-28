use super::{parse_civil_date, ScheduleError, TeachingPeriod};
use serde_json::{Map, Value};
use std::collections::HashSet;

pub(crate) fn validate_academic_profile(profile: &Value) -> Result<Value, ScheduleError> {
    let input = profile
        .as_object()
        .ok_or_else(|| ScheduleError::InvalidInput("Academic profile must be an object".into()))?;
    let name = input
        .get("name")
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| ScheduleError::InvalidInput("Academic profile needs a name".into()))?
        .trim();
    if input.get("admissionYear").is_some_and(|v| {
        !v.is_null()
            && v.as_i64()
                .is_none_or(|y| !crate::study_profile::ADMISSION_YEARS.contains(&y))
    }) {
        return Err(ScheduleError::InvalidInput(
            "Academic profile has an invalid admission year".into(),
        ));
    }
    let grade_path = required_array(
        input,
        "gradePath",
        "Academic profile needs a grade path array",
    )?;
    let mut grades = HashSet::new();
    let mut normalized_grades = Vec::new();
    for (index, row) in grade_path.iter().enumerate() {
        let object = row.as_object().ok_or_else(|| {
            ScheduleError::InvalidInput(format!("Grade path entry {} must be an object", index + 1))
        })?;
        let grade = object
            .get("grade")
            .and_then(Value::as_str)
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| {
                ScheduleError::InvalidInput(format!(
                    "Grade path entry {} needs a grade label",
                    index + 1
                ))
            })?
            .trim();
        let year = object
            .get("academicYear")
            .and_then(Value::as_i64)
            .filter(|v| *v >= 1900)
            .ok_or_else(|| {
                ScheduleError::InvalidInput(format!(
                    "Grade path entry {} has an invalid academic year",
                    index + 1
                ))
            })?;
        if !grades.insert(grade.to_owned()) {
            return Err(ScheduleError::InvalidInput(
                "Grade labels must be unique".into(),
            ));
        }
        normalized_grades.push(serde_json::json!({"grade":grade,"academicYear":year}));
    }
    let years = required_array(
        input,
        "years",
        "Academic profile needs a configured years array",
    )?;
    let mut configured = HashSet::new();
    let mut normalized_years = Vec::new();
    for (index, row) in years.iter().enumerate() {
        let object = row.as_object().ok_or_else(|| {
            ScheduleError::InvalidInput(format!("Configured year {} must be an object", index + 1))
        })?;
        let year = object
            .get("academicYear")
            .and_then(Value::as_i64)
            .filter(|v| *v >= 1900)
            .ok_or_else(|| {
                ScheduleError::InvalidInput(format!(
                    "Configured year {} has an invalid academic year",
                    index + 1
                ))
            })?;
        if !configured.insert(year) {
            return Err(ScheduleError::InvalidInput(
                "Configured academic years must be unique".into(),
            ));
        }
        let starts = object
            .get("quarterStarts")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                ScheduleError::InvalidInput(format!(
                    "Configured year {} needs quarter starts",
                    index + 1
                ))
            })?;
        let values: Vec<_> = ["1Q", "2Q", "3Q", "4Q"]
            .iter()
            .map(|q| starts.get(*q).and_then(Value::as_str))
            .collect();
        if values
            .iter()
            .any(|v| v.is_none_or(|v| parse_civil_date(v).is_err()))
        {
            return Err(ScheduleError::InvalidInput(format!(
                "Configured year {} needs valid starts for 1Q–4Q",
                index + 1
            )));
        }
        if values.windows(2).any(|w| w[1] <= w[0]) {
            return Err(ScheduleError::InvalidInput(format!(
                "Configured year {} quarter starts must be chronological",
                index + 1
            )));
        }
        // Like the published calendars: a year's quarters start within that
        // academic year, so quarters of different years never overlap.
        let (first, next) = (format!("{year}-04-01"), format!("{}-04-01", year + 1));
        if values
            .iter()
            .flatten()
            .any(|v| *v < first.as_str() || *v >= next.as_str())
        {
            return Err(ScheduleError::InvalidInput(format!(
                "Configured year {} quarter starts must fall within academic year {year}",
                index + 1
            )));
        }
        normalized_years.push(serde_json::json!({"academicYear":year,"quarterStarts":starts}));
    }
    normalized_years.sort_by_key(|r| r.get("academicYear").and_then(Value::as_i64));
    let mut output = Map::new();
    output.insert("name".into(), Value::String(name.into()));
    if let Some(year) = input.get("admissionYear").filter(|v| !v.is_null()) {
        output.insert("admissionYear".into(), year.clone());
    }
    output.insert("gradePath".into(), Value::Array(normalized_grades));
    output.insert("years".into(), Value::Array(normalized_years));
    let periods: Option<Vec<TeachingPeriod>> =
        serde_json::from_value(input.get("teachingPeriods").cloned().unwrap_or(Value::Null))
            .map_err(|error| ScheduleError::InvalidInput(error.to_string()))?;
    if let Some(table) = &periods {
        super::validate_teaching_periods(table)?;
    }
    output.insert(
        "teachingPeriods".into(),
        serde_json::to_value(periods)
            .map_err(|error| ScheduleError::Compatibility(error.to_string()))?,
    );
    let mut enrollments: Vec<crate::study_profile::StudyEnrollment> = serde_json::from_value(
        input
            .get("studyEnrollments")
            .filter(|value| !value.is_null())
            .cloned()
            .unwrap_or_else(|| Value::Array(vec![])),
    )
    .map_err(|error| ScheduleError::InvalidInput(error.to_string()))?;
    crate::study_profile::canonicalize_study_enrollments(&mut enrollments)
        .map_err(|error| ScheduleError::InvalidInput(error.to_string()))?;
    output.insert(
        "studyEnrollments".into(),
        serde_json::to_value(enrollments)
            .map_err(|error| ScheduleError::Compatibility(error.to_string()))?,
    );
    Ok(Value::Object(output))
}
fn required_array<'a>(
    input: &'a Map<String, Value>,
    key: &str,
    message: &str,
) -> Result<&'a Vec<Value>, ScheduleError> {
    input
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| ScheduleError::InvalidInput(message.into()))
}
