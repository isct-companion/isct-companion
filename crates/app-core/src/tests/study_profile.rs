use super::*;
use serde_json::json;

/// A workspace whose study history is regular first-grade entries.
fn history(entries: &[(&str, &str)]) -> Value {
    let enrollments: Vec<Value> = entries
        .iter()
        .map(|(stage, admitted_term)| {
            json!({"stage":stage, "entryKind":"regular", "entryGrade":1,
                   "admittedTerm":admitted_term})
        })
        .collect();
    json!({"academicProfile":{"studyEnrollments":enrollments}})
}

/// Each row is one decision the 2026 study guides fix for a study history, a
/// planned term and a course code. The undergraduate first-year rule is also
/// walked through the finder (`course_query`) and the catalog add path
/// (`workspace`); its rows here are only the level boundary.
#[test]
fn eligibility_follows_the_2026_guides_for_each_stage_and_term() {
    use super::CourseEligibilityReason as R;
    use super::CourseEligibilityStatus::{Allowed, Ineligible, Unknown};
    let admission_year_only = json!({"academicProfile":{"admissionYear":2026}});
    let shared_term = history(&[("undergraduate", "2026-1Q"), ("master", "2026-1Q")]);
    let undergraduate = history(&[("undergraduate", "2026-1Q")]);
    let then_master = history(&[("master", "2026-3Q"), ("undergraduate", "2026-1Q")]);
    let master = history(&[("master", "2026-1Q")]);
    let autumn_master = history(&[("master", "2026-3Q")]);
    let professional = history(&[("professional", "2026-1Q")]);
    let doctoral = history(&[("doctoral", "2026-1Q")]);
    let doctoral_2027 = history(&[("doctoral", "2027-1Q")]);
    #[rustfmt::skip]
    let cases = [
        // Identity Core cannot read stays unknown instead of being guessed:
        // an admission year alone is not an undergraduate.
        (&admission_year_only, "2026-3Q", "CSC.T201", Unknown, R::AcademicIdentityMissing),
        (&shared_term, "2026-2Q", "CSC.T201", Unknown, R::InvalidStudyHistory),
        (&undergraduate, "2026-3Q", "SPECIAL", Unknown, R::CourseCodeUnrecognized),
        (&undergraduate, "2026-3Q", "SPECIAL.2026", Unknown, R::CourseCodeUnrecognized),
        // Undergraduate first year: 200-level and up are closed, suffix or not.
        (&undergraduate, "2026-4Q", "CSC.T200", Ineligible, R::UndergraduateFirstYearUpperLevel),
        (&undergraduate, "2026-4Q", "CSC.T199-01", Allowed, R::NoKnownRestriction),
        // The stage that governs a term is the latest one admitted by then.
        (&then_master, "2026-2Q", "CSC.T201", Ineligible, R::UndergraduateFirstYearUpperLevel),
        (&then_master, "2026-3Q", "CSC.T201", Allowed, R::NoKnownRestriction),
        // Graduate stages are not held to the undergraduate rule.
        (&master, "2026-1Q", "LAH.T400", Allowed, R::NoKnownRestriction),
        (&master, "2026-1Q", "LAW.W599-01", Allowed, R::NoKnownRestriction),
        (&professional, "2026-1Q", "LAH.T500", Allowed, R::NoKnownRestriction),
        (&doctoral, "2026-1Q", "CSC.T600", Allowed, R::NoKnownRestriction),
        // Master humanities 500+ opens after the first semester, spring or autumn entry.
        (&master, "2026-2Q", "LAH.S501-01", Ineligible, R::MasterFirstSemesterHumanitiesUpperLevel),
        (&master, "2026-3Q", "LAH.S501-01", Allowed, R::NoKnownRestriction),
        (&autumn_master, "2026-4Q", "LAH.S501-01", Ineligible, R::MasterFirstSemesterHumanitiesUpperLevel),
        (&autumn_master, "2027-1Q", "LAH.S501-01", Allowed, R::NoKnownRestriction),
        // 600-level needs an approval Core cannot see; the humanities block comes first.
        (&master, "2026-3Q", "CSC.T601", Unknown, R::GraduateCourseApprovalRequired),
        (&professional, "2026-3Q", "CSC.T601", Unknown, R::GraduateCourseApprovalRequired),
        (&master, "2026-2Q", "CSC.T601", Unknown, R::GraduateCourseApprovalRequired),
        (&master, "2026-2Q", "LAH.A601", Ineligible, R::MasterFirstSemesterHumanitiesUpperLevel),
        // Leadership practica need a workshop grade, for master students only.
        (&master, "2026-3Q", "LAH.C402", Unknown, R::LeadershipWorkshopGradeRequired),
        (&master, "2026-3Q", "LAH.C501-01", Unknown, R::LeadershipWorkshopGradeRequired),
        (&professional, "2026-3Q", "LAH.C402", Allowed, R::NoKnownRestriction),
        // Doctoral entrepreneurship: principle, first-semester block, separate application.
        (&doctoral, "2026-2Q", "ENT.A621", Unknown, R::DoctoralFirstSemesterGenerallyRestricted),
        (&doctoral, "2026-2Q", "ENT.A631", Ineligible, R::DoctoralFirstSemesterRestricted),
        (&doctoral, "2026-2Q", "ENT.A635", Ineligible, R::DoctoralFirstSemesterRestricted),
        (&doctoral, "2026-2Q", "ENT.A639", Ineligible, R::DoctoralFirstSemesterRestricted),
        (&doctoral, "2026-2Q", "ENT.A641", Ineligible, R::DoctoralFirstSemesterRestricted),
        (&doctoral, "2026-2Q", "ENT.A642", Ineligible, R::DoctoralFirstSemesterRestricted),
        (&doctoral, "2026-2Q", "ENT.A643", Ineligible, R::DoctoralFirstSemesterRestricted),
        (&doctoral, "2026-3Q", "ENT.A635", Unknown, R::SeparateApplicationRequired),
        (&doctoral, "2026-3Q", "ENT.A639", Unknown, R::SeparateApplicationRequired),
        (&doctoral, "2026-3Q", "ENT.A631", Allowed, R::NoKnownRestriction),
        (&doctoral, "2026-2Q", "ENT.C620", Allowed, R::NoKnownRestriction),
        // A matching rule from a year no guide was read for is not applied.
        (&doctoral_2027, "2027-2Q", "ENT.A631", Unknown, R::RuleYearUnsupported),
    ];
    for (workspace, term, code, status, reason) in cases {
        let result = catalog_course_eligibility(workspace, term, &json!({"code":code}));
        assert_eq!(
            (result.status, result.reason),
            (status, reason),
            "{term} {code}"
        );
    }
}
