//! Tools that answer from local data and change nothing.
use super::dto::{self, pick, pick_all};
use super::registry::{Effect, Tool};
use super::{
    fallback_term, non_empty, parse_term, text, visible_rows, AgentError, Context,
};
use crate::workspace_changes::WorkspaceChange as W;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    /// The weekly class pattern stops today and no class or exam is held.
    no_classes: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    calendar_notice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    calendar: Option<dto::CalendarDay>,
    /// Classes, exams and quizzes held today, as the app's Today counts
    /// them: cancelled classes are left out (`schedule` keeps them).
    classes: Vec<dto::Occurrence>,
    /// Weekly personal events falling today.
    events: Vec<dto::Occurrence>,
    /// Open assignments not yet due, soonest first.
    assignments: Vec<dto::Assignment>,
    /// Open assignments overdue in the last 14 days.
    overdue: Vec<dto::Assignment>,
    academic_summary: Vec<dto::GpaSummary>,
    /// Stored plans or overrides Core could not apply anywhere.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    problems: Vec<dto::ScheduleProblem>,
}

impl Tool for Today {
    const NAME: &'static str = "today";
    const DESCRIPTION: &'static str = "Today in Japan time, with every personal change applied: the calendar state (holiday, replacement day), the classes, exams and quizzes actually held, weekly personal events falling today, open assignments not yet due, open assignments overdue in the last 14 days, and the GPA summary.";
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
        let (workspace, school) = cx.view()?;
        let now = cx.now;
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
        let agenda = crate::schedule::calendar_between(&workspace, &school, &iso, &iso)?
            .into_iter()
            .next()
            .ok_or_else(|| AgentError::Compatibility("no agenda for today".into()))?;
        let classes = crate::schedule::classes_on(&agenda.occurrences);
        let day = dto::Day::from_agenda(agenda)?;
        Ok(TodayAnswer {
            now: now.to_rfc3339(),
            no_classes: day.no_classes,
            calendar_notice: day.calendar_notice,
            calendar: day.calendar,
            classes: classes.into_iter().map(dto::Occurrence::from).collect(),
            events: day
                .occurrences
                .into_iter()
                .filter(|row| row.kind == "event")
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
            problems: problems(&workspace),
            date: iso,
            term,
        })
    }
}

fn problems(workspace: &Value) -> Vec<dto::ScheduleProblem> {
    crate::schedule::schedule_problems(workspace)
        .into_iter()
        .map(dto::ScheduleProblem::from)
        .collect()
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
    /// Every date of the range, in order.
    days: Vec<dto::Day>,
    /// Stored plans or overrides Core could not apply. The days are computed
    /// without them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    problems: Vec<dto::ScheduleProblem>,
}

impl Tool for Schedule {
    const NAME: &'static str = "schedule";
    const DESCRIPTION: &'static str = "What actually happens on each date of an inclusive range (at most 366 days), as the app's calendar shows it: classes after holidays, replacement days, cancellations, moved or edited sessions and the person's exam and quiz marks; published exams; and weekly personal events. Cancelled classes stay in the answer, marked cancelled. For the weekly templates of a quarter use `courses`.";
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
        let (workspace, school) = cx.view()?;
        let days = crate::schedule::calendar_between(
            &workspace,
            &school,
            &from.to_string(),
            &to.to_string(),
        )?;
        Ok(ScheduleAnswer {
            from: from.to_string(),
            to: to.to_string(),
            days: days
                .into_iter()
                .map(dto::Day::from_agenda)
                .collect::<Result<_, _>>()?,
            problems: problems(&workspace),
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
    /// The quarter's weekly personal events.
    events: Vec<dto::Event>,
}

impl Tool for Courses {
    const NAME: &'static str = "courses";
    const DESCRIPTION: &'static str = "The weekly templates of one academic quarter: registered or planned courses with their meetings, and weekly personal events. A template does not say what is held on a date; `schedule` does.";
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
    type Args = QuarterArgs;
    type Output = CoursesAnswer;

    fn run(cx: &Context<'_>, args: QuarterArgs) -> Result<CoursesAnswer, AgentError> {
        let term = format!("{}-{}", args.year, args.quarter.as_str());
        parse_term(&term)?;
        let (workspace, _) = cx.view()?;
        let events = visible_rows(&workspace, "events")
            .into_iter()
            .filter(|row| text(row, "termId") == Some(term.as_str()))
            .collect::<Vec<_>>();
        Ok(CoursesAnswer {
            courses: pick_all(&crate::schedule::scheduled_courses(&workspace, &term))?,
            events: pick_all(&events)?,
        })
    }
}

pub(super) struct Exams;

#[derive(Serialize, JsonSchema)]
pub(super) struct ExamsAnswer {
    exams: Vec<dto::Exam>,
}

impl Tool for Exams {
    const NAME: &'static str = "exams";
    const DESCRIPTION: &'static str = "Every exam and quiz of one academic quarter, from the unified exam list: the exam timetable with the person's corrections and links, and the class sessions the person marked as an exam or quiz. Entries without a time or a course are included.";
    const EFFECT: Effect = Effect::Read;
    const READS: &'static [W] = &[
        W::Courses,
        W::Registrations,
        W::Finder,
        W::Calendar,
        W::Exams,
        W::Plans,
        W::Subscriptions,
        W::Settings,
    ];
    type Args = QuarterArgs;
    type Output = ExamsAnswer;

    fn run(cx: &Context<'_>, args: QuarterArgs) -> Result<ExamsAnswer, AgentError> {
        let term = format!("{}-{}", args.year, args.quarter.as_str());
        parse_term(&term)?;
        let (workspace, school) = cx.view()?;
        Ok(ExamsAnswer {
            exams: crate::schedule::exam_views(&workspace, &school, &term)
                .into_iter()
                .map(dto::Exam::from)
                .collect(),
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
