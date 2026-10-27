//! 物理学演習 (LAS.P105/P106) and 物理学実験 (LAS.P107/P108) meet every other
//! week. A class is split into a red and a white group that take turns, and
//! the person picks their group once per semester.
//!
//! Which group meets on which date is not on the syllabus; Isct-Data
//! publishes it per syllabus offering, copied from the course's LMS schedule.
//! A class without a published schedule keeps its weekly blocks, marked as
//! alternating, rather than showing a guessed rotation.
//!
//! 演習 and 実験 of the same class share their slot by design: while one
//! group has the exercise the other has the lab. Wherever both land on one
//! slot they become one block for both courses rather than a clash, which
//! needs no published schedule.

use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::civil::Term;
use super::occurrence::{overlap, Occurrence, OccurrenceKind};
use super::{array, course_terms, text, visible, ScheduleError};

#[derive(PartialEq)]
enum Half {
    Exercise,
    Lab,
}

/// Which half of which 演習・実験 pair a course code belongs to: 演習第一
/// (P105) pairs with 実験第一 (P107), 第二 (P106) with P108.
fn half(code: &str) -> Option<(Half, u8)> {
    [
        ("LAS.P105", Half::Exercise, 1),
        ("LAS.P106", Half::Exercise, 2),
        ("LAS.P107", Half::Lab, 1),
        ("LAS.P108", Half::Lab, 2),
    ]
    .into_iter()
    .find(|(prefix, _, _)| code.starts_with(prefix))
    .map(|(_, half, number)| (half, number))
}

fn alternating(course: &Value) -> bool {
    text(course, "code").and_then(half).is_some()
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
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
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
    /// The 演習 and 実験 this block stands for, exercise first, when both meet
    /// in its slot. The block's title, code and room join the two; a shell
    /// may split the block between them.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub pair: Option<Vec<PairedCourse>>,
}

/// One course of a shared 演習・実験 block.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PairedCourse {
    pub course_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub code: Option<String>,
    pub room: String,
    pub color: String,
    /// This course's own meeting, which may be shorter than the shared block.
    pub start_time: String,
    pub end_time: String,
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
            pair: None,
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
        pair: None,
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
        pair: None,
    })
}

/// Whether `a` and `b` are 演習 and 実験 of one class, and if so whether `a`
/// is the exercise.
fn exercise_first(a: &Occurrence, b: &Occurrence) -> Option<bool> {
    let class = |block: &Occurrence| {
        (block.kind == OccurrenceKind::Course
            && block.cancelled != Some(true)
            && block
                .alternation
                .as_ref()
                .is_some_and(|row| row.pair.is_none()))
        .then(|| block.subtitle.as_deref().and_then(half))
        .flatten()
    };
    match (class(a)?, class(b)?) {
        ((Half::Exercise, x), (Half::Lab, y)) if x == y => Some(true),
        ((Half::Lab, x), (Half::Exercise, y)) if x == y => Some(false),
        _ => None,
    }
}

/// 演習 and 実験 of one class that meet in turns: they never clash.
pub(crate) fn alternate(a: &Occurrence, b: &Occurrence) -> bool {
    exercise_first(a, b).is_some()
}

fn paired_course(block: &Occurrence) -> PairedCourse {
    PairedCourse {
        course_id: block.course_id.clone().unwrap_or_default(),
        title: block.title.clone(),
        code: block.subtitle.clone(),
        room: block.room.clone(),
        color: block.color.clone(),
        start_time: block.start_time.clone(),
        end_time: block.end_time.clone(),
    }
}

/// Folds every 演習 block and the overlapping 実験 block of the same class
/// into one block for both, keyed by the exercise.
pub(crate) fn pair_blocks(blocks: &mut Vec<Occurrence>) -> Result<(), ScheduleError> {
    let mut i = 0;
    while i < blocks.len() {
        let mut partner = None;
        for j in i + 1..blocks.len() {
            if alternate(&blocks[i], &blocks[j]) && overlap(&blocks[i], &blocks[j])? {
                partner = Some(j);
                break;
            }
        }
        if let Some(j) = partner {
            let other = blocks.remove(j);
            let block = &mut blocks[i];
            let (exercise, lab) = if exercise_first(block, &other) == Some(true) {
                (block.clone(), other)
            } else {
                (other, block.clone())
            };
            let join = |a: &str, b: &str| match (a, b) {
                ("", b) => b.to_owned(),
                (a, "") => a.to_owned(),
                (a, b) if a == b => a.to_owned(),
                (a, b) => format!("{a} / {b}"),
            };
            let (own, theirs) = (
                exercise.alternation.clone().unwrap_or_default(),
                lab.alternation.clone().unwrap_or_default(),
            );
            let same_turn = own.turn == theirs.turn;
            *block = Occurrence {
                title: join(&exercise.title, &lab.title),
                subtitle: Some(join(
                    exercise.subtitle.as_deref().unwrap_or_default(),
                    lab.subtitle.as_deref().unwrap_or_default(),
                )),
                room: join(&exercise.room, &lab.room),
                start_time: exercise.start_time.clone().min(lab.start_time.clone()),
                end_time: exercise.end_time.clone().max(lab.end_time.clone()),
                alternation: Some(Alternation {
                    scheduled: own.scheduled && theirs.scheduled,
                    turn: own.turn.filter(|_| same_turn),
                    number: own.number.clone().filter(|_| same_turn),
                    session: own
                        .session
                        .clone()
                        .filter(|_| own.session == theirs.session),
                    pair: Some(vec![paired_course(&exercise), paired_course(&lab)]),
                    ..own
                }),
                ..exercise
            };
        }
        i += 1;
    }
    Ok(())
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
