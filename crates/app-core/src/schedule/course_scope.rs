use super::{array, text, visible};
use serde_json::Value;
use std::collections::HashMap;

/// Live memberships and plan items: the rows that place a course on the
/// timetable of their quarter.
pub(super) fn placements(workspace: &Value) -> impl Iterator<Item = &Value> {
    array(workspace, "memberships")
        .iter()
        .chain(array(workspace, "planItems"))
        .filter(|row| visible(row))
}

fn visible_courses(workspace: &Value) -> impl Iterator<Item = &Value> {
    array(workspace, "courses")
        .iter()
        .filter(|row| visible(row))
}

fn saved_term(course: &Value) -> Option<&str> {
    // A catalog copy's term records where it was first saved. The plan,
    // membership or independent LMS record owns its current affiliation.
    if course.pointer("/source/provider").and_then(Value::as_str) == Some("kyomu") {
        return None;
    }
    text(course, "termId")
}

#[derive(Eq, Hash, PartialEq)]
enum CourseKey<'a> {
    Local(&'a str),
    Lms(u64),
    Offering(&'a str),
}

fn lms_keys(id: Option<u64>, offering: Option<&str>) -> impl Iterator<Item = CourseKey<'_>> {
    id.map(CourseKey::Lms).into_iter().chain(
        offering
            .filter(|id| !id.is_empty())
            .map(CourseKey::Offering),
    )
}

struct ActiveTerms<'a>(HashMap<CourseKey<'a>, Vec<String>>);

impl<'a> ActiveTerms<'a> {
    fn new(workspace: &'a Value) -> Self {
        let mut terms = HashMap::<_, Vec<_>>::new();
        for row in placements(workspace).filter(|row| text(row, "status") != Some("withdrawn")) {
            if let (Some(id), Some(term)) = (text(row, "courseId"), text(row, "termId")) {
                terms
                    .entry(CourseKey::Local(id))
                    .or_default()
                    .push(term.into());
            }
        }
        for row in array(workspace, "lmsCourses")
            .iter()
            .filter(|row| visible(row) && row["hidden"] != true)
        {
            let quarters = crate::data::lms_terms(row);
            for key in lms_keys(row["lmsId"].as_u64(), text(row, "offeringId")) {
                terms
                    .entry(key)
                    .or_default()
                    .extend(quarters.iter().cloned());
            }
        }
        Self(terms)
    }

    fn of<'b>(&'b self, course: &'b Value) -> impl Iterator<Item = &'b str> {
        text(course, "id")
            .map(CourseKey::Local)
            .into_iter()
            .chain(lms_keys(
                course["lmsCourseId"].as_u64(),
                text(course, "offeringId"),
            ))
            .filter_map(|key| self.0.get(&key))
            .flatten()
            .map(String::as_str)
    }

    fn including_saved<'b>(&'b self, course: &'b Value) -> impl Iterator<Item = &'b str> {
        saved_term(course).into_iter().chain(self.of(course))
    }
}

pub(super) fn lecture_courses<'a>(workspace: &'a Value, term: Option<&str>) -> Vec<&'a Value> {
    let active = ActiveTerms::new(workspace);
    visible_courses(workspace)
        .filter(|course| {
            active
                .of(course)
                .any(|quarter| term.is_none_or(|term| term == quarter))
        })
        .collect()
}

/// Every quarter a course belongs to, including its active placements and LMS quarters.
pub fn course_terms(workspace: &Value, course: &Value) -> Vec<String> {
    let active = ActiveTerms::new(workspace);
    let mut terms: Vec<_> = active.including_saved(course).map(str::to_owned).collect();
    terms.sort_unstable();
    terms.dedup();
    terms
}

/// The quarter's courses, including independently saved courses off the timetable.
pub fn term_courses<'a>(workspace: &'a Value, term: &str) -> Vec<&'a Value> {
    let active = ActiveTerms::new(workspace);
    visible_courses(workspace)
        .filter(|course| active.including_saved(course).any(|t| t == term))
        .collect()
}

pub(super) fn has_courses(workspace: &Value) -> bool {
    let active = ActiveTerms::new(workspace);
    visible_courses(workspace).any(|course| active.including_saved(course).next().is_some())
}

/// Quarters with placements or independently saved courses.
pub(crate) fn placement_terms(workspace: &Value) -> impl Iterator<Item = &str> {
    visible_courses(workspace)
        .filter_map(saved_term)
        .chain(placements(workspace).filter_map(|row| text(row, "termId")))
}
