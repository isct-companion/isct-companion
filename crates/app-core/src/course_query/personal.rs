use super::{FinderCatalog, FinderError, FinderRowView, IndexedCourse, Recommendation};
use crate::{
    schedule::{array, text_or_empty as text},
    study_profile::{CourseEligibility, CourseEligibilityStatus},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
#[cfg(feature = "ts")]
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
pub struct BusySlot {
    pub day: u8,
    pub from: u8,
    pub to: u8,
}

pub(super) fn weekly(course: &Value) -> impl Iterator<Item = &Value> {
    array(course, "schedule").iter().filter(|s| {
        text(s, "kind") == "weekly"
            && s.get("day").and_then(Value::as_u64).is_some_and(|v| v > 0)
            && array(s, "periods").len() == 2
    })
}
pub(super) fn overlap(slot: &Value, from: u64, to: u64) -> bool {
    let p = array(slot, "periods");
    matches!((p.first().and_then(Value::as_u64),p.get(1).and_then(Value::as_u64)),(Some(a),Some(b)) if a<=to && from<=b)
}
pub fn total_credits(course: &Value) -> f64 {
    ["lecture", "exercise", "lab"]
        .iter()
        .map(|key| {
            course
                .pointer(&format!("/credits/{key}"))
                .and_then(Value::as_f64)
                .unwrap_or(0.)
        })
        .sum()
}
pub fn clashes(course: &Value, busy: &[BusySlot]) -> bool {
    weekly(course).any(|s| {
        busy.iter().any(|b| {
            s.get("day").and_then(Value::as_u64) == Some(u64::from(b.day))
                && overlap(s, u64::from(b.from), u64::from(b.to))
        })
    })
}
pub fn clashing_courses(course: &Value, placed: &[Value]) -> Vec<Value> {
    placed
        .iter()
        .filter(|p| {
            text(p, "code") != text(course, "code")
                && array(p, "meetings").iter().any(|m| {
                    let periods: Vec<_> = array(m, "periods")
                        .iter()
                        .filter_map(Value::as_u64)
                        .collect();
                    match (periods.iter().min(), periods.iter().max()) {
                        (Some(from), Some(to)) => weekly(course).any(|s| {
                            array(m, "days").iter().any(|d| Some(d) == s.get("day"))
                                && overlap(s, *from, *to)
                        }),
                        _ => false,
                    }
                })
        })
        .cloned()
        .collect()
}

/// What a search reads from the workspace for one term and catalog,
/// whatever is typed: recommendations, the timetable, and each row's
/// eligibility, clashes and replacements.
pub struct FinderPersonal {
    pub(super) current_quarter: u8,
    pub(super) recommended: BTreeMap<String, Recommendation>,
    pub(super) busy: Vec<BusySlot>,
    eligibility: BTreeMap<String, CourseEligibility>,
    pub(super) ineligible: usize,
    /// Per catalog row: whether a default search lists it.
    pub(super) eligible: Vec<bool>,
    replacements: BTreeMap<String, Vec<String>>,
    clashes: HashSet<String>,
    pub(super) on_timetable: Vec<String>,
}
impl FinderPersonal {
    pub fn new(
        workspace: &Value,
        catalog: &FinderCatalog,
        year: u16,
        term: &str,
    ) -> Result<Self, FinderError> {
        let planned = crate::study_profile::PlannedTerm::new(workspace, term);
        // 教務 marks each row for the signed-in student's own affiliation.
        let recommended = catalog
            .guide
            .iter()
            .filter(|r| r.year == Some(u64::from(year)))
            .filter(|r| {
                planned.eligibility_of_code(r.code.as_deref()).status
                    != CourseEligibilityStatus::Ineligible
            })
            .map(|r| {
                (
                    r.code.clone().unwrap_or_default(),
                    Recommendation {
                        group: r.group.clone(),
                        mark: r.mark.clone(),
                    },
                )
            })
            .collect();
        let placed = crate::schedule::scheduled_courses(workspace, term);
        let busy: Vec<BusySlot> = placed
            .iter()
            .flat_map(|c| array(c, "meetings"))
            .flat_map(|m| {
                let periods: Vec<_> = array(m, "periods")
                    .iter()
                    .filter_map(Value::as_u64)
                    .collect();
                let (from, to) = match (periods.iter().min(), periods.iter().max()) {
                    (Some(a), Some(b)) => (*a as u8, *b as u8),
                    _ => return Vec::new(),
                };
                array(m, "days")
                    .iter()
                    .filter_map(|d| {
                        d.as_u64().map(|day| BusySlot {
                            day: day as u8,
                            from,
                            to,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        let courses = || catalog.courses.iter().map(|c| &c.course);
        let eligibility = courses()
            .map(|course| (text(course, "id").to_string(), planned.eligibility(course)))
            .collect::<BTreeMap<_, _>>();
        let ineligible = eligibility
            .iter()
            .filter(|(_, result)| result.status == CourseEligibilityStatus::Ineligible)
            .map(|(id, _)| id.as_str())
            .collect::<HashSet<_>>();
        let eligible = courses()
            .map(|course| !ineligible.contains(text(course, "id")))
            .collect();
        let ineligible = ineligible.len();
        let replaceable: Vec<_> = placed
            .iter()
            .filter(|course| {
                array(workspace, "planItems").iter().any(|p| {
                    text(p, "courseId") == text(course, "id")
                        && text(p, "termId") == term
                        && p.get("deletedAt").is_none_or(Value::is_null)
                })
            })
            .cloned()
            .collect();
        let replacements = courses()
            .map(|c| {
                (
                    text(c, "id").to_string(),
                    clashing_courses(c, &replaceable)
                        .iter()
                        .map(|v| text(v, "id").to_string())
                        .collect(),
                )
            })
            .collect();
        let clashes = courses()
            .filter(|c| clashes(c, &busy))
            .map(|c| text(c, "id").to_string())
            .collect();
        let on_timetable = placed.iter().map(|c| text(c, "code").to_string()).collect();
        Ok(Self {
            current_quarter: text(workspace, "currentTerm")
                .chars()
                .rev()
                .nth(1)
                .and_then(|c| c.to_digit(10))
                .unwrap_or(1) as u8,
            recommended,
            busy,
            eligibility,
            ineligible,
            eligible,
            replacements,
            clashes,
            on_timetable,
        })
    }
    pub(super) fn ineligible(&self, id: &str) -> bool {
        self.eligibility
            .get(id)
            .is_some_and(|result| result.status == CourseEligibilityStatus::Ineligible)
    }
    /// One result as its row shows it.
    pub(super) fn row(&self, c: &IndexedCourse, matched: Vec<String>) -> FinderRowView {
        let id = text(&c.course, "id");
        let code = text(&c.course, "code");
        FinderRowView {
            course: c.course.clone(),
            matched,
            placed: self.on_timetable.iter().any(|placed| placed == code),
            clash: self.clashes.contains(id),
            replaces: self.replacements.get(id).cloned().unwrap_or_default(),
            credits: total_credits(&c.course),
            eligibility: self
                .eligibility
                .get(id)
                .filter(|result| result.status != CourseEligibilityStatus::Allowed)
                .cloned(),
            recommended: self.recommended.get(code).cloned(),
        }
    }
}
