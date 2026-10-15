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

    let Some(admitted_term) = term_key(&enrollment.admitted_term) else {
        return CourseEligibility::unknown(CourseEligibilityReason::InvalidStudyHistory);
    };
    let Some(code) = offering
        .get("code")
        .and_then(Value::as_str)
        .and_then(course_code)
    else {
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
mod tests {
    use super::*;
    use serde_json::json;

    fn offering(code: &str) -> Value {
        json!({"code":code})
    }

    fn enrolled(stage: &str, admitted_term: &str) -> Value {
        json!({"academicProfile":{"studyEnrollments":[{
            "stage":stage, "entryKind":"regular", "entryGrade":1,
            "admittedTerm":admitted_term
        }]}})
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
            {"stage":"master","entryKind":"regular","entryGrade":1,"admittedTerm":"2026-3Q"},
            {"stage":"undergraduate","entryKind":"regular","entryGrade":1,"admittedTerm":"2026-1Q"}
        ]}});
        assert_eq!(
            catalog_course_eligibility(&workspace, "2026-2Q", &offering("CSC.T201")).status,
            CourseEligibilityStatus::Ineligible
        );
        assert_eq!(
            catalog_course_eligibility(&workspace, "2026-3Q", &offering("CSC.T201")).status,
            CourseEligibilityStatus::Allowed
        );
    }

    #[test]
    fn graduate_stages_are_not_blocked_by_the_undergraduate_rule() {
        for (stage, code) in [
            ("master", "LAH.T400"),
            ("master", "LAW.W599-01"),
            ("professional", "LAH.T500"),
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
    fn master_humanities_500_opens_after_the_first_semester_for_spring_and_autumn_entry() {
        for (admitted, blocked, open) in [
            ("2026-1Q", "2026-2Q", "2026-3Q"),
            ("2026-3Q", "2026-4Q", "2027-1Q"),
        ] {
            let workspace = enrolled("master", admitted);
            let blocked = catalog_course_eligibility(&workspace, blocked, &offering("LAH.S501-01"));
            assert_eq!(blocked.status, CourseEligibilityStatus::Ineligible);
            assert_eq!(
                blocked.reason,
                CourseEligibilityReason::MasterFirstSemesterHumanitiesUpperLevel
            );
            assert_eq!(
                blocked.rule.as_ref().map(|rule| rule.academic_year),
                Some(2026)
            );
            assert_eq!(
                catalog_course_eligibility(&workspace, open, &offering("LAH.S501-01")).status,
                CourseEligibilityStatus::Allowed
            );
        }
    }

    #[test]
    fn graduate_600_level_requires_unknown_course_approval() {
        for stage in ["master", "professional"] {
            let result = catalog_course_eligibility(
                &enrolled(stage, "2026-1Q"),
                "2026-3Q",
                &offering("CSC.T601"),
            );
            assert_eq!(result.status, CourseEligibilityStatus::Unknown);
            assert_eq!(
                result.reason,
                CourseEligibilityReason::GraduateCourseApprovalRequired
            );
        }
        let master = enrolled("master", "2026-1Q");
        let humanities = catalog_course_eligibility(&master, "2026-2Q", &offering("LAH.A601"));
        assert_eq!(humanities.status, CourseEligibilityStatus::Ineligible);
        assert_eq!(
            humanities.reason,
            CourseEligibilityReason::MasterFirstSemesterHumanitiesUpperLevel
        );
        assert_eq!(
            catalog_course_eligibility(&master, "2026-2Q", &offering("CSC.T601")).reason,
            CourseEligibilityReason::GraduateCourseApprovalRequired
        );
    }

    #[test]
    fn leadership_practica_need_an_authoritative_workshop_grade() {
        let workspace = enrolled("master", "2026-1Q");
        for code in ["LAH.C402", "LAH.C501-01"] {
            let result = catalog_course_eligibility(&workspace, "2026-3Q", &offering(code));
            assert_eq!(result.status, CourseEligibilityStatus::Unknown, "{code}");
            assert_eq!(
                result.reason,
                CourseEligibilityReason::LeadershipWorkshopGradeRequired,
                "{code}"
            );
        }
        assert_eq!(
            catalog_course_eligibility(
                &enrolled("professional", "2026-1Q"),
                "2026-3Q",
                &offering("LAH.C402")
            )
            .status,
            CourseEligibilityStatus::Allowed
        );
    }

    #[test]
    fn doctoral_entrepreneurship_rules_distinguish_principle_block_and_application() {
        let workspace = enrolled("doctoral", "2026-1Q");
        let principle = catalog_course_eligibility(&workspace, "2026-2Q", &offering("ENT.A621"));
        assert_eq!(principle.status, CourseEligibilityStatus::Unknown);
        assert_eq!(
            principle.reason,
            CourseEligibilityReason::DoctoralFirstSemesterGenerallyRestricted
        );
        for code in [
            "ENT.A631", "ENT.A635", "ENT.A639", "ENT.A641", "ENT.A642", "ENT.A643",
        ] {
            let result = catalog_course_eligibility(&workspace, "2026-2Q", &offering(code));
            assert_eq!(result.status, CourseEligibilityStatus::Ineligible, "{code}");
            assert_eq!(
                result.reason,
                CourseEligibilityReason::DoctoralFirstSemesterRestricted,
                "{code}"
            );
        }
        for code in ["ENT.A635", "ENT.A639"] {
            let result = catalog_course_eligibility(&workspace, "2026-3Q", &offering(code));
            assert_eq!(result.status, CourseEligibilityStatus::Unknown, "{code}");
            assert_eq!(
                result.reason,
                CourseEligibilityReason::SeparateApplicationRequired,
                "{code}"
            );
        }
        assert_eq!(
            catalog_course_eligibility(&workspace, "2026-3Q", &offering("ENT.A631")).status,
            CourseEligibilityStatus::Allowed
        );
        assert_eq!(
            catalog_course_eligibility(&workspace, "2026-2Q", &offering("ENT.C620")).status,
            CourseEligibilityStatus::Allowed
        );
    }

    #[test]
    fn matching_restrictions_from_an_unverified_year_are_unknown() {
        let result = catalog_course_eligibility(
            &enrolled("doctoral", "2027-1Q"),
            "2027-2Q",
            &offering("ENT.A631"),
        );
        assert_eq!(result.status, CourseEligibilityStatus::Unknown);
        assert_eq!(result.reason, CourseEligibilityReason::RuleYearUnsupported);
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
