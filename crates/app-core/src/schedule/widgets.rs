//! The display-only contract shared by WidgetKit and Glance.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{
    array,
    calendar::academic_year_of,
    calendar_between, content_language, parse_civil_date, parse_timestamp,
    tasks::assignment_boundaries,
    teaching_periods,
    today::{at, class_moment, open_assignment_groups, timed_classes, LOOKAHEAD_DAYS},
    tokyo_date, ScheduleError, SchoolReads, TodayClass, TodayPhase,
};
use crate::{
    lms_ingest::{current_lms_term, lms_course_term, stored_course},
    reference_refresh::{reference_key, FileScope},
    sync_state::{expires_at, ttl, Key, Term},
    workspace_changes::WorkspaceChange,
};
use chrono::{DateTime, Duration, FixedOffset, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const SCHEMA_VERSION: u32 = 1;
const COVERAGE_DAYS: i64 = 7;
const SCHOOL_TIME_ZONE: &str = "Asia/Tokyo";

pub(crate) const READS: &[WorkspaceChange] = &[
    WorkspaceChange::Courses,
    WorkspaceChange::Registrations,
    WorkspaceChange::Calendar,
    WorkspaceChange::Exams,
    WorkspaceChange::Assignments,
    WorkspaceChange::Events,
    WorkspaceChange::Freshness,
    WorkspaceChange::Settings,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WidgetKind {
    Courses,
    Assignments,
}

impl WidgetKind {
    pub(crate) const ALL: [Self; 2] = [Self::Courses, Self::Assignments];
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WidgetDestination {
    Course { key: String, date: String },
    Assignment { id: String },
    Timetable,
    Assignments,
    Sync,
}

impl WidgetDestination {
    pub fn from_url(url: &url::Url) -> Option<Self> {
        if url.scheme() != "isctcompanion" || url.host_str() != Some("widget") {
            return None;
        }
        let param = |name| {
            url.query_pairs()
                .find(|(key, value)| key == name && !value.is_empty())
                .map(|(_, value)| value.into_owned())
        };
        Some(match url.path() {
            "/course" => Self::Course {
                key: param("key")?,
                date: param("date")?,
            },
            "/assignment" => Self::Assignment { id: param("id")? },
            "/timetable" => Self::Timetable,
            "/assignments" => Self::Assignments,
            "/sync" => Self::Sync,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
enum Availability {
    Fresh,
    Cached,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Source {
    key: String,
    fetched_at: Option<DateTime<Utc>>,
    stale_at: Option<DateTime<Utc>>,
}

impl Source {
    fn read(workspace: &Value, key: Key, term: Term) -> Self {
        let name = key.to_string();
        let fetched_at = workspace["freshness"][&name]
            .get("fetchedAt")
            .and_then(Value::as_str)
            .and_then(|raw| parse_timestamp(raw).ok())
            .map(|value| value.to_utc());
        Self {
            key: name,
            fetched_at,
            stale_at: fetched_at.and_then(|at| expires_at(at, ttl(key.dataset, term))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct Freshness {
    availability: Availability,
    coverage_complete: bool,
    updated_at: Option<DateTime<Utc>>,
}

impl Freshness {
    fn at(sources: &[Source], now: DateTime<FixedOffset>) -> Self {
        let coverage_complete = sources.iter().all(|source| source.fetched_at.is_some());
        let availability = if !coverage_complete {
            Availability::Unknown
        } else if sources
            .iter()
            .any(|source| source.stale_at.is_some_and(|at| at <= now))
        {
            Availability::Cached
        } else {
            Availability::Fresh
        };
        Self {
            availability,
            coverage_complete,
            updated_at: sources.iter().filter_map(|source| source.fetched_at).min(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CourseRow {
    title: String,
    room: String,
    color: String,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    date: String,
    destination: WidgetDestination,
}

impl From<&TodayClass> for CourseRow {
    fn from(item: &TodayClass) -> Self {
        let block = &item.occurrence;
        Self {
            title: block.title.clone(),
            room: block.room.clone(),
            color: block.color.clone(),
            starts_at: item.starts_at.to_utc(),
            ends_at: item.ends_at.to_utc(),
            date: block.date.clone(),
            destination: WidgetDestination::Course {
                key: block.key.clone(),
                date: block.date.clone(),
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CourseContent {
    date: String,
    phase: TodayPhase,
    freshness: Freshness,
    primary: Option<CourseRow>,
    upcoming: Vec<CourseRow>,
    today: Vec<CourseRow>,
    destination: WidgetDestination,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct AssignmentRow {
    title: String,
    course_title: Option<String>,
    color: Option<String>,
    due_at: Option<DateTime<Utc>>,
    overdue: bool,
    pinned: bool,
    destination: WidgetDestination,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct AssignmentContent {
    freshness: Freshness,
    rows: Vec<AssignmentRow>,
    destination: WidgetDestination,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Entry<T> {
    effective_at: DateTime<Utc>,
    content: T,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalContent {
    message_key: &'static str,
    destination: WidgetDestination,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Contract {
    schema_version: u32,
    locale: String,
    content_language: String,
    school_time_zone: &'static str,
    coverage_start: DateTime<Utc>,
    coverage_end: DateTime<Utc>,
    terminal_content: TerminalContent,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetHashes {
    pub courses: String,
    pub assignments: String,
}

impl WidgetHashes {
    pub(crate) fn get(&self, kind: WidgetKind) -> &str {
        match kind {
            WidgetKind::Courses => &self.courses,
            WidgetKind::Assignments => &self.assignments,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetPublication {
    #[serde(flatten)]
    contract: Contract,
    pub generated_at: DateTime<Utc>,
    pub content_hash: WidgetHashes,
    course_freshness: Vec<Source>,
    assignment_freshness: Vec<Source>,
    course_entries: Vec<Entry<CourseContent>>,
    assignment_entries: Vec<Entry<AssignmentContent>>,
}

impl WidgetPublication {
    pub(crate) fn metrics(&self) -> (usize, usize) {
        (self.course_entries.len(), self.assignment_entries.len())
    }

    pub(crate) fn renew_at(&self) -> DateTime<Utc> {
        self.contract.coverage_start + Duration::days(1)
    }

    pub(crate) fn display_boundaries(&self, kind: WidgetKind) -> Vec<DateTime<Utc>> {
        let mut times: Vec<_> = match kind {
            WidgetKind::Courses => entry_times(&self.course_entries).collect(),
            WidgetKind::Assignments => entry_times(&self.assignment_entries).collect(),
        };
        times.push(self.contract.coverage_end);
        times
    }
}

fn entry_times<T>(entries: &[Entry<T>]) -> impl Iterator<Item = DateTime<Utc>> + '_ {
    entries.iter().map(|entry| entry.effective_at)
}

fn hash(
    contract: &Contract,
    sources: &[Source],
    entries: &[Entry<impl Serialize>],
) -> Result<String, serde_json::Error> {
    let bytes = serde_json::to_vec(&(contract, sources, entries))?;
    Ok(data_encoding::HEXLOWER.encode(&Sha256::digest(bytes)))
}

fn entry<T: Eq>(entries: &mut Vec<Entry<T>>, time: DateTime<FixedOffset>, content: T) {
    if entries.last().is_none_or(|last| last.content != content) {
        entries.push(Entry {
            effective_at: time.to_utc(),
            content,
        });
    }
}

fn source_boundaries(sources: &[Source]) -> impl Iterator<Item = DateTime<FixedOffset>> + '_ {
    sources
        .iter()
        .filter_map(|source| source.stale_at)
        .map(|date| date.fixed_offset())
}

fn course_content(
    time: DateTime<FixedOffset>,
    today: &[TodayClass],
    later: &[Vec<TodayClass>],
    sources: &[Source],
) -> CourseContent {
    let moment = class_moment(time, today, later);
    let freshness = Freshness::at(sources, time);
    let mut upcoming = moment.upcoming.iter();
    let primary = moment
        .current
        .as_ref()
        .or_else(|| upcoming.next())
        .map(CourseRow::from);
    let destination = match &primary {
        Some(row) => row.destination.clone(),
        None if freshness.coverage_complete => WidgetDestination::Timetable,
        None => WidgetDestination::Sync,
    };
    CourseContent {
        date: tokyo_date(time),
        phase: moment.phase,
        freshness,
        primary,
        upcoming: upcoming.take(2).map(CourseRow::from).collect(),
        today: today.iter().map(CourseRow::from).collect(),
        destination,
    }
}

fn assignment_content(
    workspace: &Value,
    time: DateTime<FixedOffset>,
    sources: &[Source],
) -> Result<AssignmentContent, ScheduleError> {
    let rows = open_assignment_groups(workspace, time)?
        .into_open()
        .map(|item| {
            Ok(AssignmentRow {
                title: item.title,
                course_title: item.course_title,
                color: item.color,
                due_at: item
                    .due_at
                    .as_deref()
                    .map(parse_timestamp)
                    .transpose()?
                    .map(|at| at.to_utc()),
                overdue: item.overdue,
                pinned: item.pinned,
                destination: WidgetDestination::Assignment {
                    id: item.assignment_id,
                },
            })
        })
        .collect::<Result<Vec<_>, ScheduleError>>()?;
    let freshness = Freshness::at(sources, time);
    let destination =
        rows.first()
            .map(|row| row.destination.clone())
            .unwrap_or(if freshness.coverage_complete {
                WidgetDestination::Assignments
            } else {
                WidgetDestination::Sync
            });
    Ok(AssignmentContent {
        freshness,
        rows,
        destination,
    })
}

pub(crate) fn project(
    workspace: &Value,
    school: &SchoolReads,
    now: DateTime<Utc>,
) -> Result<WidgetPublication, crate::widgets::WidgetError> {
    let date = tokyo_date(now.fixed_offset());
    let start = at(&date, "00:00")?;
    let end = start + Duration::days(COVERAGE_DAYS);
    let last = parse_civil_date(&date)? + Duration::days(COVERAGE_DAYS + LOOKAHEAD_DAYS - 1);
    let periods = teaching_periods(workspace)?;
    let days = calendar_between(workspace, school, &date, &last.to_string())?;
    let classes = days
        .iter()
        .map(|day| timed_classes(day, &periods))
        .collect::<Result<Vec<_>, _>>()?;

    let enrolment = || Source::read(workspace, Key::lms_enrolment(), Term::Current);
    let mut course_freshness = vec![enrolment()];
    let start_year = academic_year_of(&date)?;
    let end_year = academic_year_of(&tokyo_date(end - Duration::seconds(1)))?;
    for year in start_year..=end_year {
        for kind in ["calendar", "class-schedules", "exams"] {
            course_freshness.push(Source::read(
                workspace,
                reference_key(kind, FileScope::AcademicYear(year)),
                Term::Current,
            ));
        }
    }
    let mut assignment_freshness = vec![
        enrolment(),
        Source::read(workspace, Key::lms_deadlines(), Term::Current),
    ];
    // With a fixed workspace, an open assignment can change group or age
    // out after the window starts. Time alone cannot add a new one.
    let assignment_keys: BTreeSet<_> = open_assignment_groups(workspace, start)?
        .into_open()
        .filter_map(|item| super::visible_by_id(workspace, "assignments", &item.assignment_id))
        .filter_map(|row| row["lmsCourseId"].as_u64())
        .collect();
    let current = current_lms_term(start.to_utc());
    for course in array(workspace, "lmsCourses")
        .iter()
        .filter_map(stored_course)
    {
        if assignment_keys.contains(&course.id) {
            // Deadline reads can seed a task before its course detail has been read.
            let source = Source::read(
                workspace,
                Key::lms_assignments(course.id),
                lms_course_term(&course, current),
            );
            if source.fetched_at.is_some() {
                assignment_freshness.push(source);
            }
        }
    }
    let mut course_entries = Vec::new();
    for (day, days) in classes.windows(LOOKAHEAD_DAYS as usize + 1).enumerate() {
        let (today, following) = (&days[0], &days[1..]);
        let midnight = start + Duration::days(day as i64);
        let next_midnight = midnight + Duration::days(1);
        let times: BTreeSet<_> = std::iter::once(midnight)
            .chain(source_boundaries(&course_freshness))
            .chain(today.iter().flat_map(|row| [row.starts_at, row.ends_at]))
            .filter(|at| midnight <= *at && *at < next_midnight)
            .collect();
        for time in times {
            entry(
                &mut course_entries,
                time,
                course_content(time, today, following, &course_freshness),
            );
        }
    }

    let assignment_times: BTreeSet<_> = std::iter::once(start)
        .chain(assignment_boundaries(workspace))
        .chain(source_boundaries(&assignment_freshness))
        .filter(|time| start <= *time && *time < end)
        .collect();
    let mut assignment_entries = Vec::new();
    for time in assignment_times {
        entry(
            &mut assignment_entries,
            time,
            assignment_content(workspace, time, &assignment_freshness)?,
        );
    }
    let contract = Contract {
        schema_version: SCHEMA_VERSION,
        locale: workspace
            .pointer("/settings/locale")
            .and_then(Value::as_str)
            .unwrap_or("en")
            .into(),
        content_language: content_language(workspace).into(),
        school_time_zone: SCHOOL_TIME_ZONE,
        coverage_start: start.to_utc(),
        coverage_end: end.to_utc(),
        terminal_content: TerminalContent {
            message_key: "widget.openToUpdate",
            destination: WidgetDestination::Sync,
        },
    };
    let content_hash = WidgetHashes {
        courses: hash(&contract, &course_freshness, &course_entries)?,
        assignments: hash(&contract, &assignment_freshness, &assignment_entries)?,
    };
    Ok(WidgetPublication {
        contract,
        generated_at: now,
        content_hash,
        course_freshness,
        assignment_freshness,
        course_entries,
        assignment_entries,
    })
}
