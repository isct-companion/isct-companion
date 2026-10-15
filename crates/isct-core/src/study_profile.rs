//! Study identity and the enrollment rules that depend on it.
//!
//! `academicProfile.admissionYear` remains the cohort used to select study
//! guides.  It is deliberately not used to infer a degree stage: old profiles
//! without `studyEnrollments` therefore stay unknown instead of becoming
//! undergraduate profiles by accident.

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::LazyLock;
#[cfg(feature = "ts")]
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum StudyStage {
    Undergraduate,
    Master,
    Doctoral,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum StudyEntryKind {
    Regular,
    Transfer,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StudyEnrollment {
    #[serde(default)]
    pub stage: StudyStage,
    #[serde(default)]
    pub entry_kind: StudyEntryKind,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub entry_grade: Option<u8>,
    #[cfg_attr(feature = "ts", ts(type = "`${number}-${Quarter}`"))]
    pub admitted_term: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum CourseEligibilityStatus {
    Allowed,
    Ineligible,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum CourseEligibilityReason {
    NoKnownRestriction,
    AcademicIdentityMissing,
    EnrollmentForTermMissing,
    InvalidStudyHistory,
    EntryDetailsMissing,
    CourseCodeUnrecognized,
    UndergraduateFirstYearUpperLevel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CourseEligibility {
    pub status: CourseEligibilityStatus,
    pub reason: CourseEligibilityReason,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum StudyProfileError {
    #[error("study-stage admission term must look like 2026-1Q, got {term}")]
    InvalidAdmissionTerm { term: String },
    #[error("study-stage entry grade must be between 1 and 9")]
    InvalidEntryGrade,
    #[error("study stages cannot share admission term {term}")]
    DuplicateAdmissionTerm { term: String },
}

impl CourseEligibility {
    fn allowed() -> Self {
        Self {
            status: CourseEligibilityStatus::Allowed,
            reason: CourseEligibilityReason::NoKnownRestriction,
        }
    }

    fn unknown(reason: CourseEligibilityReason) -> Self {
        Self {
            status: CourseEligibilityStatus::Unknown,
            reason,
        }
    }
}

/// A sortable `YYYY-nQ` key. Invalid terms stay outside eligibility decisions.
fn term_key(term: &str) -> Option<(i32, u8)> {
    let (year, quarter) = term.split_once('-')?;
    if year.len() != 4 || !year.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let year = year
        .parse()
        .ok()
        .filter(|year| (1000..=9999).contains(year))?;
    let quarter = match quarter {
        "1Q" => 1,
        "2Q" => 2,
        "3Q" => 3,
        "4Q" => 4,
        _ => return None,
    };
    Some((year, quarter))
}

/// Validate and canonicalize an enrollment history before it is persisted.
pub fn canonicalize_study_enrollments(
    enrollments: &mut [StudyEnrollment],
) -> Result<(), StudyProfileError> {
    for enrollment in enrollments.iter() {
        if term_key(&enrollment.admitted_term).is_none() {
            return Err(StudyProfileError::InvalidAdmissionTerm {
                term: enrollment.admitted_term.clone(),
            });
        }
        if enrollment
            .entry_grade
            .is_some_and(|grade| grade == 0 || grade > 9)
        {
            return Err(StudyProfileError::InvalidEntryGrade);
        }
    }
    enrollments.sort_by_key(|enrollment| term_key(&enrollment.admitted_term));
    let mut terms = HashSet::new();
    if let Some(duplicate) = enrollments
        .iter()
        .filter_map(|enrollment| {
            term_key(&enrollment.admitted_term).map(|key| (key, enrollment.admitted_term.as_str()))
        })
        .find(|(key, _)| !terms.insert(*key))
    {
        return Err(StudyProfileError::DuplicateAdmissionTerm {
            term: duplicate.1.to_string(),
        });
    }
    Ok(())
}

fn history(workspace: &Value) -> Result<Vec<StudyEnrollment>, CourseEligibilityReason> {
    let Some(value) = workspace.pointer("/academicProfile/studyEnrollments") else {
        return Err(CourseEligibilityReason::AcademicIdentityMissing);
    };
    let mut enrollments: Vec<StudyEnrollment> = serde_json::from_value(value.clone())
        .map_err(|_| CourseEligibilityReason::InvalidStudyHistory)?;
    if enrollments.is_empty() {
        return Err(CourseEligibilityReason::AcademicIdentityMissing);
    }
    canonicalize_study_enrollments(&mut enrollments)
        .map_err(|_| CourseEligibilityReason::InvalidStudyHistory)?;
    Ok(enrollments)
}

fn course_level(code: &str) -> Option<u16> {
    static COURSE_CODE: LazyLock<Option<Regex>> =
        LazyLock::new(|| Regex::new(r"(?i)^[a-z]{3}\.[a-z]([1-9][0-9]{2})(?:-[0-9]{2})?$").ok());
    COURSE_CODE
        .as_ref()?
        .captures(code.trim())?
        .get(1)?
        .as_str()
        .parse()
        .ok()
}

/// Decide whether a catalog offering may be newly planned in `term_id`.
///
/// `Allowed` means no implemented, evidence-backed rule restricts this
/// identity and course. It is not an official enrollment approval. Only an
/// explicit first-year, regular undergraduate identity can trigger the known
/// 200-level restriction. Missing or malformed facts return `Unknown` and do
/// not hard-block planning.
pub fn catalog_course_eligibility(
    workspace: &Value,
    term_id: &str,
    offering: &Value,
) -> CourseEligibility {
    let Some(planned_term) = term_key(term_id) else {
        return CourseEligibility::unknown(CourseEligibilityReason::EnrollmentForTermMissing);
    };
    let enrollments = match history(workspace) {
        Ok(enrollments) => enrollments,
        Err(reason) => return CourseEligibility::unknown(reason),
    };
    let Some(enrollment) = enrollments.iter().rev().find(|enrollment| {
        term_key(&enrollment.admitted_term).is_some_and(|term| term <= planned_term)
    }) else {
        return CourseEligibility::unknown(CourseEligibilityReason::EnrollmentForTermMissing);
    };

    match enrollment.stage {
        StudyStage::Unknown => {
            CourseEligibility::unknown(CourseEligibilityReason::AcademicIdentityMissing)
        }
        StudyStage::Master | StudyStage::Doctoral => CourseEligibility::allowed(),
        StudyStage::Undergraduate => match enrollment.entry_kind {
            StudyEntryKind::Transfer => CourseEligibility::allowed(),
            StudyEntryKind::Unknown => {
                CourseEligibility::unknown(CourseEligibilityReason::EntryDetailsMissing)
            }
            StudyEntryKind::Regular => {
                let Some(grade) = enrollment.entry_grade else {
                    return CourseEligibility::unknown(
                        CourseEligibilityReason::EntryDetailsMissing,
                    );
                };
                let Some(admitted_term) = term_key(&enrollment.admitted_term) else {
                    return CourseEligibility::unknown(
                        CourseEligibilityReason::InvalidStudyHistory,
                    );
                };
                if grade != 1 || planned_term.0 != admitted_term.0 {
                    return CourseEligibility::allowed();
                }
                let Some(level) = offering
                    .get("code")
                    .and_then(Value::as_str)
                    .and_then(course_level)
                else {
                    return CourseEligibility::unknown(
                        CourseEligibilityReason::CourseCodeUnrecognized,
                    );
                };
                if level >= 200 {
                    CourseEligibility {
                        status: CourseEligibilityStatus::Ineligible,
                        reason: CourseEligibilityReason::UndergraduateFirstYearUpperLevel,
                    }
                } else {
                    CourseEligibility::allowed()
                }
            }
        },
    }
}

pub fn ineligibility_message(offering: &Value, eligibility: CourseEligibility) -> Option<String> {
    if eligibility.status != CourseEligibilityStatus::Ineligible {
        return None;
    }
    let code = offering
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("This course");
    Some(match eligibility.reason {
        CourseEligibilityReason::UndergraduateFirstYearUpperLevel => {
            format!("{code} is not available to a regular first-year undergraduate student.")
        }
        _ => format!("{code} is not available for this study profile."),
    })
}

#[cfg(feature = "ts")]
pub fn declarations() -> Vec<String> {
    vec![
        StudyStage::decl(),
        StudyEntryKind::decl(),
        StudyEnrollment::decl(),
        CourseEligibilityStatus::decl(),
        CourseEligibilityReason::decl(),
        CourseEligibility::decl(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn offering(code: &str) -> Value {
        json!({"code":code})
    }

    #[test]
    fn old_profiles_stay_unknown_instead_of_becoming_undergraduates() {
        let result = catalog_course_eligibility(
            &json!({"academicProfile":{"admissionYear":2026}}),
            "2026-3Q",
            &offering("CSC.T201"),
        );
        assert_eq!(result.status, CourseEligibilityStatus::Unknown);
        assert_eq!(
            result.reason,
            CourseEligibilityReason::AcademicIdentityMissing
        );
    }

    #[test]
    fn regular_first_year_rule_parses_class_suffixes() {
        let workspace = json!({"academicProfile":{"studyEnrollments":[{
            "stage":"undergraduate", "entryKind":"regular", "entryGrade":1,
            "admittedTerm":"2026-1Q"
        }]}});
        for code in ["CSC.T200", "CSC.T299-01", "LAE.E201-12"] {
            assert_eq!(
                catalog_course_eligibility(&workspace, "2026-4Q", &offering(code)).status,
                CourseEligibilityStatus::Ineligible,
                "{code}"
            );
        }
        assert_eq!(
            catalog_course_eligibility(&workspace, "2026-4Q", &offering("CSC.T199-01")).status,
            CourseEligibilityStatus::Allowed
        );
    }

    #[test]
    fn historical_stage_changes_at_the_admission_quarter() {
        let workspace = json!({"academicProfile":{"studyEnrollments":[
            {"stage":"master","entryKind":"regular","entryGrade":1,"admittedTerm":"2030-3Q"},
            {"stage":"undergraduate","entryKind":"regular","entryGrade":1,"admittedTerm":"2030-1Q"}
        ]}});
        assert_eq!(
            catalog_course_eligibility(&workspace, "2030-2Q", &offering("CSC.T201")).status,
            CourseEligibilityStatus::Ineligible
        );
        assert_eq!(
            catalog_course_eligibility(&workspace, "2030-3Q", &offering("CSC.T201")).status,
            CourseEligibilityStatus::Allowed
        );
    }

    #[test]
    fn graduate_stages_are_not_blocked_by_the_undergraduate_rule() {
        for (stage, code) in [
            ("master", "LAH.T400"),
            ("master", "LAW.W599-01"),
            ("doctoral", "CSC.T600"),
        ] {
            let workspace = json!({"academicProfile":{"studyEnrollments":[{
                "stage":stage,"entryKind":"regular","entryGrade":1,"admittedTerm":"2026-1Q"
            }]}});
            assert_eq!(
                catalog_course_eligibility(&workspace, "2026-1Q", &offering(code)).status,
                CourseEligibilityStatus::Allowed,
                "{stage} {code}"
            );
        }
    }

    #[test]
    fn duplicate_stage_terms_make_the_decision_unknown() {
        let workspace = json!({"academicProfile":{"studyEnrollments":[
            {"stage":"undergraduate","entryKind":"regular","entryGrade":1,"admittedTerm":"2026-1Q"},
            {"stage":"master","entryKind":"regular","entryGrade":1,"admittedTerm":"2026-1Q"}
        ]}});
        let result = catalog_course_eligibility(&workspace, "2026-2Q", &offering("CSC.T201"));
        assert_eq!(result.status, CourseEligibilityStatus::Unknown);
        assert_eq!(result.reason, CourseEligibilityReason::InvalidStudyHistory);
    }
}
