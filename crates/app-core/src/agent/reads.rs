//! Tools that answer from local data and change nothing.
use super::dto::{self, pick, pick_all};
use super::registry::{Effect, Tool};
use super::{
    fallback_term, non_empty, now_jst, parse_term, text, visible_rows, AgentError, Context,
};
use crate::workspace_changes::WorkspaceChange as W;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct NoArgs {}

#[derive(Clone, Copy, Deserialize, JsonSchema)]
pub(super) enum Quarter {
    #[serde(rename = "1Q")]
    Q1,
    #[serde(rename = "2Q")]
    Q2,
    #[serde(rename = "3Q")]
    Q3,
    #[serde(rename = "4Q")]
    Q4,
}

impl Quarter {
    fn as_str(self) -> &'static str {
        match self {
            Self::Q1 => "1Q",
            Self::Q2 => "2Q",
            Self::Q3 => "3Q",
            Self::Q4 => "4Q",
        }
    }
}

pub(super) struct Today;

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct TodayAnswer {
    /// The moment answered, RFC 3339 in Japan time.
    now: String,
    date: String,
    term: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    calendar: Option<dto::CalendarDay>,
    classes: Vec<dto::ClassToday>,
    events: Vec<dto::EventToday>,
    /// Open assignments not yet due, soonest first.
    assignments: Vec<dto::Assignment>,
    /// Open assignments overdue in the last 14 days.
    overdue: Vec<dto::Assignment>,
    academic_summary: Vec<dto::GpaSummary>,
}

impl Tool for Today {
    const NAME: &'static str = "today";
    const DESCRIPTION: &'static str = "Today in Japan time: the calendar state (holiday, replacement day), classes actually held, weekly personal events falling today, open assignments not yet due, open assignments overdue in the last 14 days, and the GPA summary.";
    const EFFECT: Effect = Effect::Read;
    const READS: &'static [W] = &[
        W::Courses,
        W::Registrations,
        W::Finder,
        W::Calendar,
        W::Exams,
        W::Plans,
        W::Events,
        W::Subscriptions,
        W::Assignments,
        W::Grades,
        W::Settings,
    ];
    type Args = NoArgs;
    type Output = TodayAnswer;

    fn run(cx: &Context<'_>, _: NoArgs) -> Result<TodayAnswer, AgentError> {
        let (workspace, _) = cx.view()?;
        let now = now_jst()?;
        let date = now.date_naive();
        let iso = date.to_string();
        let term = text(&workspace, "currentTerm")
            .map(str::to_string)
            .unwrap_or_else(|| fallback_term(date));
        let due = |row: &Value| {
            text(row, "dueAt").and_then(|due| chrono::DateTime::parse_from_rfc3339(due).ok())
        };
        let mut open = visible_rows(&workspace, "assignments")
            .into_iter()
            .filter(|row| {
                row.get("hidden") != Some(&Value::Bool(true))
                    && !crate::schedule::assignment_submitted(row)
            })
            .collect::<Vec<_>>();
        open.sort_by_key(|row| due(row));
        let (upcoming, overdue): (Vec<_>, Vec<_>) = open
            .into_iter()
            .partition(|row| due(row).is_none_or(|due| due >= now));
        let recent = now - chrono::Duration::days(14);
        let overdue = overdue
            .into_iter()
            .filter(|row| due(row).is_some_and(|due| due >= recent))
            .collect::<Vec<_>>();
        Ok(TodayAnswer {
            now: now.to_rfc3339(),
            calendar: crate::schedule::calendar_state(&workspace, &iso)
                .map(|row| pick(&row))
                .transpose()?,
            classes: occurrences(&workspace, &term, &iso, false)
                .into_iter()
                .filter(|row| row.kind == crate::schedule::OccurrenceKind::Course)
                .map(|row| dto::ClassToday {
                    course_id: row.course_id,
                    code: row.subtitle,
                    title: row.title,
                    start_time: row.start_time,
                    end_time: row.end_time,
                    room: row.room,
                })
                .collect(),
            events: occurrences(&workspace, &term, &iso, true)
                .into_iter()
                .filter(|row| row.kind == crate::schedule::OccurrenceKind::Event)
                .map(|row| dto::EventToday {
                    id: row.id,
                    title: row.title,
                    start_time: row.start_time,
                    end_time: row.end_time,
                    room: row.room,
                })
                .collect(),
            assignments: upcoming
                .iter()
                .map(dto::Assignment::from_row)
                .collect::<Result<_, _>>()?,
            overdue: overdue
                .iter()
                .map(dto::Assignment::from_row)
                .collect::<Result<_, _>>()?,
            academic_summary: pick_all(&visible_rows(&workspace, "gpaSummary"))?,
            date: iso,
            term,
        })
    }
}

/// What is actually held on `date`, after holidays, replacement days and
/// the quarter's teaching ranges; personal events only when asked.
fn occurrences(
    workspace: &Value,
    term: &str,
    date: &str,
    include_personal: bool,
) -> Vec<crate::schedule::Occurrence> {
    // A date the calendar cannot place answers with nothing held.
    crate::schedule::occurrences_on_date(
        workspace,
        term,
        date,
        crate::schedule::AgendaOptions {
            include_personal,
            ..crate::schedule::AgendaOptions::default()
        },
    )
    .unwrap_or_default()
}

pub(super) struct Schedule;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct DateRange {
    /// First date, YYYY-MM-DD.
    from: chrono::NaiveDate,
    /// Last date, YYYY-MM-DD; at most 366 days after `from`.
    to: chrono::NaiveDate,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct ScheduleAnswer {
    from: String,
    to: String,
    courses: Vec<dto::Course>,
    intensive_schedules: Vec<dto::IntensiveSchedule>,
    events: Vec<dto::Event>,
}

impl Tool for Schedule {
    const NAME: &'static str = "schedule";
    const DESCRIPTION: &'static str = "Registered or planned courses of the quarters in an inclusive date range (at most 366 days), intensive dates and the weekly personal events of those quarters.";
    const EFFECT: Effect = Effect::Read;
    const READS: &'static [W] = &[
        W::Courses,
        W::Registrations,
        W::Finder,
        W::Calendar,
        W::Plans,
        W::Events,
        W::Settings,
    ];
    type Args = DateRange;
    type Output = ScheduleAnswer;

    fn run(
        cx: &Context<'_>,
        DateRange { from, to }: DateRange,
    ) -> Result<ScheduleAnswer, AgentError> {
        if !(0..=365).contains(&(to - from).num_days()) {
            return Err("date range must be ordered and no longer than 366 days".into());
        }
        let (workspace, _) = cx.view()?;
        let mut terms = BTreeSet::new();
        let mut cursor = from;
        while cursor <= to {
            terms.insert(fallback_term(cursor));
            cursor += chrono::Duration::days(1);
        }
        let date = |row: &Value, key: &str| {
            text(row, key)
                .and_then(|value| chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
        };
        let courses = terms
            .iter()
            .flat_map(|term| crate::schedule::scheduled_courses(&workspace, term))
            .collect::<Vec<_>>();
        let intensive = visible_rows(&workspace, "intensiveSchedules")
            .into_iter()
            .filter(|row| {
                matches!((date(row, "from"), date(row, "to")),
                    (Some(start), Some(end)) if start <= to && end >= from)
            })
            .collect::<Vec<_>>();
        let events = visible_rows(&workspace, "events")
            .into_iter()
            .filter(|row| text(row, "termId").is_some_and(|term| terms.contains(term)))
            .collect::<Vec<_>>();
        Ok(ScheduleAnswer {
            from: from.to_string(),
            to: to.to_string(),
            courses: pick_all(&courses)?,
            intensive_schedules: pick_all(&intensive)?,
            events: pick_all(&events)?,
        })
    }
}

pub(super) struct Courses;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct QuarterArgs {
    year: i64,
    quarter: Quarter,
}

#[derive(Serialize, JsonSchema)]
pub(super) struct CoursesAnswer {
    courses: Vec<dto::Course>,
}

impl Tool for Courses {
    const NAME: &'static str = "courses";
    const DESCRIPTION: &'static str = "Registered or planned courses for one academic quarter.";
    const EFFECT: Effect = Effect::Read;
    const READS: &'static [W] = &[
        W::Courses,
        W::Registrations,
        W::Finder,
        W::Calendar,
        W::Plans,
        W::Settings,
    ];
    type Args = QuarterArgs;
    type Output = CoursesAnswer;

    fn run(cx: &Context<'_>, args: QuarterArgs) -> Result<CoursesAnswer, AgentError> {
        let term = format!("{}-{}", args.year, args.quarter.as_str());
        parse_term(&term)?;
        let (workspace, _) = cx.view()?;
        Ok(CoursesAnswer {
            courses: pick_all(&crate::schedule::scheduled_courses(&workspace, &term))?,
        })
    }
}

pub(super) struct Course;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct IdArgs {
    pub(super) id: String,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct CourseAnswer {
    course: dto::Course,
    lms_items: Vec<dto::LmsItem>,
    assignments: Vec<dto::Assignment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    syllabus_url: Option<String>,
}

impl Tool for Course {
    const NAME: &'static str = "course";
    const DESCRIPTION: &'static str =
        "One course with its LMS items, assignments and the link to its syllabus page.";
    const EFFECT: Effect = Effect::Read;
    const READS: &'static [W] = &[
        W::Courses,
        W::Registrations,
        W::Finder,
        W::Calendar,
        W::Materials,
        W::Assignments,
        W::Exams,
        W::Settings,
    ];
    type Args = IdArgs;
    type Output = CourseAnswer;

    fn run(cx: &Context<'_>, IdArgs { id }: IdArgs) -> Result<CourseAnswer, AgentError> {
        let id = non_empty(&id, "id")?;
        let (workspace, school) = cx.view()?;
        let course = visible_rows(&workspace, "courses")
            .into_iter()
            .find(|row| text(row, "id") == Some(id))
            .ok_or("course was not found")?;
        let of_course = |row: &&Value| text(row, "courseId") == Some(id);
        Ok(CourseAnswer {
            course: pick(&course)?,
            lms_items: cx
                .store
                .table_rows("lms_items")?
                .iter()
                .filter(of_course)
                .filter(|row| row.get("visible") != Some(&Value::Bool(false)))
                .map(dto::LmsItem::from_row)
                .collect::<Result<_, _>>()?,
            assignments: visible_rows(&workspace, "assignments")
                .iter()
                .filter(of_course)
                .filter(|row| row.get("hidden") != Some(&Value::Bool(true)))
                .map(dto::Assignment::from_row)
                .collect::<Result<_, _>>()?,
            syllabus_url: crate::schedule::course_detail(&workspace, &school, id).syllabus_url,
        })
    }
}

pub(super) struct SearchCatalog;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct QueryArgs {
    /// Words of a title, code or instructor; empty lists the catalog.
    query: String,
}

#[derive(Serialize, JsonSchema)]
pub(super) struct CatalogAnswer {
    /// At most 100, best match first.
    offerings: Vec<dto::Offering>,
}

impl Tool for SearchCatalog {
    const NAME: &'static str = "search_catalog";
    const DESCRIPTION: &'static str =
        "Search the local bilingual course catalog (the 教務 course search the app last read).";
    const EFFECT: Effect = Effect::Read;
    type Args = QueryArgs;
    type Output = CatalogAnswer;

    fn run(cx: &Context<'_>, QueryArgs { query }: QueryArgs) -> Result<CatalogAnswer, AgentError> {
        Ok(CatalogAnswer {
            offerings: pick_all(&cx.store.catalog_search(&query)?)?,
        })
    }
}

pub(super) struct Registration;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct RegistrationArgs {
    #[serde(default)]
    year: Option<i64>,
    #[serde(default)]
    term: Option<Quarter>,
}

#[derive(Serialize, JsonSchema)]
pub(super) struct RegistrationAnswer {
    registrations: Vec<dto::Registration>,
}

impl Tool for Registration {
    const NAME: &'static str = "registration";
    const DESCRIPTION: &'static str = "Imported read-only registration records.";
    const EFFECT: Effect = Effect::Read;
    type Args = RegistrationArgs;
    type Output = RegistrationAnswer;

    fn run(cx: &Context<'_>, args: RegistrationArgs) -> Result<RegistrationAnswer, AgentError> {
        let term = args.term.map(Quarter::as_str);
        let rows =
            cx.store
                .table_rows("registrations")?
                .into_iter()
                .filter(|row| {
                    let offering = text(row, "offeringId").unwrap_or_default();
                    let matches_year = args.year.is_none_or(|year| {
                        offering.get(..4).and_then(|v| v.parse::<i64>().ok()) == Some(year)
                    });
                    let matches_term = term.is_none_or(|term| {
                        let number = term.trim_end_matches('Q').parse::<u64>().ok();
                        text(row, "termId").is_some_and(|value| value.ends_with(term))
                            || row.get("quarters").and_then(Value::as_array).is_some_and(
                                |quarters| {
                                    quarters.iter().any(|quarter| {
                                        quarter.as_u64() == number || quarter.as_str() == Some(term)
                                    })
                                },
                            )
                    });
                    matches_year && matches_term
                })
                .collect::<Vec<_>>();
        Ok(RegistrationAnswer {
            registrations: pick_all(&rows)?,
        })
    }
}

pub(super) struct Grades;

#[derive(Serialize, JsonSchema)]
pub(super) struct GradesAnswer {
    grades: Vec<dto::Grade>,
}

impl Tool for Grades {
    const NAME: &'static str = "grades";
    const DESCRIPTION: &'static str = "Imported official grade records.";
    const EFFECT: Effect = Effect::Read;
    type Args = NoArgs;
    type Output = GradesAnswer;

    fn run(cx: &Context<'_>, _: NoArgs) -> Result<GradesAnswer, AgentError> {
        Ok(GradesAnswer {
            grades: pick_all(&cx.store.table_rows("grades")?)?,
        })
    }
}

pub(super) struct Gpa;

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct GpaAnswer {
    summary: Vec<dto::GpaSummary>,
    by_term: Vec<dto::GpaTerm>,
}

impl Tool for Gpa {
    const NAME: &'static str = "gpa";
    const DESCRIPTION: &'static str = "Imported GPA, GPT and credit summaries.";
    const EFFECT: Effect = Effect::Read;
    type Args = NoArgs;
    type Output = GpaAnswer;

    fn run(cx: &Context<'_>, _: NoArgs) -> Result<GpaAnswer, AgentError> {
        Ok(GpaAnswer {
            summary: pick_all(&cx.store.table_rows("gpa_summary")?)?,
            by_term: pick_all(&cx.store.table_rows("gpa_by_term")?)?,
        })
    }
}

pub(super) struct Requirements;

#[derive(Serialize, JsonSchema)]
pub(super) struct RequirementsAnswer {
    requirements: Vec<dto::Requirement>,
}

impl Tool for Requirements {
    const NAME: &'static str = "requirements";
    const DESCRIPTION: &'static str = "Imported graduation requirement progress.";
    const EFFECT: Effect = Effect::Read;
    type Args = NoArgs;
    type Output = RequirementsAnswer;

    fn run(cx: &Context<'_>, _: NoArgs) -> Result<RequirementsAnswer, AgentError> {
        Ok(RequirementsAnswer {
            requirements: pick_all(&cx.store.table_rows("requirement_progress")?)?,
        })
    }
}

pub(super) struct Recommended;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct RecommendedArgs {
    /// Defaults to the department chosen in Settings.
    #[serde(default)]
    department_id: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub(super) struct RecommendedAnswer {
    courses: Vec<dto::RecommendedCourse>,
}

impl Tool for Recommended {
    const NAME: &'static str = "recommended";
    const DESCRIPTION: &'static str = "Recommended courses for the person's admission year. departmentId defaults to the department chosen in Settings.";
    const EFFECT: Effect = Effect::Read;
    const READS: &'static [W] = &[W::Finder, W::Settings];
    type Args = RecommendedArgs;
    type Output = RecommendedAnswer;

    fn run(cx: &Context<'_>, args: RecommendedArgs) -> Result<RecommendedAnswer, AgentError> {
        let (workspace, _) = cx.view()?;
        let department = args
            .department_id
            .as_deref()
            .or_else(|| {
                workspace
                    .pointer("/settings/departmentId")
                    .and_then(Value::as_str)
            })
            .ok_or("departmentId is required: no department is chosen in Settings")?;
        let admission = workspace
            .pointer("/academicProfile/admissionYear")
            .and_then(Value::as_i64);
        let rows = visible_rows(&workspace, "recommendedCourses")
            .into_iter()
            .filter(|row| {
                // Settings may hold a scoped id such as "school:ee".
                text(row, "departmentId")
                    .is_some_and(|id| id == department || department.ends_with(&format!(":{id}")))
                    && admission.is_none_or(|year| {
                        row.get("admissionYear").and_then(Value::as_i64) == Some(year)
                    })
            })
            .collect::<Vec<_>>();
        Ok(RecommendedAnswer {
            courses: pick_all(&rows)?,
        })
    }
}
