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
