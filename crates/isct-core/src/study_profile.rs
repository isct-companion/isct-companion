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

/// Study-guide cohorts are four-digit years from the 2000 curriculum onwards.
pub(crate) const ADMISSION_YEARS: std::ops::RangeInclusive<i64> = 2000..=9999;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum StudyStage {
    Undergraduate,
    Master,
    Professional,
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
    MasterFirstSemesterHumanitiesUpperLevel,
    GraduateCourseApprovalRequired,
    LeadershipWorkshopGradeRequired,
    DoctoralFirstSemesterGenerallyRestricted,
    DoctoralFirstSemesterRestricted,
    SeparateApplicationRequired,
    RuleYearUnsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum CourseEligibilityRuleId {
    UndergraduateFirstYearCourseLevel,
    MasterFirstSemesterHumanities,
    GraduateDoctoralLevelApproval,
    MasterLeadershipPracticumPrerequisite,
    DoctoralAcademicLiteracyFirstSemester,
    DoctoralAcademicFirstSemester,
    DoctoralAcademicTrainingApplication,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CourseEligibilityRuleEvidence {
    pub id: CourseEligibilityRuleId,
    pub academic_year: i32,
    pub source_url: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CourseEligibility {
    pub status: CourseEligibilityStatus,
    pub reason: CourseEligibilityReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub rule: Option<CourseEligibilityRuleEvidence>,
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
            rule: None,
        }
    }

    fn unknown(reason: CourseEligibilityReason) -> Self {
        Self {
            status: CourseEligibilityStatus::Unknown,
            reason,
            rule: None,
        }
    }

    fn ruled(
        status: CourseEligibilityStatus,
        reason: CourseEligibilityReason,
        rule: EligibilityRule,
    ) -> Self {
        Self {
            status,
            reason,
            rule: Some(rule.evidence()),
        }
    }
}

const UNDERGRADUATE_GUIDE: &str = "https://www.titech.ac.jp/guide/guide_2026/gakubu1/pdf/01.pdf";
const GRADUATE_HUMANITIES_GUIDE: &str =
    "https://www.titech.ac.jp/guide/guide_2026/graduate/pdf/03-01.pdf";
const GRADUATE_ENTREPRENEURSHIP_GUIDE: &str =
    "https://www.titech.ac.jp/guide/guide_2026/graduate/pdf/03-06.pdf";
const GRADUATE_GENERAL_GUIDE: &str =
    "https://www.titech.ac.jp/guide/guide_2026/graduate/pdf/01-01.pdf";

use CourseEligibilityRuleId as EligibilityRule;

impl CourseEligibilityRuleId {
    fn evidence(self) -> CourseEligibilityRuleEvidence {
        let source_url = match self {
            Self::UndergraduateFirstYearCourseLevel => UNDERGRADUATE_GUIDE,
            Self::MasterFirstSemesterHumanities | Self::MasterLeadershipPracticumPrerequisite => {
                GRADUATE_HUMANITIES_GUIDE
            }
            Self::GraduateDoctoralLevelApproval => GRADUATE_GENERAL_GUIDE,
            Self::DoctoralAcademicLiteracyFirstSemester
            | Self::DoctoralAcademicFirstSemester
            | Self::DoctoralAcademicTrainingApplication => GRADUATE_ENTREPRENEURSHIP_GUIDE,
        };
        CourseEligibilityRuleEvidence {
            id: self,
            academic_year: 2026,
            source_url: source_url.to_string(),
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

struct CourseCode {
    base: String,
    unit: String,
    level: u16,
}

fn course_code(code: &str) -> Option<CourseCode> {
    static COURSE_CODE: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(r"(?i)^([a-z]{3})\.([a-z])([1-9][0-9]{2})(?:-[0-9]{2})?$").ok()
    });
    let captures = COURSE_CODE.as_ref()?.captures(code.trim())?;
    let unit = captures.get(1)?.as_str().to_ascii_uppercase();
    let field = captures.get(2)?.as_str().to_ascii_uppercase();
    let number = captures.get(3)?.as_str();
    Some(CourseCode {
        base: format!("{unit}.{field}{number}"),
        unit,
        level: number.parse().ok()?,
    })
}

fn first_semester(admitted_term: (i32, u8), planned_term: (i32, u8)) -> bool {
    if admitted_term.1 <= 2 {
        planned_term.0 == admitted_term.0 && planned_term.1 <= 2
    } else {
        planned_term.0 == admitted_term.0 && planned_term.1 >= 3
    }
}

fn supported_rule_year(actual: i32, rule: EligibilityRule) -> Option<CourseEligibility> {
    (actual != rule.evidence().academic_year).then(|| {
        CourseEligibility::ruled(
            CourseEligibilityStatus::Unknown,
            CourseEligibilityReason::RuleYearUnsupported,
            rule,
        )
    })
}

fn undergraduate_eligibility(
    enrollment: &StudyEnrollment,
    admitted_term: (i32, u8),
    planned_term: (i32, u8),
    code: &CourseCode,
) -> CourseEligibility {
    match enrollment.entry_kind {
        StudyEntryKind::Transfer => CourseEligibility::allowed(),
        StudyEntryKind::Unknown => {
            CourseEligibility::unknown(CourseEligibilityReason::EntryDetailsMissing)
        }
        StudyEntryKind::Regular => {
            let Some(grade) = enrollment.entry_grade else {
                return CourseEligibility::unknown(CourseEligibilityReason::EntryDetailsMissing);
            };
            if grade != 1 || planned_term.0 != admitted_term.0 || code.level < 200 {
                return CourseEligibility::allowed();
            }
            let rule = EligibilityRule::UndergraduateFirstYearCourseLevel;
            if let Some(unsupported) = supported_rule_year(admitted_term.0, rule) {
                return unsupported;
            }
            CourseEligibility::ruled(
                CourseEligibilityStatus::Ineligible,
                CourseEligibilityReason::UndergraduateFirstYearUpperLevel,
                rule,
            )
        }
    }
}

fn master_eligibility(
    admitted_term: (i32, u8),
    planned_term: (i32, u8),
    code: &CourseCode,
) -> CourseEligibility {
    if code.unit == "LAH" && code.level >= 500 && first_semester(admitted_term, planned_term) {
        let rule = EligibilityRule::MasterFirstSemesterHumanities;
        if let Some(unsupported) = supported_rule_year(admitted_term.0, rule) {
            return unsupported;
        }
        return CourseEligibility::ruled(
            CourseEligibilityStatus::Ineligible,
            CourseEligibilityReason::MasterFirstSemesterHumanitiesUpperLevel,
            rule,
        );
    }
    if code.level >= 600 {
        let rule = EligibilityRule::GraduateDoctoralLevelApproval;
        if let Some(unsupported) = supported_rule_year(planned_term.0, rule) {
            return unsupported;
        }
        return CourseEligibility::ruled(
            CourseEligibilityStatus::Unknown,
            CourseEligibilityReason::GraduateCourseApprovalRequired,
            rule,
        );
    }
    if matches!(code.base.as_str(), "LAH.C402" | "LAH.C501") {
        let rule = EligibilityRule::MasterLeadershipPracticumPrerequisite;
        if let Some(unsupported) = supported_rule_year(planned_term.0, rule) {
            return unsupported;
        }
        return CourseEligibility::ruled(
            CourseEligibilityStatus::Unknown,
            CourseEligibilityReason::LeadershipWorkshopGradeRequired,
            rule,
        );
    }
    CourseEligibility::allowed()
}

fn professional_eligibility(planned_term: (i32, u8), code: &CourseCode) -> CourseEligibility {
    if code.level < 600 {
        return CourseEligibility::allowed();
    }
    let rule = EligibilityRule::GraduateDoctoralLevelApproval;
    if let Some(unsupported) = supported_rule_year(planned_term.0, rule) {
        return unsupported;
    }
    CourseEligibility::ruled(
        CourseEligibilityStatus::Unknown,
        CourseEligibilityReason::GraduateCourseApprovalRequired,
        rule,
    )
}

fn doctoral_eligibility(
    admitted_term: (i32, u8),
    planned_term: (i32, u8),
    code: &CourseCode,
) -> CourseEligibility {
    let first_semester_rule = match code.base.as_str() {
        "ENT.A621" => Some((
            EligibilityRule::DoctoralAcademicLiteracyFirstSemester,
            CourseEligibilityStatus::Unknown,
            CourseEligibilityReason::DoctoralFirstSemesterGenerallyRestricted,
        )),
        "ENT.A631" | "ENT.A635" | "ENT.A639" | "ENT.A641" | "ENT.A642" | "ENT.A643" => Some((
            EligibilityRule::DoctoralAcademicFirstSemester,
            CourseEligibilityStatus::Ineligible,
            CourseEligibilityReason::DoctoralFirstSemesterRestricted,
        )),
        _ => None,
    };
    if first_semester(admitted_term, planned_term) {
        if let Some((rule, status, reason)) = first_semester_rule {
            if let Some(unsupported) = supported_rule_year(admitted_term.0, rule) {
                return unsupported;
            }
            return CourseEligibility::ruled(status, reason, rule);
        }
    }
    if matches!(code.base.as_str(), "ENT.A635" | "ENT.A639") {
        let rule = EligibilityRule::DoctoralAcademicTrainingApplication;
        if let Some(unsupported) = supported_rule_year(planned_term.0, rule) {
            return unsupported;
        }
        return CourseEligibility::ruled(
            CourseEligibilityStatus::Unknown,
            CourseEligibilityReason::SeparateApplicationRequired,
            rule,
        );
    }
    CourseEligibility::allowed()
}

/// Decide whether a catalog offering may be newly planned in `term_id`.
///
/// `Allowed` means no implemented, evidence-backed rule restricts this
/// identity and course. It is not an official enrollment approval. Rules are
/// finite, source-linked, and limited to the academic year stated by the
/// source. Missing facts and approvals return `Unknown` and do not hard-block
/// planning.
pub fn catalog_course_eligibility(
    workspace: &Value,
    term_id: &str,
    offering: &Value,
) -> CourseEligibility {
    PlannedTerm::new(workspace, term_id).eligibility(offering)
}

/// The study enrollment that governs one planned term, read once from the
/// workspace so a whole catalog can be judged against it.
pub(crate) struct PlannedTerm {
    /// The governing enrollment, or the result every offering gets when the
    /// history cannot decide.
    basis: Result<Basis, CourseEligibility>,
}

struct Basis {
    enrollment: StudyEnrollment,
    admitted_term: (i32, u8),
    planned_term: (i32, u8),
}

impl PlannedTerm {
    pub(crate) fn new(workspace: &Value, term_id: &str) -> Self {
        Self {
            basis: Self::basis(workspace, term_id),
        }
    }

    fn basis(workspace: &Value, term_id: &str) -> Result<Basis, CourseEligibility> {
        let Some(planned_term) = term_key(term_id) else {
            return Err(CourseEligibility::unknown(
                CourseEligibilityReason::EnrollmentForTermMissing,
            ));
        };
        let enrollments = history(workspace).map_err(CourseEligibility::unknown)?;
        let Some(enrollment) = enrollments.into_iter().rev().find(|enrollment| {
            term_key(&enrollment.admitted_term).is_some_and(|term| term <= planned_term)
        }) else {
            return Err(CourseEligibility::unknown(
                CourseEligibilityReason::EnrollmentForTermMissing,
            ));
        };
        let Some(admitted_term) = term_key(&enrollment.admitted_term) else {
            return Err(CourseEligibility::unknown(
                CourseEligibilityReason::InvalidStudyHistory,
            ));
        };
        Ok(Basis {
            enrollment,
            admitted_term,
            planned_term,
        })
    }

    pub(crate) fn eligibility(&self, offering: &Value) -> CourseEligibility {
        self.eligibility_of_code(offering.get("code").and_then(Value::as_str))
    }

    /// The same decision from the offering's `code` field alone.
    pub(crate) fn eligibility_of_code(&self, code: Option<&str>) -> CourseEligibility {
        let Basis {
            enrollment,
            admitted_term,
            planned_term,
        } = match &self.basis {
            Ok(basis) => basis,
            Err(result) => return result.clone(),
        };
        let (admitted_term, planned_term) = (*admitted_term, *planned_term);
        let Some(code) = code.and_then(course_code) else {
            return CourseEligibility::unknown(CourseEligibilityReason::CourseCodeUnrecognized);
        };
        match enrollment.stage {
            StudyStage::Unknown => {
                CourseEligibility::unknown(CourseEligibilityReason::AcademicIdentityMissing)
            }
            StudyStage::Undergraduate => {
                undergraduate_eligibility(enrollment, admitted_term, planned_term, &code)
            }
            StudyStage::Master => master_eligibility(admitted_term, planned_term, &code),
            StudyStage::Professional => professional_eligibility(planned_term, &code),
            StudyStage::Doctoral => doctoral_eligibility(admitted_term, planned_term, &code),
        }
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
        CourseEligibilityReason::MasterFirstSemesterHumanitiesUpperLevel => {
            format!("{code} is not available during the first semester of a master's program.")
        }
        CourseEligibilityReason::DoctoralFirstSemesterRestricted => {
            format!("{code} is not available during the first six months of a doctoral program.")
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
        CourseEligibilityRuleId::decl(),
        CourseEligibilityRuleEvidence::decl(),
        CourseEligibility::decl(),
    ]
}

#[cfg(test)]
#[path = "tests/study_profile.rs"]
mod tests;
