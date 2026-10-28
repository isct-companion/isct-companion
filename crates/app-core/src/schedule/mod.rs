//! Calendar and schedule read models shared by every application shell.

pub mod alternation;
mod calendar;
mod civil;
mod colors;
mod course_content;
mod course_detail;
mod credits;
mod exams;
mod ics;
mod lms_html;
mod occurrence;
mod range;
mod reminders;
pub(crate) use reminders::assignment_lead_minutes;
mod requests;
mod school;
mod search;
mod tasks;
mod today;

#[cfg(test)]
#[path = "../tests/schedule.rs"]
mod tests;

mod timing;

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
#[cfg(feature = "ts")]
use ts_rs::TS;

pub use alternation::{
    class_group_choices, validate_class_groups, validate_semester, Alternation, ClassGroup,
    ClassGroupChoice, ClassGroupCourse, ClassGroupSession, PairedCourse, Turn,
};
pub use calendar::{
    academic_term_for_date, academic_year_calendar, calendar_state, AcademicYearCalendar,
    CalendarDayView, QuarterRange, QuarterView, TeachingRange, TimetableQuarter, TimetableWeek,
    TimetableYear,
};
pub(crate) use calendar::{content_language, localized};
pub use civil::{add_days, japan_today, parse_civil_date, tokyo_date, Term};
pub use colors::{display_color, COURSE_PALETTE};
pub use course_content::{
    course_content, CourseContent, LessonContent, LessonItem, LessonItemKind,
};
pub use course_detail::{
    assignment_detail, course_detail, AssignmentAttachment, AssignmentDetail, CourseDetail,
};
pub use credits::declared_term_credits;
pub(crate) use exams::{
    adopted_exam_override, corrected_exam_override, linked_exam_override, marked_session_override,
    removed_exam_override, ExamCorrection,
};
pub use exams::{
    exam_association, exam_views, ExamAssociation, ExamConflict, ExamSchedule, ExamView,
};
pub use ics::{export_calendar_ics, export_quarter_ics, CalendarExportScope};
pub(crate) use occurrence::placement_terms;
pub use occurrence::{
    course_terms, course_timeline, intensive_days, occurrences_on_date, planning_week,
    resolve_day_agenda, schedule_problems, scheduled_courses, term_courses,
    validate_academic_profile, AgendaOptions, CourseConflict, CourseTimeline, EventCarryOver,
    GridSlot, Occurrence, OccurrenceKind, PlanningDay, PlanningWeek, ScheduleProblem,
    ScheduleProblemKind,
};
pub use range::calendar_between;
pub use reminders::{
    glance, notification_plan, GlanceSnapshot, NotificationItem, NotificationLocale,
    NotificationPlan, NotificationPriority, NotificationSink, NotificationSinkError, ReminderError,
    ReminderService, DEFAULT_ASSIGNMENT_LEADS,
};
pub use requests::{ColorCourse, ColorOverrides};
pub use school::SchoolReads;
pub(crate) use school::{CatalogEntry, ExamTimetable};
pub use search::LectureSearch;
pub(crate) use tasks::assignment_submitted;
pub use tasks::{task_view, AssignmentGroups, TaskDeadlineView, TaskView, TermTaskView};
pub(crate) use timing::{clock_for_periods, validate_period_range};
pub use timing::{
    default_teaching_periods, teaching_periods, timetable_axis, validate_teaching_periods, AxisRow,
    AxisStop, TeachingPeriod, TimetableAxis,
};
pub use today::{
    assignment_board, today_view, AssignmentBoard, DeadlineGroups, DeadlineItem, MaterialItem,
    RecentItem, RecentKind, TodayClass, TodayPhase, TodayView,
};
pub(crate) use today::{recent_key, recent_version, unseen_recent, RECENT_DAYS};

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ScheduleError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Compatibility(String),
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AgendaRequestOptions {
    #[serde(default)]
    pub include_personal: Option<bool>,
    #[serde(default)]
    pub include_cancelled_courses: bool,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "unknown[]"))]
    pub courses: Option<Vec<Value>>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "unknown[]"))]
    pub events: Option<Vec<Value>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum WorkspaceQuery {
    TeachingPeriods,
    /// The equal-row axis timetables place times on, widened to the week's
    /// earliest start and latest end when those leave the teaching day.
    TimetableAxis {
        #[serde(default)]
        earliest: Option<String>,
        #[serde(default)]
        latest: Option<String>,
    },
    WorkspaceOverview {
        term: String,
    },
    SubscriptionViews,
    PlanningWeek {
        term: String,
    },
    AcademicYears {
        date: String,
        selected_year: i32,
    },
    TaskView {
        now: String,
    },
    /// Phone Today: phase, hero and sections for this moment.
    Today {
        now: String,
    },
    /// Phone assignment lists: a quarter's timeline as rows, the current
    /// quarter's unless `term` names another (the iPad 授業 page's 課題).
    AssignmentBoard {
        now: String,
        #[serde(default)]
        term: Option<String>,
    },
    CourseDetail {
        course_id: String,
    },
    /// A course's LMS content lesson by lesson.
    CourseContent {
        course_id: String,
        now: String,
    },
    /// One assignment with its LMS text and files, for an in-app page.
    AssignmentDetail {
        assignment_id: String,
    },
    ExamAssociation {
        exam_key: String,
        course_id: String,
    },
    LectureSearch {
        term: Option<String>,
        query: String,
    },
    ScheduleView {
        /// Toolbar/credit context only; agendas include every term in the date range.
        term: String,
        from: String,
        to: String,
        #[serde(default = "default_true")]
        include_personal: bool,
    },
    DayAgenda {
        term: String,
        date: String,
        #[serde(default)]
        options: AgendaRequestOptions,
    },
    CalendarState {
        date: String,
    },
    QuarterRange {
        term: String,
    },
    ScheduledCourses {
        term: String,
    },
    AcademicYear {
        year: i32,
    },
    /// Every week of an academic year with its quarter, for the timetable's
    /// time picker.
    TimetableYear {
        year: i32,
    },
    QuarterIcs {
        term: String,
        #[serde(default)]
        scope: CalendarExportScope,
        generated_at: String,
    },
    DeclaredCredits {
        term: String,
    },
    DisplayColor {
        course: ColorCourse,
    },
    IntensiveDays {
        from: String,
        to: String,
        periods: [u8; 2],
    },
    ValidateAcademicProfile {
        profile: crate::contracts::AcademicProfile,
    },
    AcademicTermForDate {
        date: String,
        profile: crate::contracts::AcademicProfile,
    },
    Glance {
        now: String,
    },
    NotificationPlan {
        now: String,
        horizon: String,
        #[serde(default)]
        locale: Option<NotificationLocale>,
    },
    /// Semesters with 物理学演習・実験 and the red/white group of each.
    ClassGroups {
        today: String,
    },
    /// Workspace records as stored, only the fields named (e.g. `settings`,
    /// `plans`), for a page that edits them; settings, terms and the current
    /// term come with every answer.
    Records {
        fields: Vec<String>,
    },
}

use crate::workspace_changes::WorkspaceChange::{self, *};

/// Dated agendas: courses and everything else placed on a day.
const AGENDA: &[WorkspaceChange] = &[
    Courses,
    Registrations,
    Finder,
    Calendar,
    Exams,
    Plans,
    Events,
    Subscriptions,
    Settings,
];
const CALENDAR: &[WorkspaceChange] = &[Calendar, Settings];

impl WorkspaceQuery {
    /// The changes whose records this query's result is computed from. A
    /// shell keeps the result until a `workspace-changed` names one of them;
    /// a query of only its own input reads none.
    pub fn reads(&self) -> Vec<WorkspaceChange> {
        if let Self::Records { fields } = self {
            let mut named = fields.clone();
            named.extend(ALWAYS_RECORDS.map(str::to_owned));
            return crate::workspace_changes::feeding(&named);
        }
        self.fixed_reads().to_vec()
    }

    fn fixed_reads(&self) -> &'static [WorkspaceChange] {
        match self {
            Self::Records { .. } => &[Settings],
            Self::DisplayColor { .. }
            | Self::ValidateAcademicProfile { .. }
            | Self::AcademicTermForDate { .. } => &[],
            Self::TeachingPeriods
            | Self::TimetableAxis { .. }
            | Self::IntensiveDays { .. }
            | Self::AcademicYears { .. }
            | Self::CalendarState { .. }
            | Self::QuarterRange { .. }
            | Self::AcademicYear { .. }
            | Self::TimetableYear { .. } => CALENDAR,
            Self::SubscriptionViews => &[Subscriptions, Settings],
            Self::DeclaredCredits { .. } => &[Registrations, Settings],
            Self::ScheduledCourses { .. } | Self::ClassGroups { .. } => {
                &[Courses, Registrations, Finder, Calendar, Plans, Settings]
            }
            Self::WorkspaceOverview { .. } => &[
                Courses,
                Registrations,
                Finder,
                Calendar,
                Grades,
                Exams,
                Plans,
                Settings,
            ],
            Self::ScheduleView { .. } => &[
                Courses,
                Registrations,
                Finder,
                Calendar,
                Grades,
                Exams,
                Plans,
                Events,
                Subscriptions,
                Settings,
            ],
            Self::PlanningWeek { .. } | Self::DayAgenda { .. } | Self::QuarterIcs { .. } => AGENDA,
            Self::ExamAssociation { .. } => {
                &[Courses, Registrations, Finder, Calendar, Exams, Settings]
            }
            Self::TaskView { .. } | Self::AssignmentBoard { .. } => &[
                Courses,
                Registrations,
                Finder,
                Calendar,
                Assignments,
                Settings,
            ],
            Self::AssignmentDetail { .. } | Self::LectureSearch { .. } => &[
                Courses,
                Registrations,
                Finder,
                Calendar,
                Materials,
                Assignments,
                Settings,
            ],
            Self::CourseDetail { .. } => &[
                Courses,
                Registrations,
                Finder,
                Calendar,
                Materials,
                Assignments,
                Exams,
                Settings,
            ],
            Self::Today { .. }
            | Self::CourseContent { .. }
            | Self::Glance { .. }
            | Self::NotificationPlan { .. } => &[
                Courses,
                Registrations,
                Finder,
                Calendar,
                Materials,
                Assignments,
                Exams,
                Plans,
                Events,
                Subscriptions,
                Settings,
            ],
        }
    }
}

/// The union of what `queries` read, sorted, for a page that caches them
/// together.
pub fn reads_of<'a>(queries: impl IntoIterator<Item = &'a WorkspaceQuery>) -> Vec<WorkspaceChange> {
    let mut reads: Vec<_> = queries
        .into_iter()
        .flat_map(WorkspaceQuery::reads)
        .collect();
    reads.sort();
    reads.dedup();
    reads
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DatedAgenda {
    pub date: String,
    pub occurrences: Vec<Occurrence>,
    #[cfg_attr(feature = "ts", ts(type = "unknown | null"))]
    pub calendar_state: Option<Value>,
    #[cfg_attr(feature = "ts", ts(type = "unknown[]"))]
    pub important_dates: Vec<Value>,
    pub no_classes: bool,
    pub calendar_notice: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ScheduleView {
    pub term: String,
    pub range: Option<QuarterRange>,
    #[cfg_attr(feature = "ts", ts(type = "unknown[]"))]
    pub courses: Vec<Value>,
    pub agendas: Vec<DatedAgenda>,
    pub exams: Vec<ExamView>,
    pub upcoming_exams: Vec<ExamView>,
    pub declared_credits: Option<f64>,
    pub colors: std::collections::BTreeMap<String, String>,
    pub enrolled_course_ids: Vec<String>,
    pub on_timetable_course_ids: Vec<String>,
    pub all_terms: Vec<String>,
    /// Stored plans and overrides left out of `agendas`.
    pub problems: Vec<ScheduleProblem>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceOverview {
    pub term: String,
    pub current_term: String,
    pub selected_term: String,
    pub today_term: String,
    pub courses_term: String,
    pub range: Option<QuarterRange>,
    #[cfg_attr(feature = "ts", ts(type = "unknown[]"))]
    pub courses: Vec<Value>,
    /// Every visible course belonging to `term` (own quarter or placed
    /// there), on its timetable or not.
    pub term_course_ids: Vec<String>,
    pub declared_credits: Option<f64>,
    pub term_credits: f64,
    pub colors: std::collections::BTreeMap<String, String>,
    pub enrolled_course_ids: Vec<String>,
    pub on_timetable_course_ids: Vec<String>,
    pub all_terms: Vec<String>,
    pub default_finder_keys: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AcademicYearIndex {
    pub current_year: i32,
    pub years: Vec<i32>,
}

// A result is built once and moved to the caller; boxing a variant would also
// change the records the native SDK generates from this enum.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum WorkspaceQueryResult {
    TeachingPeriods(Vec<TeachingPeriod>),
    TimetableAxis(TimetableAxis),
    WorkspaceOverview(WorkspaceOverview),
    SubscriptionViews(#[cfg_attr(feature = "ts", ts(type = "unknown"))] Value),
    PlanningWeek(PlanningWeek),
    AcademicYears(AcademicYearIndex),
    TaskView(TaskView),
    Today(TodayView),
    AssignmentBoard(AssignmentBoard),
    CourseDetail(CourseDetail),
    AssignmentDetail(Option<AssignmentDetail>),
    CourseContent(Option<CourseContent>),
    ExamAssociation(ExamAssociation),
    LectureSearch(LectureSearch),
    ScheduleView(ScheduleView),
    DayAgenda(Vec<Occurrence>),
    CalendarState(#[cfg_attr(feature = "ts", ts(type = "unknown | null"))] Option<Value>),
    QuarterRange(Option<QuarterRange>),
    ScheduledCourses(#[cfg_attr(feature = "ts", ts(type = "unknown[]"))] Vec<Value>),
    AcademicYear(AcademicYearCalendar),
    TimetableYear(TimetableYear),
    QuarterIcs(String),
    DeclaredCredits(Option<f64>),
    DisplayColor(String),
    IntensiveDays(Vec<ScheduleIntensiveDay>),
    AcademicProfile(#[cfg_attr(feature = "ts", ts(type = "unknown"))] Value),
    AcademicTerm(Option<String>),
    Glance(GlanceSnapshot),
    NotificationPlan(NotificationPlan),
    ClassGroups(Vec<ClassGroupChoice>),
    Records(#[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))] Value),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ScheduleIntensiveDay {
    pub date: String,
    pub periods: [u8; 2],
}

/// `request` evaluated over `workspace` and the school records read with it.
pub fn query(
    workspace: &Value,
    school: &SchoolReads,
    request: WorkspaceQuery,
) -> Result<WorkspaceQueryResult, ScheduleError> {
    match request {
        WorkspaceQuery::LectureSearch { term, query } => {
            if let Some(term) = &term {
                Term::parse(term)?;
            }
            Ok(WorkspaceQueryResult::LectureSearch(search::lecture_search(
                workspace,
                school,
                term.as_deref(),
                &query,
            )))
        }
        WorkspaceQuery::ExamAssociation {
            exam_key,
            course_id,
        } => Ok(WorkspaceQueryResult::ExamAssociation(exam_association(
            workspace, school, &exam_key, &course_id,
        )?)),
        WorkspaceQuery::TeachingPeriods => Ok(WorkspaceQueryResult::TeachingPeriods(
            teaching_periods(workspace)?,
        )),
        WorkspaceQuery::TimetableAxis { earliest, latest } => {
            Ok(WorkspaceQueryResult::TimetableAxis(timetable_axis(
                &teaching_periods(workspace)?,
                earliest.as_deref(),
                latest.as_deref(),
            )?))
        }
        WorkspaceQuery::WorkspaceOverview { term } => Ok(WorkspaceQueryResult::WorkspaceOverview(
            workspace_overview(workspace, school, &term)?,
        )),
        WorkspaceQuery::SubscriptionViews => Ok(WorkspaceQueryResult::SubscriptionViews(
            crate::reference_refresh::subscription_views(workspace),
        )),
        WorkspaceQuery::PlanningWeek { term } => Ok(WorkspaceQueryResult::PlanningWeek(
            planning_week(workspace, &term)?,
        )),
        WorkspaceQuery::AcademicYears {
            date,
            selected_year,
        } => {
            let current_year = calendar::academic_year_of(&date)?;
            let mut years = vec![current_year, selected_year];
            years.extend(
                array(workspace, "academicCalendar")
                    .iter()
                    .filter_map(|row| {
                        row.get("academicYear")
                            .and_then(Value::as_i64)
                            .map(|year| year as i32)
                            .or_else(|| {
                                text(row, "date")
                                    .and_then(|date| calendar::academic_year_of(date).ok())
                            })
                    }),
            );
            years.extend(array(workspace, "exceptions").iter().filter_map(|row| {
                text(row, "date").and_then(|date| calendar::academic_year_of(date).ok())
            }));
            years.sort_unstable();
            years.dedup();
            Ok(WorkspaceQueryResult::AcademicYears(AcademicYearIndex {
                current_year,
                years,
            }))
        }
        WorkspaceQuery::Today { now } => Ok(WorkspaceQueryResult::Today(today_view(
            workspace,
            school,
            parse_timestamp(&now)?,
        )?)),
        WorkspaceQuery::AssignmentBoard { now, term } => {
            let term = term.map(|term| Term::parse(&term)).transpose()?;
            Ok(WorkspaceQueryResult::AssignmentBoard(assignment_board(
                workspace,
                parse_timestamp(&now)?,
                term.map(|term| term.to_string()),
            )?))
        }
        WorkspaceQuery::CourseDetail { course_id } => Ok(WorkspaceQueryResult::CourseDetail(
            course_detail(workspace, school, &course_id),
        )),
        WorkspaceQuery::CourseContent { course_id, now } => {
            Ok(WorkspaceQueryResult::CourseContent(course_content(
                workspace,
                school,
                &course_id,
                parse_timestamp(&now)?,
            )))
        }
        WorkspaceQuery::AssignmentDetail { assignment_id } => Ok(
            WorkspaceQueryResult::AssignmentDetail(assignment_detail(workspace, &assignment_id)),
        ),
        WorkspaceQuery::TaskView { now } => Ok(WorkspaceQueryResult::TaskView(task_view(
            workspace,
            parse_timestamp(&now)?,
        )?)),
        WorkspaceQuery::ScheduleView {
            term,
            from,
            to,
            include_personal,
        } => {
            let term = Term::parse(&term)?;
            let term_id = term.to_string();
            let overview = workspace_overview(workspace, school, &term_id)?;
            let courses = overview.courses.clone();
            let mut agendas = calendar_between(workspace, school, &from, &to)?;
            if !include_personal {
                let periods = teaching_periods(workspace)?;
                for agenda in &mut agendas {
                    agenda.occurrences.retain(|block| {
                        matches!(block.kind, OccurrenceKind::Course | OccurrenceKind::Exam)
                    });
                    agenda.occurrences = occurrence::finalize_agenda(
                        std::mem::take(&mut agenda.occurrences),
                        &periods,
                    )?;
                }
            }
            Ok(WorkspaceQueryResult::ScheduleView(ScheduleView {
                term: term.to_string(),
                range: overview.range,
                courses,
                agendas,
                exams: exam_views(workspace, school, &term_id),
                upcoming_exams: exams::upcoming_exams(workspace, school, &from),
                colors: overview.colors,
                enrolled_course_ids: overview.enrolled_course_ids,
                on_timetable_course_ids: overview.on_timetable_course_ids,
                all_terms: overview.all_terms,
                declared_credits: overview.declared_credits,
                problems: schedule_problems(workspace),
            }))
        }
        WorkspaceQuery::DayAgenda {
            term,
            date,
            options,
        } => {
            Term::parse(&term)?;
            parse_civil_date(&date)?;
            Ok(WorkspaceQueryResult::DayAgenda(resolve_day_agenda(
                workspace,
                school,
                &term,
                &date,
                AgendaOptions {
                    include_personal: options.include_personal.unwrap_or(true),
                    include_cancelled_courses: options.include_cancelled_courses,
                    courses: options.courses.map(Into::into),
                    events: options.events,
                },
            )?))
        }
        WorkspaceQuery::CalendarState { date } => {
            parse_civil_date(&date)?;
            Ok(WorkspaceQueryResult::CalendarState(
                calendar::calendar_state(workspace, &date),
            ))
        }
        WorkspaceQuery::QuarterRange { term } => {
            Term::parse(&term)?;
            Ok(WorkspaceQueryResult::QuarterRange(calendar::quarter_range(
                workspace, &term,
            )?))
        }
        WorkspaceQuery::ScheduledCourses { term } => {
            Term::parse(&term)?;
            Ok(WorkspaceQueryResult::ScheduledCourses(scheduled_courses(
                workspace, &term,
            )))
        }
        WorkspaceQuery::AcademicYear { year } => Ok(WorkspaceQueryResult::AcademicYear(
            academic_year_calendar(workspace, year)?,
        )),
        WorkspaceQuery::TimetableYear { year } => Ok(WorkspaceQueryResult::TimetableYear(
            calendar::timetable_year(workspace, year)?,
        )),
        WorkspaceQuery::QuarterIcs {
            term,
            scope,
            generated_at,
        } => {
            let now = parse_timestamp(&generated_at)?;
            Ok(WorkspaceQueryResult::QuarterIcs(export_quarter_ics(
                workspace, school, &term, scope, now,
            )?))
        }
        WorkspaceQuery::DeclaredCredits { term } => Ok(WorkspaceQueryResult::DeclaredCredits(
            declared_term_credits(array(workspace, "registrations"), &term)?,
        )),
        WorkspaceQuery::DisplayColor { course } => Ok(WorkspaceQueryResult::DisplayColor(
            display_color(&request_value(course)?),
        )),
        WorkspaceQuery::IntensiveDays { from, to, periods } => {
            let teaching_periods = teaching_periods(workspace)?;
            Ok(WorkspaceQueryResult::IntensiveDays(intensive_days(
                &teaching_periods,
                &from,
                &to,
                periods,
            )?))
        }
        WorkspaceQuery::ValidateAcademicProfile { profile } => {
            Ok(WorkspaceQueryResult::AcademicProfile(
                validate_academic_profile(&request_value(profile)?)?,
            ))
        }
        WorkspaceQuery::AcademicTermForDate { date, profile } => {
            Ok(WorkspaceQueryResult::AcademicTerm(academic_term_for_date(
                &date,
                &request_value(profile)?,
            )?))
        }
        WorkspaceQuery::Glance { now } => Ok(WorkspaceQueryResult::Glance(glance(
            workspace,
            school,
            parse_timestamp(&now)?,
        )?)),
        WorkspaceQuery::NotificationPlan {
            now,
            horizon,
            locale,
        } => Ok(WorkspaceQueryResult::NotificationPlan(notification_plan(
            workspace,
            school,
            parse_timestamp(&now)?,
            parse_timestamp(&horizon)?,
            locale,
        )?)),
        WorkspaceQuery::Records { fields } => {
            let mut records = serde_json::Map::new();
            for field in fields.iter().map(String::as_str).chain(ALWAYS_RECORDS) {
                if let Some(value) = workspace.get(field) {
                    records.insert(field.to_owned(), value.clone());
                }
            }
            Ok(WorkspaceQueryResult::Records(Value::Object(records)))
        }
        WorkspaceQuery::ClassGroups { today } => {
            parse_civil_date(&today)?;
            Ok(WorkspaceQueryResult::ClassGroups(class_group_choices(
                workspace, &today,
            )))
        }
    }
}

fn workspace_overview(
    workspace: &Value,
    school: &SchoolReads,
    term: &str,
) -> Result<WorkspaceOverview, ScheduleError> {
    let current_term = text(workspace, "currentTerm")
        .or_else(|| (!term.is_empty()).then_some(term))
        .ok_or_else(|| ScheduleError::InvalidInput("Current term is unavailable.".into()))?
        .to_owned();
    let selected_term = text(workspace, "selectedTerm")
        .unwrap_or(&current_term)
        .to_owned();
    let mut on_timetable_course_ids: Vec<String> = array(workspace, "memberships")
        .iter()
        .filter(|row| visible(row))
        .filter_map(|row| text(row, "courseId").map(str::to_owned))
        .collect();
    on_timetable_course_ids.extend(
        array(workspace, "planItems")
            .iter()
            .filter(|row| visible(row))
            .filter_map(|row| text(row, "courseId").map(str::to_owned)),
    );
    on_timetable_course_ids.sort();
    on_timetable_course_ids.dedup();
    let earned = credits::earned_credits_by_term(array(workspace, "grades"));
    let mut all_terms: Vec<String> = placement_terms(workspace).map(str::to_owned).collect();
    all_terms.extend(exams::exam_terms(school));
    all_terms.extend(earned.keys().cloned());
    all_terms.extend(
        array(workspace, "terms")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned),
    );
    if !current_term.is_empty() {
        all_terms.push(current_term.clone());
    }
    all_terms.sort();
    all_terms.dedup();
    let colors = array(workspace, "courses")
        .iter()
        .filter(|course| visible(course))
        .filter_map(|course| Some((text(course, "id")?.to_owned(), display_color(course))))
        .collect();
    let today_term = current_term.clone();
    let courses_term = if all_terms.contains(&selected_term) {
        selected_term.clone()
    } else {
        current_term.clone()
    };
    let term = if term.is_empty() {
        courses_term.clone()
    } else {
        Term::parse(term)?.to_string()
    };
    Term::parse(&term)?;
    let courses = scheduled_courses(workspace, &term);
    let term_course_ids = term_courses(workspace, &term)
        .into_iter()
        .filter_map(|course| text(course, "id").map(str::to_owned))
        .collect();
    let enrolled_course_ids: Vec<String> = array(workspace, "memberships")
        .iter()
        .filter(|row| visible(row) && text(row, "termId") == Some(term.as_str()))
        .filter_map(|row| text(row, "courseId").map(str::to_owned))
        .collect();
    let declared_credits = declared_term_credits(array(workspace, "registrations"), &term)?;
    let current = Term::parse(&current_term)?;
    let selected = Term::parse(&term)?;
    let current_semester =
        current.year == selected.year && (current.quarter - 1) / 2 == (selected.quarter - 1) / 2;
    let term_credits = if current_semester {
        declared_credits.or_else(|| earned.get(&term).copied())
    } else {
        earned.get(&term).copied().or(declared_credits)
    }
    .unwrap_or_else(|| {
        courses
            .iter()
            .filter_map(|course| course.get("credits").and_then(Value::as_f64))
            .sum()
    });
    let planned = Term::parse(&term)?;
    let mut default_finder_keys = vec![if term == current_term {
        "curr".into()
    } else {
        format!("{}Q", planned.quarter)
    }];
    let department_id = workspace
        .pointer("/settings/departmentId")
        .and_then(Value::as_str);
    let department_name = department_id
        .map(crate::course_query::department_name_ja)
        .transpose()
        .map_err(|error| ScheduleError::Compatibility(error.to_string()))?
        .flatten();
    if let Some(name) = department_name {
        if !name.is_empty() {
            default_finder_keys.push(name);
        }
    }
    Ok(WorkspaceOverview {
        term: term.clone(),
        current_term,
        selected_term,
        today_term,
        courses_term,
        range: calendar::quarter_range(workspace, &term)?,
        courses,
        term_course_ids,
        declared_credits,
        term_credits,
        colors,
        enrolled_course_ids,
        on_timetable_course_ids,
        all_terms,
        default_finder_keys,
    })
}

/// Fields every `Records` answer carries.
const ALWAYS_RECORDS: [&str; 6] = [
    "settings",
    "academicProfile",
    "deviceId",
    "selectedTerm",
    "currentTerm",
    "terms",
];

fn default_true() -> bool {
    true
}

pub(crate) fn parse_timestamp(raw: &str) -> Result<DateTime<FixedOffset>, ScheduleError> {
    static RFC3339: std::sync::LazyLock<Result<regex::Regex, regex::Error>> =
        std::sync::LazyLock::new(|| {
            regex::Regex::new(
                r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$",
            )
        });
    let valid_shape = RFC3339
        .as_ref()
        .map_err(|error| ScheduleError::Compatibility(error.to_string()))?
        .is_match(raw);
    if !valid_shape {
        return Err(ScheduleError::InvalidInput(
            "timestamp must be RFC 3339".into(),
        ));
    }
    DateTime::parse_from_rfc3339(raw)
        .map_err(|_| ScheduleError::InvalidInput("timestamp must be RFC 3339".into()))
}

pub(crate) fn array<'a>(workspace: &'a Value, key: &str) -> &'a [Value] {
    workspace
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

pub(crate) fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// A string field, or "" when it is missing or not a string.
pub(crate) fn text_or_empty<'a>(value: &'a Value, key: &str) -> &'a str {
    text(value, key).unwrap_or_default()
}

/// Not tombstoned. Unlike [`visible`], a row the school no longer lists
/// still counts: course details keep cached rows until they are deleted.
pub(super) fn not_deleted(row: &Value) -> bool {
    row.get("deletedAt").is_none_or(Value::is_null)
}

pub(crate) fn visible(value: &Value) -> bool {
    value.get("deletedAt").is_none_or(Value::is_null)
        && value.get("missingSince").is_none_or(Value::is_null)
}

#[cfg(feature = "ts")]
pub fn declarations() -> Vec<String> {
    vec![
        TeachingPeriod::decl(),
        TimetableAxis::decl(),
        AxisStop::decl(),
        AxisRow::decl(),
        Term::decl(),
        TeachingRange::decl(),
        QuarterRange::decl(),
        CalendarDayView::decl(),
        QuarterView::decl(),
        AcademicYearCalendar::decl(),
        TimetableWeek::decl(),
        TimetableQuarter::decl(),
        TimetableYear::decl(),
        CalendarExportScope::decl(),
        ClassGroup::decl(),
        Turn::decl(),
        Alternation::decl(),
        PairedCourse::decl(),
        ClassGroupCourse::decl(),
        ClassGroupSession::decl(),
        ClassGroupChoice::decl(),
        OccurrenceKind::decl(),
        GridSlot::decl(),
        Occurrence::decl(),
        ScheduleProblemKind::decl(),
        ScheduleProblem::decl(),
        NotificationLocale::decl(),
        NotificationPriority::decl(),
        NotificationItem::decl(),
        NotificationPlan::decl(),
        GlanceSnapshot::decl(),
        AgendaRequestOptions::decl(),
        DatedAgenda::decl(),
        PlanningDay::decl(),
        CourseConflict::decl(),
        PlanningWeek::decl(),
        EventCarryOver::decl(),
        ColorCourse::decl(),
        ColorOverrides::decl(),
        CourseDetail::decl(),
        AssignmentDetail::decl(),
        CourseContent::decl(),
        LessonContent::decl(),
        LessonItem::decl(),
        LessonItemKind::decl(),
        AssignmentAttachment::decl(),
        <crate::course_materials::CourseKey as TS>::decl(),
        ExamView::decl(),
        ExamSchedule::decl(),
        ExamConflict::decl(),
        ExamAssociation::decl(),
        LectureSearch::decl(),
        TaskDeadlineView::decl(),
        AssignmentGroups::decl(),
        TermTaskView::decl(),
        TaskView::decl(),
        TodayPhase::decl(),
        TodayClass::decl(),
        DeadlineItem::decl(),
        MaterialItem::decl(),
        RecentItem::decl(),
        RecentKind::decl(),
        TodayView::decl(),
        DeadlineGroups::decl(),
        AssignmentBoard::decl(),
        AcademicYearIndex::decl(),
        WorkspaceOverview::decl(),
        ScheduleView::decl(),
        ScheduleIntensiveDay::decl(),
        WorkspaceQuery::decl(),
        WorkspaceQueryResult::decl(),
    ]
}

fn request_value(value: impl Serialize) -> Result<Value, ScheduleError> {
    serde_json::to_value(value).map_err(|error| ScheduleError::InvalidInput(error.to_string()))
}
