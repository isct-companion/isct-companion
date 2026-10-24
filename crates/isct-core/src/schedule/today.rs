//! Phone read models: what Today shows right now and the quarter's assignments
//! as rows. Core decides the phase, the hero and every list; which assignment
//! belongs to which group comes from the one task timeline (`tasks.rs`) that
//! desktop also renders. Shells render the fields in order and format
//! countdowns from the timestamps.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{DateTime, Duration, FixedOffset};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::calendar::{quarter_range, teaching_week};
use super::reminders::reminder_term;
use super::tasks::{countdown_tone, task_view, AssignmentGroups};
use super::{
    array, assignment_submitted, calendar_between, display_color, parse_civil_date,
    teaching_periods, text, tokyo_date, visible, Occurrence, OccurrenceKind, ScheduleError,
    TeachingPeriod,
};

/// Classes the Today stack previews: the next one and two after it.
const UPCOMING_CLASSES: usize = 3;
pub(crate) const RECENT_DAYS: i64 = 3;
const RECENT_ITEMS: usize = 3;
/// In class: the newest material and the nearest open assignment.
const CLASS_ITEMS: usize = 1;
/// How far ahead Today looks for the next class day.
const LOOKAHEAD_DAYS: i64 = 21;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum TodayPhase {
    /// Classes today, none started yet.
    BeforeClasses,
    /// Between two classes today.
    BetweenClasses,
    InClass,
    /// Today's classes are over.
    AfterClasses,
    /// No class today (weekend, holiday, break or an empty day).
    NoClasses,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TodayClass {
    pub occurrence: Occurrence,
    /// Teaching periods the class spans, when its times match the period table.
    pub periods: Option<[u8; 2]>,
    pub starts_at: String,
    pub ends_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DeadlineItem {
    pub assignment_id: String,
    pub course_id: Option<String>,
    pub course_title: Option<String>,
    pub color: Option<String>,
    pub title: String,
    pub due_at: Option<String>,
    pub overdue: bool,
    /// `past`, `urgent` (< 24 h), `soon` (< 72 h) or `normal`.
    pub tone: String,
    pub pinned: bool,
    pub submitted: bool,
    pub hidden: bool,
    pub grade: Option<String>,
    pub source_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MaterialItem {
    pub material_id: String,
    pub course_id: String,
    pub course_title: Option<String>,
    pub color: Option<String>,
    pub title: String,
    /// `pdf`, `link` or `video`.
    pub kind: String,
    pub section: Option<i64>,
    pub section_title: Option<String>,
    pub url: Option<String>,
    pub updated_at: Option<String>,
    pub added_today: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RecentItem {
    Grade(DeadlineItem),
    Material(MaterialItem),
}

/// Which list a `RecentItem` comes from, naming it to `MarkRecentSeen`.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum RecentKind {
    Grade,
    Material,
}

/// The `recentSeen` entry of one recent item.
pub(crate) fn recent_key(kind: RecentKind, id: &str) -> String {
    match kind {
        RecentKind::Grade => format!("grade:{id}"),
        RecentKind::Material => format!("material:{id}"),
    }
}

/// The `updatedAt` a recent item is shown for; seeing it records this value.
pub(crate) fn recent_version<'a>(
    workspace: &'a Value,
    kind: RecentKind,
    id: &str,
) -> Option<&'a str> {
    let field = match kind {
        RecentKind::Grade => "assignments",
        RecentKind::Material => "materials",
    };
    array(workspace, field)
        .iter()
        .find(|row| text(row, "id") == Some(id))
        .and_then(|row| text(row, "updatedAt"))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TodayView {
    pub date: String,
    pub term: String,
    /// Week of the quarter, counted from its first day.
    pub teaching_week: Option<u32>,
    pub no_classes: bool,
    pub calendar_notice: Option<String>,
    pub phase: TodayPhase,
    /// The class in progress.
    pub hero: Option<TodayClass>,
    /// The next classes after now and after the hero, today first and then
    /// the following class days, at most `UPCOMING_CLASSES`.
    pub upcoming: Vec<TodayClass>,
    /// In class: the current course's newest materials and open work.
    pub class_materials: Vec<MaterialItem>,
    pub class_assignments: Vec<DeadlineItem>,
    /// After classes: the nearest open deadline before midnight.
    pub deadline_hero: Option<DeadlineItem>,
    /// No classes: grades and materials that changed recently.
    pub recent: Vec<RecentItem>,
    /// When the phase or hero changes next; shells re-query then.
    pub next_boundary: Option<String>,
}

/// `AssignmentGroups` with each id resolved to its row.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DeadlineGroups {
    pub pinned: Vec<DeadlineItem>,
    pub due_this_week: Vec<DeadlineItem>,
    pub later: Vec<DeadlineItem>,
    pub undated: Vec<DeadlineItem>,
    pub past_recent: Vec<DeadlineItem>,
    pub past_earlier: Vec<DeadlineItem>,
    pub hidden: Vec<DeadlineItem>,
}

/// The quarter's assignment timeline as rows: `today` is what is left to do,
/// `record` every assignment whatever its state (`TermTaskView`).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AssignmentBoard {
    pub term: String,
    pub today: DeadlineGroups,
    pub record: DeadlineGroups,
}

fn jst() -> Result<FixedOffset, ScheduleError> {
    FixedOffset::east_opt(9 * 3600)
        .ok_or_else(|| ScheduleError::Compatibility("JST offset unavailable".into()))
}

fn at(date: &str, time: &str) -> Result<DateTime<FixedOffset>, ScheduleError> {
    DateTime::parse_from_rfc3339(&format!("{date}T{time}:00+09:00"))
        .map_err(|_| ScheduleError::Compatibility("schedule contains an invalid time".into()))
}

fn start_of_next_day(date: &str) -> Result<DateTime<FixedOffset>, ScheduleError> {
    let next = parse_civil_date(date)? + Duration::days(1);
    at(&next.to_string(), "00:00")
}

fn class(block: Occurrence, periods: &[TeachingPeriod]) -> Result<TodayClass, ScheduleError> {
    let starts_at = at(&block.date, &block.start_time)?;
    let ends_at = at(
        block.end_date.as_deref().unwrap_or(&block.date),
        &block.end_time,
    )?;
    let first = periods
        .iter()
        .find(|row| row.start_time == block.start_time)
        .map(|row| row.periods[0]);
    let last = periods
        .iter()
        .find(|row| row.end_time == block.end_time)
        .map(|row| row.periods[1]);
    Ok(TodayClass {
        periods: first.zip(last).map(|(a, b)| [a, b]),
        starts_at: starts_at.to_rfc3339(),
        ends_at: ends_at.to_rfc3339(),
        occurrence: block,
    })
}

/// Classes that count for Today: courses and exams that are not cancelled.
fn classes_on(agenda: &[Occurrence]) -> Vec<Occurrence> {
    agenda
        .iter()
        .filter(|block| {
            matches!(block.kind, OccurrenceKind::Course | OccurrenceKind::Exam)
                && block.cancelled != Some(true)
        })
        .cloned()
        .collect()
}

fn course<'a>(workspace: &'a Value, id: Option<&str>) -> Option<&'a Value> {
    let id = id?;
    array(workspace, "courses")
        .iter()
        .find(|row| visible(row) && text(row, "id") == Some(id))
}

fn deadline_item(
    workspace: &Value,
    row: &Value,
    now: DateTime<FixedOffset>,
) -> Option<DeadlineItem> {
    let id = text(row, "id")?;
    let course_id = text(row, "courseId");
    let course = course(workspace, course_id);
    let due = text(row, "dueAt").and_then(|raw| DateTime::parse_from_rfc3339(raw).ok());
    Some(DeadlineItem {
        assignment_id: id.into(),
        course_id: course_id.map(str::to_owned),
        course_title: course.and_then(|c| text(c, "title")).map(str::to_owned),
        color: course.map(display_color),
        title: text(row, "title").unwrap_or_default().into(),
        due_at: due.map(|due| due.to_rfc3339()),
        overdue: due.is_some_and(|due| due < now),
        tone: due.map_or("normal", |due| countdown_tone(due, now)).into(),
        pinned: row.get("pinned").and_then(Value::as_bool) == Some(true),
        submitted: assignment_submitted(row),
        hidden: row.get("hidden").and_then(Value::as_bool) == Some(true),
        grade: text(row, "grade")
            .filter(|grade| !grade.trim().is_empty())
            .map(str::to_owned),
        source_url: text(row, "sourceUrl").map(str::to_owned),
    })
}

fn material_item(workspace: &Value, row: &Value, date: &str) -> Option<MaterialItem> {
    let course_id = text(row, "courseId")?;
    let course = course(workspace, Some(course_id));
    let updated_at = text(row, "updatedAt");
    Some(MaterialItem {
        material_id: text(row, "id")?.into(),
        course_id: course_id.into(),
        course_title: course.and_then(|c| text(c, "title")).map(str::to_owned),
        color: course.map(display_color),
        title: text(row, "title").unwrap_or_default().into(),
        kind: text(row, "type").unwrap_or("link").into(),
        section: row
            .get("sectionNumber")
            .and_then(Value::as_i64)
            .or_else(|| row.get("session").and_then(Value::as_i64)),
        section_title: text(row, "sectionTitle").map(str::to_owned),
        url: text(row, "resourceUrl")
            .or_else(|| text(row, "url"))
            .map(str::to_owned),
        updated_at: updated_at.map(str::to_owned),
        added_today: updated_at
            .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
            .is_some_and(|at| tokyo_date(at) == date),
    })
}

/// Materials a student can open: visible LMS rows other than assignment and quiz modules.
fn materials(workspace: &Value) -> impl Iterator<Item = &Value> {
    array(workspace, "materials").iter().filter(|row| {
        visible(row)
            && row.get("visible").and_then(Value::as_bool) != Some(false)
            && !matches!(text(row, "module"), Some("assign" | "quiz"))
    })
}

fn deadline_groups(
    workspace: &Value,
    groups: &AssignmentGroups,
    now: DateTime<FixedOffset>,
) -> DeadlineGroups {
    let rows = |ids: &[String]| -> Vec<DeadlineItem> {
        ids.iter()
            .filter_map(|id| {
                array(workspace, "assignments")
                    .iter()
                    .find(|row| visible(row) && text(row, "id") == Some(id))
            })
            .filter_map(|row| deadline_item(workspace, row, now))
            .collect()
    };
    DeadlineGroups {
        pinned: rows(&groups.pinned_ids),
        due_this_week: rows(&groups.due_this_week_ids),
        later: rows(&groups.later_ids),
        undated: rows(&groups.undated_ids),
        past_recent: rows(&groups.past_recent_ids),
        past_earlier: rows(&groups.past_earlier_ids),
        hidden: rows(&groups.hidden_ids),
    }
}

/// What is left to do this quarter, by deadline: the timeline's `today` groups.
fn open_deadlines(
    workspace: &Value,
    term: &str,
    now: DateTime<FixedOffset>,
) -> Result<Vec<DeadlineItem>, ScheduleError> {
    let view = task_view(workspace, now)?;
    let Some(groups) = view.by_term.get(term) else {
        return Ok(Vec::new());
    };
    let groups = deadline_groups(workspace, &groups.today, now);
    let mut rows: Vec<_> = [
        groups.pinned,
        groups.due_this_week,
        groups.later,
        groups.past_recent,
    ]
    .into_iter()
    .flatten()
    .collect();
    // Dated rows by deadline, undated pinned rows last.
    rows.sort_by(|a, b| {
        a.due_at
            .is_none()
            .cmp(&b.due_at.is_none())
            .then_with(|| a.due_at.cmp(&b.due_at))
    });
    Ok(rows)
}

pub fn today_view(
    workspace: &Value,
    now: DateTime<FixedOffset>,
) -> Result<TodayView, ScheduleError> {
    let now = now.with_timezone(&jst()?);
    let date = tokyo_date(now);
    let term = reminder_term(workspace);
    let periods = teaching_periods(workspace)?;
    let lookahead = (parse_civil_date(&date)? + Duration::days(LOOKAHEAD_DAYS)).to_string();
    let days = calendar_between(workspace, &date, &lookahead)?;
    let (no_classes, calendar_notice, todays) = days
        .first()
        .map(|day| {
            (
                day.no_classes,
                day.calendar_notice.clone(),
                classes_on(&day.occurrences),
            )
        })
        .unwrap_or_default();
    let teaching_week = quarter_range(workspace, &term)
        .ok()
        .flatten()
        .and_then(|range| teaching_week(&range, parse_civil_date(&date).ok()?));

    let mut current = None;
    let mut upcoming = Vec::new();
    let mut started = false;
    for block in todays.iter().cloned() {
        let item = class(block, &periods)?;
        let starts = DateTime::parse_from_rfc3339(&item.starts_at)
            .map_err(|_| ScheduleError::Compatibility("invalid class start".into()))?;
        let ends = DateTime::parse_from_rfc3339(&item.ends_at)
            .map_err(|_| ScheduleError::Compatibility("invalid class end".into()))?;
        if starts <= now {
            started = true;
        }
        if starts <= now && now < ends && current.is_none() {
            current = Some(item);
        } else if starts > now {
            upcoming.push(item);
        }
    }
    let phase = if todays.is_empty() {
        TodayPhase::NoClasses
    } else if current.is_some() {
        TodayPhase::InClass
    } else if upcoming.is_empty() {
        TodayPhase::AfterClasses
    } else if started {
        TodayPhase::BetweenClasses
    } else {
        TodayPhase::BeforeClasses
    };
    // The stack runs on past today: the following class days fill it.
    for day in days.iter().skip(1) {
        if upcoming.len() >= UPCOMING_CLASSES {
            break;
        }
        for block in classes_on(&day.occurrences) {
            upcoming.push(class(block, &periods)?);
        }
    }
    upcoming.truncate(UPCOMING_CLASSES);
    let next_start = upcoming.first().map(|item| item.starts_at.clone());

    let midnight = start_of_next_day(&date)?;
    let open = open_deadlines(workspace, &term, now)?;
    let due_before = |item: &DeadlineItem, limit: DateTime<FixedOffset>| {
        item.due_at
            .as_deref()
            .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
            .is_some_and(|due| due < limit)
    };

    let mut view = TodayView {
        date: date.clone(),
        term,
        teaching_week,
        no_classes,
        calendar_notice,
        phase,
        hero: None,
        upcoming,
        class_materials: Vec::new(),
        class_assignments: Vec::new(),
        deadline_hero: None,
        recent: Vec::new(),
        next_boundary: None,
    };
    match phase {
        TodayPhase::BeforeClasses | TodayPhase::BetweenClasses => {
            view.next_boundary = next_start;
        }
        TodayPhase::InClass => {
            let hero = current.ok_or_else(|| {
                ScheduleError::Compatibility("in-class phase without a class".into())
            })?;
            let course_ids: Vec<String> = hero.occurrence.course_ids().map(str::to_owned).collect();
            let mut class_materials: Vec<_> = materials(workspace)
                .filter(|row| {
                    text(row, "courseId").is_some_and(|id| course_ids.iter().any(|own| own == id))
                })
                .filter_map(|row| material_item(workspace, row, &date))
                .collect();
            // Newest section first, then the most recently changed row.
            class_materials.sort_by(|a, b| {
                b.section
                    .cmp(&a.section)
                    .then_with(|| b.updated_at.cmp(&a.updated_at))
            });
            view.class_materials = class_materials.into_iter().take(CLASS_ITEMS).collect();
            view.class_assignments = open
                .into_iter()
                .filter(|item| {
                    item.course_id
                        .as_ref()
                        .is_some_and(|id| course_ids.contains(id))
                        && !item.overdue
                })
                .take(CLASS_ITEMS)
                .collect();
            view.next_boundary = Some(hero.ends_at.clone());
            view.hero = Some(hero);
        }
        TodayPhase::AfterClasses => {
            view.deadline_hero = open
                .into_iter()
                .find(|item| !item.overdue && due_before(item, midnight));
            view.next_boundary = Some(midnight.to_rfc3339());
        }
        TodayPhase::NoClasses => {
            view.recent = recent(workspace, now, &date);
            view.next_boundary = Some(midnight.to_rfc3339());
        }
    }
    Ok(view)
}

/// Grades and materials changed within the last few days, newest first.
/// One opened since its last change (`recentSeen`) is no longer new.
fn recent(workspace: &Value, now: DateTime<FixedOffset>, date: &str) -> Vec<RecentItem> {
    unseen(workspace, now, date)
        .into_iter()
        .take(RECENT_ITEMS)
        .map(|unseen| unseen.item)
        .collect()
}

/// Every item that is new now, past the few Today shows: read all marks
/// them, so the list does not refill from the ones it held back.
pub(crate) fn unseen_recent(
    workspace: &Value,
    now: DateTime<FixedOffset>,
) -> Vec<(String, String)> {
    unseen(workspace, now, &tokyo_date(now))
        .into_iter()
        .map(|unseen| (unseen.key, unseen.version))
        .collect()
}

/// A new recent item with its `recentSeen` entry: the key and the
/// `updatedAt` seeing it records.
struct Unseen {
    key: String,
    version: String,
    item: RecentItem,
}

fn unseen(workspace: &Value, now: DateTime<FixedOffset>, date: &str) -> Vec<Unseen> {
    let since = now - Duration::days(RECENT_DAYS);
    let seen = workspace.get("recentSeen");
    let changed = |kind: RecentKind, row: &Value| {
        let raw = text(row, "updatedAt")?;
        let key = recent_key(kind, text(row, "id")?);
        if seen.and_then(|seen| seen.get(&key)).and_then(Value::as_str) == Some(raw) {
            return None;
        }
        DateTime::parse_from_rfc3339(raw)
            .ok()
            .filter(|at| *at >= since && *at <= now)
    };
    let entry = |kind: RecentKind, row: &Value, item: RecentItem| {
        Some(Unseen {
            key: recent_key(kind, text(row, "id")?),
            version: text(row, "updatedAt")?.to_owned(),
            item,
        })
    };
    let mut items: Vec<(DateTime<FixedOffset>, Unseen)> = array(workspace, "assignments")
        .iter()
        .filter(|row| visible(row) && text(row, "submission") == Some("graded"))
        .filter_map(|row| {
            let at = changed(RecentKind::Grade, row)?;
            let item = deadline_item(workspace, row, now)?;
            if item.hidden || item.grade.is_none() {
                return None;
            }
            Some((at, entry(RecentKind::Grade, row, RecentItem::Grade(item))?))
        })
        .collect();
    items.extend(materials(workspace).filter_map(|row| {
        let at = changed(RecentKind::Material, row)?;
        let item = RecentItem::Material(material_item(workspace, row, date)?);
        Some((at, entry(RecentKind::Material, row, item)?))
    }));
    items.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    items.into_iter().map(|(_, unseen)| unseen).collect()
}

pub fn assignment_board(
    workspace: &Value,
    now: DateTime<FixedOffset>,
    term: Option<String>,
) -> Result<AssignmentBoard, ScheduleError> {
    let now = now.with_timezone(&jst()?);
    let term = term.unwrap_or_else(|| reminder_term(workspace));
    let view = task_view(workspace, now)?;
    let groups = view.by_term.get(&term).cloned().unwrap_or_default();
    Ok(AssignmentBoard {
        today: deadline_groups(workspace, &groups.today, now),
        record: deadline_groups(workspace, &groups.record, now),
        term,
    })
}
