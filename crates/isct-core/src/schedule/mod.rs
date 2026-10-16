//! Calendar and schedule read models shared by every application shell.

pub mod alternation;
mod calendar;
mod civil;
mod colors;
mod credits;
mod ics;
mod occurrence;
mod reminders;
mod tasks;
#[cfg(test)]
mod tests;

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
#[cfg(feature = "ts")]
use ts_rs::TS;

pub use alternation::{
    class_group_choices, validate_class_groups, validate_semester, Alternation, ClassGroup,
    ClassGroupChoice, ClassGroupCourse, ClassGroupSession, Turn,
};
pub use calendar::{
    academic_term_for_date, academic_year_calendar, calendar_state, AcademicYearCalendar,
    CalendarDayView, QuarterRange, QuarterView, TeachingRange,
};
pub(crate) use civil::clock_for_periods;
pub use civil::{add_days, parse_civil_date, tokyo_date, Term, PERIOD_CLOCKS};
pub use colors::{display_color, COURSE_PALETTE};
pub use credits::declared_term_credits;
pub use ics::{export_quarter_ics, CalendarExportScope};
pub(crate) use occurrence::placement_terms;
pub use occurrence::{
    course_terms, course_timeline, intensive_days, next_plan_occurrences, occurrences_on_date,
    plan_start, planning_week, resolve_day_agenda, schedule_problems, scheduled_courses,
    term_courses, validate_academic_profile, AgendaOptions, CourseTimeline, Occurrence,
    OccurrenceKind, PlanCivilStart, PlanningDay, PlanningWeek, ScheduleProblem,
    ScheduleProblemKind,
};
pub use reminders::{
    glance, notification_plan, GlanceSnapshot, NotificationItem, NotificationLocale,
    NotificationPlan, NotificationPriority, NotificationSink, NotificationSinkError, ReminderError,
    ReminderService,
};
pub(crate) use tasks::assignment_submitted;
pub use tasks::{task_view, TaskDeadlineView, TaskView, TermTaskView};

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
    PlanCivilStart {
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        plan: Value,
    },
    CourseTimeline {
        course_id: String,
        now: String,
        #[serde(default = "default_timeline_before")]
        before_days: u16,
        #[serde(default = "default_timeline_after")]
        after_days: u16,
    },
    TaskView {
        now: String,
    },
    ScheduleView {
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
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        course: Value,
    },
    IntensiveDays {
        from: String,
        to: String,
        periods: [u8; 2],
    },
    NextPlanOccurrences {
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        plan: Value,
        #[serde(default = "default_occurrence_limit")]
        limit: u16,
    },
    ValidateAcademicProfile {
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        profile: Value,
    },
    AcademicTermForDate {
        date: String,
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        profile: Value,
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

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum WorkspaceQueryResult {
    WorkspaceOverview(WorkspaceOverview),
    SubscriptionViews(#[cfg_attr(feature = "ts", ts(type = "unknown"))] Value),
    PlanningWeek(PlanningWeek),
    AcademicYears(AcademicYearIndex),
    PlanCivilStart(PlanCivilStart),
    CourseTimeline(CourseTimeline),
    TaskView(TaskView),
    ScheduleView(ScheduleView),
    DayAgenda(Vec<Occurrence>),
    CalendarState(#[cfg_attr(feature = "ts", ts(type = "unknown | null"))] Option<Value>),
    QuarterRange(Option<QuarterRange>),
    ScheduledCourses(#[cfg_attr(feature = "ts", ts(type = "unknown[]"))] Vec<Value>),
    AcademicYear(AcademicYearCalendar),
    QuarterIcs(String),
    DeclaredCredits(Option<f64>),
    DisplayColor(String),
    IntensiveDays(Vec<ScheduleIntensiveDay>),
    NextPlanOccurrences(Vec<String>),
    AcademicProfile(#[cfg_attr(feature = "ts", ts(type = "unknown"))] Value),
    AcademicTerm(Option<String>),
    Glance(GlanceSnapshot),
    NotificationPlan(NotificationPlan),
    ClassGroups(Vec<ClassGroupChoice>),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ScheduleIntensiveDay {
    pub date: String,
    pub periods: [u8; 2],
}

pub fn query(
    workspace: &Value,
    request: WorkspaceQuery,
) -> Result<WorkspaceQueryResult, ScheduleError> {
    match request {
        WorkspaceQuery::WorkspaceOverview { term } => Ok(WorkspaceQueryResult::WorkspaceOverview(
            workspace_overview(workspace, &term)?,
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
        WorkspaceQuery::PlanCivilStart { plan } => {
            Ok(WorkspaceQueryResult::PlanCivilStart(plan_start(&plan)?))
        }
        WorkspaceQuery::CourseTimeline {
            course_id,
            now,
            before_days,
            after_days,
        } => Ok(WorkspaceQueryResult::CourseTimeline(course_timeline(
            workspace,
            &course_id,
            parse_timestamp(&now)?,
            before_days,
            after_days,
        )?)),
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
            let from_date = parse_civil_date(&from)?;
            let to_date = parse_civil_date(&to)?;
            let days = (to_date - from_date).num_days();
            if !(0..=366).contains(&days) {
                return Err(ScheduleError::InvalidInput(
                    "date range must be ordered and no longer than 367 days".into(),
                ));
            }
            let term_id = term.to_string();
            let overview = workspace_overview(workspace, &term_id)?;
            let courses = overview.courses.clone();
            let mut agendas = Vec::with_capacity((days + 1) as usize);
            for offset in 0..=days {
                let date = (from_date + chrono::Duration::days(offset)).to_string();
                let calendar_state = calendar::calendar_state(workspace, &date);
                let outside_teaching = calendar::quarter_range(workspace, &term.to_string())?
                    .is_some_and(|range| {
                        date.as_str() >= range.from.as_str()
                            && date.as_str() <= range.to.as_str()
                            && range.teaching_ranges.is_some_and(|ranges| {
                                calendar_state.as_ref().and_then(|row| text(row, "type"))
                                    != Some("replacement")
                                    && !ranges
                                        .iter()
                                        .any(|range| date >= range.start && date <= range.end)
                            })
                    });
                let no_classes = outside_teaching
                    || calendar_state
                        .as_ref()
                        .and_then(|row| text(row, "type"))
                        .is_some_and(|kind| matches!(kind, "holiday" | "no-class" | "break"));
                let locale = calendar::content_language(workspace);
                let mut notices = Vec::new();
                if let Some(state) = calendar_state.as_ref() {
                    let label = calendar::localized(state.get("label"), locale, "");
                    if !label.is_empty() {
                        notices.push(label);
                    }
                }
                let important_dates: Vec<_> = array(workspace, "importantDates")
                    .iter()
                    .filter(|row| {
                        visible(row)
                            && text(row, "startDate").is_some_and(|from| date.as_str() >= from)
                            && text(row, "endDate").is_some_and(|to| date.as_str() <= to)
                    })
                    .cloned()
                    .collect();
                notices.extend(important_dates.iter().filter_map(|row| {
                    let label = calendar::localized(row.get("label"), locale, "");
                    (!label.is_empty()).then_some(label)
                }));
                if notices.is_empty() && no_classes {
                    notices.push(
                        if locale == "ja" {
                            "授業なし"
                        } else {
                            "No classes"
                        }
                        .into(),
                    );
                }
                agendas.push(DatedAgenda {
                    occurrences: resolve_day_agenda(
                        workspace,
                        &term.to_string(),
                        &date,
                        AgendaOptions {
                            include_personal,
                            include_cancelled_courses: true,
                            ..AgendaOptions::default()
                        },
                    )?,
                    important_dates,
                    calendar_state,
                    no_classes,
                    calendar_notice: (!notices.is_empty()).then(|| notices.join(" · ")),
                    date,
                });
            }
            Ok(WorkspaceQueryResult::ScheduleView(ScheduleView {
                term: term.to_string(),
                range: overview.range,
                courses,
                agendas,
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
                &term,
                &date,
                AgendaOptions {
                    include_personal: options.include_personal.unwrap_or(true),
                    include_cancelled_courses: options.include_cancelled_courses,
                    courses: options.courses,
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
        WorkspaceQuery::QuarterIcs {
            term,
            scope,
            generated_at,
        } => {
            let now = parse_timestamp(&generated_at)?;
            Ok(WorkspaceQueryResult::QuarterIcs(export_quarter_ics(
                workspace, &term, scope, now,
            )?))
        }
        WorkspaceQuery::DeclaredCredits { term } => Ok(WorkspaceQueryResult::DeclaredCredits(
            declared_term_credits(array(workspace, "registrations"), &term)?,
        )),
        WorkspaceQuery::DisplayColor { course } => {
            Ok(WorkspaceQueryResult::DisplayColor(display_color(&course)))
        }
        WorkspaceQuery::IntensiveDays { from, to, periods } => Ok(
            WorkspaceQueryResult::IntensiveDays(intensive_days(&from, &to, periods)?),
        ),
        WorkspaceQuery::NextPlanOccurrences { plan, limit } => Ok(
            WorkspaceQueryResult::NextPlanOccurrences(next_plan_occurrences(&plan, limit)?),
        ),
        WorkspaceQuery::ValidateAcademicProfile { profile } => Ok(
            WorkspaceQueryResult::AcademicProfile(validate_academic_profile(&profile)?),
        ),
        WorkspaceQuery::AcademicTermForDate { date, profile } => Ok(
            WorkspaceQueryResult::AcademicTerm(academic_term_for_date(&date, &profile)?),
        ),
        WorkspaceQuery::Glance { now } => Ok(WorkspaceQueryResult::Glance(glance(
            workspace,
            parse_timestamp(&now)?,
        )?)),
        WorkspaceQuery::NotificationPlan {
            now,
            horizon,
            locale,
        } => Ok(WorkspaceQueryResult::NotificationPlan(notification_plan(
            workspace,
            parse_timestamp(&now)?,
            parse_timestamp(&horizon)?,
            locale,
        )?)),
        WorkspaceQuery::ClassGroups { today } => {
            parse_civil_date(&today)?;
            Ok(WorkspaceQueryResult::ClassGroups(class_group_choices(
                workspace, &today,
            )))
        }
    }
}

fn workspace_overview(workspace: &Value, term: &str) -> Result<WorkspaceOverview, ScheduleError> {
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
    let mut all_terms: Vec<String> = placement_terms(workspace).map(str::to_owned).collect();
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
    // Today and Lecture Desk follow the current quarter once it has courses.
    let today_term = if !term_courses(workspace, &current_term).is_empty() {
        current_term.clone()
    } else {
        selected_term.clone()
    };
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
    let term_credits = declared_credits.unwrap_or_else(|| {
        courses
            .iter()
            .filter_map(|course| course.get("credits").and_then(Value::as_f64))
            .sum()
    });
    let planned = Term::parse(&courses_term)?;
    let mut default_finder_keys = vec![if courses_term == current_term {
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

fn default_true() -> bool {
    true
}
fn default_occurrence_limit() -> u16 {
    5
}
fn default_timeline_before() -> u16 {
    14
}
fn default_timeline_after() -> u16 {
    14
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

pub(crate) fn visible(value: &Value) -> bool {
    value.get("deletedAt").is_none_or(Value::is_null)
        && value.get("missingSince").is_none_or(Value::is_null)
}

#[cfg(feature = "ts")]
pub fn declarations() -> Vec<String> {
    vec![
        Term::decl(),
        TeachingRange::decl(),
        QuarterRange::decl(),
        CalendarDayView::decl(),
        QuarterView::decl(),
        AcademicYearCalendar::decl(),
        CalendarExportScope::decl(),
        ClassGroup::decl(),
        Turn::decl(),
        Alternation::decl(),
        ClassGroupCourse::decl(),
        ClassGroupSession::decl(),
        ClassGroupChoice::decl(),
        OccurrenceKind::decl(),
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
        PlanningWeek::decl(),
        PlanCivilStart::decl(),
        CourseTimeline::decl(),
        TaskDeadlineView::decl(),
        TermTaskView::decl(),
        TaskView::decl(),
        AcademicYearIndex::decl(),
        WorkspaceOverview::decl(),
        ScheduleView::decl(),
        ScheduleIntensiveDay::decl(),
        WorkspaceQuery::decl(),
        WorkspaceQueryResult::decl(),
    ]
}
