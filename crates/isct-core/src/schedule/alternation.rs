//! 物理学演習 (LAS.P105/P106) and 物理学実験 (LAS.P107/P108) meet every other
//! week. A class is split into a red and a white group that take turns, and
//! the person picks their group once per semester.
//!
//! Which group meets on which date is not on the syllabus; Isct-Data
//! publishes it per syllabus offering, copied from the course's LMS schedule.
//! A class without a published schedule keeps its weekly blocks, marked as
//! alternating, rather than showing a guessed rotation.

use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::civil::Term;
use super::{array, course_terms, text, visible, ScheduleError};

fn alternating(course: &Value) -> bool {
    text(course, "code").is_some_and(|code| {
        ["LAS.P105", "LAS.P106", "LAS.P107", "LAS.P108"]
            .iter()
            .any(|prefix| code.starts_with(prefix))
    })
}

/// The two halves of a 物理学演習・実験 class, which meet in turns.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "lowercase")]
pub enum ClassGroup {
    Red,
    White,
}

/// Who meets on one date: one group, or both (orientation, exam).
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "lowercase")]
pub enum Turn {
    Red,
    White,
    All,
}

/// The alternating-class facts of one course block.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Alternation {
    /// Who meets on this date; absent on the weekly template and for classes
    /// without a published schedule.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub turn: Option<Turn>,
    /// Isct-Data publishes this class's sessions.
    pub scheduled: bool,
    /// The person's group for the semester, once chosen.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub group: Option<ClassGroup>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "ts",
        ts(
            optional,
            type = "\"orientation\" | \"exercise\" | \"lab\" | \"exam\" | \"no-class\""
        )
    )]
    pub session: Option<String>,
    /// The published session number, such as "3Q-1".
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub number: Option<String>,
}

/// How a weekly meeting of a course shows on one date.
pub(crate) enum Resolution {
    /// The weekly rules apply; `Some` marks an alternating class whose
    /// sessions are not published.
    Weekly(Option<Alternation>),
    Hide,
    Show {
        alternation: Alternation,
        cancelled: bool,
    },
}

/// "2026-1Q2Q" or "2026-3Q4Q": the two quarters a group choice covers.
fn semester_of(term: &str) -> Option<String> {
    let term = Term::parse(term).ok()?;
    Some(if term.quarter <= 2 {
        format!("{}-1Q2Q", term.year)
    } else {
        format!("{}-3Q4Q", term.year)
    })
}

/// The two quarter ids ("2026-3Q", "2026-4Q") of a semester key.
fn semester_terms(semester: &str) -> Option<[String; 2]> {
    let (year, quarters) = semester.split_once('-')?;
    if year.len() != 4 || !year.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    match quarters {
        "1Q2Q" => Some([format!("{year}-1Q"), format!("{year}-2Q")]),
        "3Q4Q" => Some([format!("{year}-3Q"), format!("{year}-4Q")]),
        _ => None,
    }
}

pub fn validate_semester(semester: &str) -> Result<(), ScheduleError> {
    semester_terms(semester)
        .map(|_| ())
        .ok_or_else(|| ScheduleError::InvalidInput("semester is invalid".into()))
}

/// `classGroups` maps semesters to "red" or "white".
pub fn validate_class_groups(value: &Value) -> Result<(), ScheduleError> {
    let groups = value
        .as_object()
        .ok_or_else(|| ScheduleError::InvalidInput("class groups are invalid".into()))?;
    for (semester, group) in groups {
        validate_semester(semester)?;
        if !matches!(group.as_str(), Some("red" | "white")) {
            return Err(ScheduleError::InvalidInput(
                "class groups are invalid".into(),
            ));
        }
    }
    Ok(())
}

fn chosen_group(workspace: &Value, semester: &str) -> Option<ClassGroup> {
    workspace
        .pointer(&format!("/classGroups/{semester}"))
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

/// The Isct-Data schedule published for a course's syllabus offering.
fn published_schedule<'a>(workspace: &'a Value, course: &Value) -> Option<&'a Value> {
    let offering = text(course, "offeringId")?;
    array(workspace, "classSchedules").iter().find(|row| {
        visible(row)
            && row
                .get("offeringIds")
                .and_then(Value::as_array)
                .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(offering)))
    })
}

fn sessions(schedule: &Value) -> impl Iterator<Item = &Value> {
    schedule
        .get("sessions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn turn_of(session: &Value) -> Option<Turn> {
    match (text(session, "kind")?, text(session, "group")) {
        ("orientation" | "exam", _) => Some(Turn::All),
        ("exercise" | "lab", Some("red")) => Some(Turn::Red),
        ("exercise" | "lab", Some("white")) => Some(Turn::White),
        _ => None,
    }
}

fn includes(turn: Turn, group: Option<ClassGroup>) -> bool {
    !matches!(
        (turn, group),
        (Turn::Red, Some(ClassGroup::White)) | (Turn::White, Some(ClassGroup::Red))
    )
}

/// How a weekly meeting of `course` shows on `date`. A published schedule
/// replaces the weekly rule: only its dates show, so an exam after the
/// teaching range still appears and an unlisted date does not.
pub(crate) fn resolve(
    workspace: &Value,
    course: &Value,
    term: &str,
    date: &str,
    include_cancelled: bool,
) -> Resolution {
    if !alternating(course) {
        return Resolution::Weekly(None);
    }
    let Some(semester) = semester_of(term) else {
        return Resolution::Weekly(None);
    };
    let group = chosen_group(workspace, &semester);
    let Some(schedule) = published_schedule(workspace, course) else {
        return Resolution::Weekly(Some(Alternation {
            turn: None,
            scheduled: false,
            group,
            session: None,
            number: None,
        }));
    };
    let Some(session) = sessions(schedule).find(|row| text(row, "date") == Some(date)) else {
        return Resolution::Hide;
    };
    let alternation = |turn| Alternation {
        turn: Some(turn),
        scheduled: true,
        group,
        session: text(session, "kind").map(str::to_owned),
        number: text(session, "number").map(str::to_owned),
    };
    match turn_of(session) {
        Some(turn) if includes(turn, group) => Resolution::Show {
            alternation: alternation(turn),
            cancelled: false,
        },
        None if include_cancelled && text(session, "kind") == Some("no-class") => {
            Resolution::Show {
                alternation: alternation(Turn::All),
                cancelled: true,
            }
        }
        _ => Resolution::Hide,
    }
}

/// The weekly template's marker for an alternating course.
pub(crate) fn template(workspace: &Value, course: &Value, term: &str) -> Option<Alternation> {
    if !alternating(course) {
        return None;
    }
    let semester = semester_of(term)?;
    Some(Alternation {
        turn: None,
        scheduled: published_schedule(workspace, course).is_some(),
        group: chosen_group(workspace, &semester),
        session: None,
        number: None,
    })
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ClassGroupCourse {
    pub id: String,
    pub title: String,
    pub scheduled: bool,
}

/// The first published one-group session from `today`, so the person can
/// recognise their group ("10/16 is the red group's exercise").
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ClassGroupSession {
    pub date: String,
    pub turn: Turn,
    pub course_title: String,
    #[cfg_attr(feature = "ts", ts(type = "\"exercise\" | \"lab\""))]
    pub session: String,
}

/// One semester with alternating classes, and the group chosen for it.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ClassGroupChoice {
    pub semester: String,
    /// The semester's two quarter ids, such as ["2026-3Q", "2026-4Q"].
    pub terms: Vec<String>,
    pub group: Option<ClassGroup>,
    /// No group is chosen yet for the current quarter's semester or a later
    /// one, so the person should be asked.
    pub needs_choice: bool,
    pub courses: Vec<ClassGroupCourse>,
    pub next: Option<ClassGroupSession>,
}

/// Every semester with an alternating course, newest first.
pub fn class_group_choices(workspace: &Value, today: &str) -> Vec<ClassGroupChoice> {
    let current = text(workspace, "currentTerm").and_then(semester_of);
    let mut choices: Vec<ClassGroupChoice> = Vec::new();
    for course in array(workspace, "courses")
        .iter()
        .filter(|row| visible(row) && alternating(row))
    {
        let title = course
            .pointer("/overrides/title")
            .and_then(Value::as_str)
            .or_else(|| text(course, "title"))
            .unwrap_or_default();
        let schedule = published_schedule(workspace, course);
        let next = schedule.and_then(|schedule| {
            sessions(schedule)
                .filter(|row| text(row, "date").is_some_and(|date| date >= today))
                .find_map(|row| {
                    Some(ClassGroupSession {
                        turn: turn_of(row).filter(|turn| *turn != Turn::All)?,
                        date: text(row, "date")?.into(),
                        course_title: title.into(),
                        session: text(row, "kind")?.into(),
                    })
                })
        });
        let mut semesters: Vec<String> = course_terms(workspace, course)
            .iter()
            .filter_map(|term| semester_of(term))
            .collect();
        semesters.dedup();
        for semester in semesters {
            let index = match choices.iter().position(|row| row.semester == semester) {
                Some(index) => index,
                None => {
                    let group = chosen_group(workspace, &semester);
                    let past = current.as_ref().is_some_and(|current| &semester < current);
                    choices.push(ClassGroupChoice {
                        terms: semester_terms(&semester).map(Vec::from).unwrap_or_default(),
                        needs_choice: group.is_none() && !past,
                        group,
                        semester,
                        courses: Vec::new(),
                        next: None,
                    });
                    choices.len() - 1
                }
            };
            let choice = &mut choices[index];
            choice.courses.push(ClassGroupCourse {
                id: text(course, "id").unwrap_or_default().into(),
                title: title.into(),
                scheduled: schedule.is_some(),
            });
            if next.as_ref().is_some_and(|next| {
                choice
                    .next
                    .as_ref()
                    .is_none_or(|current| next.date < current.date)
            }) {
                choice.next.clone_from(&next);
            }
        }
    }
    choices.sort_by(|a, b| b.semester.cmp(&a.semester));
    choices
}

#[cfg(test)]
#[path = "../tests/schedule__alternation.rs"]
mod tests;
