//! One LMS pull over either transport: the order and bounds of the former
//! injected extractor, producing its payload shape.
use super::{
    ajax::{self, CookieTransport},
    mapping::{
        course_row, course_term, course_title, course_year, month_deadlines, months_to_read,
        normalize_event, select_courses, BodyBudget, ModuleInfo, ACTION_EVENT_KEYS,
        UPCOMING_EVENT_KEYS,
    },
    part,
    rest::{self, TokenTransport},
    Dashboard, LmsScope, MoodleClient, MoodleError, MoodleFunction,
};
use crate::{
    lifecycle::SchoolSite,
    school_browser::SchoolPullProgress,
    school_parser::lms::{id_of, LMS_WWWROOT},
};
use chrono::{FixedOffset, Utc};
use futures_util::stream::{self, StreamExt};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

/// Requests in flight per pull; the host policy spaces them.
pub(super) const WORKERS: usize = 4;
const MAX_COURSES: usize = 500;
const MAX_EVENTS: usize = 5_000;

pub(super) enum Transport<'a> {
    Token(TokenTransport<'a>),
    Cookie(CookieTransport<'a>),
}

impl Transport<'_> {
    pub(super) async fn call(
        &self,
        function: MoodleFunction,
        args: Value,
    ) -> Result<Value, MoodleError> {
        match self {
            Self::Token(transport) => transport.call(function, &args).await,
            Self::Cookie(transport) => transport.call(function, &args).await,
        }
    }
}

/// One selected course and what was read of it.
pub(super) struct CourseRead {
    pub(super) id: u64,
    pub(super) title: String,
    pub(super) term: String,
    pub(super) state: Option<Value>,
    pub(super) error: Option<String>,
    /// Token transport only: module instance ids and listed files.
    pub(super) modules: HashMap<u64, ModuleInfo>,
}

impl CourseRead {
    /// The stored state's `cm` entries.
    pub(super) fn cms(&self) -> Vec<Value> {
        self.state
            .as_ref()
            .and_then(|state| state.get("cm"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    }

    /// Writes a detail onto the `cm` entry `cmid`.
    pub(super) fn set_cm(&mut self, cmid: u64, key: &str, value: Value) {
        if let Some(state) = self.state.as_mut() {
            set_detail(state, "cm", cmid, key, value);
        }
    }
}

/// Writes `key` onto the entry `id` of the state's `list` (`cm`, `section`).
pub(super) fn set_detail(state: &mut Value, list: &str, id: u64, key: &str, value: Value) {
    if let Some(entry) = state
        .get_mut(list)
        .and_then(Value::as_array_mut)
        .and_then(|entries| {
            entries
                .iter_mut()
                .find(|entry| id_of(entry.get("id")) == Some(id))
        })
    {
        entry[key] = value;
    }
}

pub(crate) struct Output {
    pub(crate) payload: Value,
    pub(crate) missing: Vec<&'static str>,
}

pub(super) struct Pull<'a> {
    pub(super) client: &'a MoodleClient,
    pub(super) transport: Transport<'a>,
    pub(super) scope: LmsScope,
    pub(super) course: Option<u64>,
    progress: &'a (dyn Fn(SchoolPullProgress) + Send + Sync),
    pub(super) budget: BodyBudget,
    missing: Mutex<BTreeSet<&'static str>>,
    language: String,
    dashboard: Option<Dashboard>,
}

fn truncate(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

impl<'a> Pull<'a> {
    pub(super) fn new(
        client: &'a MoodleClient,
        transport: Transport<'a>,
        scope: LmsScope,
        course: Option<u64>,
        progress: &'a (dyn Fn(SchoolPullProgress) + Send + Sync),
        language: String,
        dashboard: Option<Dashboard>,
    ) -> Self {
        Self {
            client,
            transport,
            scope,
            course,
            progress,
            budget: BodyBudget::new(),
            missing: Mutex::new(BTreeSet::new()),
            language,
            dashboard,
        }
    }

    pub(super) fn miss(&self, part: &'static str) {
        if let Ok(mut missing) = self.missing.lock() {
            missing.insert(part);
        }
    }

    pub(super) fn report(
        &self,
        phase: &str,
        completed: usize,
        total: usize,
        course: Option<(u64, &str, &str)>,
        item: &str,
    ) {
        let (course_id, course_title, term) = course.unwrap_or((0, "", ""));
        (self.progress)(SchoolPullProgress {
            site: SchoolSite::ScienceLms,
            phase: phase.to_string(),
            completed,
            total,
            course_id,
            course_title: truncate(course_title, 512),
            term: truncate(term, 16),
            item_title: truncate(item, 512),
        });
    }

    pub(super) async fn run(mut self) -> Result<Output, MoodleError> {
        // The dashboard grid only exists without a named course.
        let (timetables, timetable_errors) = if self.course.is_none() {
            self.read_timetables().await
        } else {
            (Vec::new(), Vec::new())
        };
        self.report("catalog", 0, 1, None, "");
        let (courses, courses_complete) = self.read_courses().await?;
        self.report("catalog", 1, 1, None, "");
        // Open work drives the assignment states; it is not optional.
        let action_events = self.read_action_events().await?;
        let mut dated_events = if self.course.is_none() {
            self.read_upcoming().await
        } else {
            Vec::new()
        };
        let current: HashSet<u64> = timetables
            .first()
            .map(|grid| {
                let slots = grid
                    .get("slots")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|slot| id_of(slot.get("courseId")));
                let unscheduled = grid
                    .get("unscheduled")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|id| id_of(Some(id)));
                slots.chain(unscheduled).collect()
            })
            .unwrap_or_default();
        let selected = select_courses(self.scope, self.course, &courses, &current);
        let mut reads = self.read_structures(&selected).await;
        if self.scope == LmsScope::Full && self.course.is_none() {
            let months = self.read_months(&courses).await;
            let room = MAX_EVENTS.saturating_sub(dated_events.len());
            dated_events.extend(months.into_iter().take(room));
        }
        let mut assign_status = Vec::new();
        match &self.transport {
            Transport::Token(transport) => {
                rest::read_details(&self, transport, &mut reads, &mut assign_status).await
            }
            Transport::Cookie(transport) => {
                ajax::read_details(&self, transport, &mut reads, &mut assign_status).await
            }
        }
        let structures: Vec<Value> = reads
            .into_iter()
            .map(|read| {
                json!({
                    "courseId": read.id,
                    "complete": read.state.is_some(),
                    "state": read.state,
                    "error": read.error,
                })
            })
            .collect();
        let payload = json!({
            "status": "ok",
            "sourceUrl": format!("{LMS_WWWROOT}/my/"),
            "language": self.language,
            "coursesComplete": courses_complete,
            "fullScan": self.scope == LmsScope::Full && self.course.is_none(),
            "courses": courses,
            "structures": structures,
            "actionEvents": action_events,
            "datedEvents": dated_events,
            "timetables": timetables,
            "timetableErrors": timetable_errors,
            "assignStatus": assign_status,
        });
        let missing = self
            .missing
            .lock()
            .map(|missing| missing.iter().copied().collect())
            .unwrap_or_default();
        Ok(Output { payload, missing })
    }

    async fn read_timetables(&mut self) -> (Vec<Value>, Vec<String>) {
        let dashboard = match self.dashboard.take() {
            Some(dashboard) => Ok(dashboard),
            None => self.client.dashboard().await,
        };
        let every_quarter = self.scope == LmsScope::Full;
        let (grids, errors) = match &dashboard {
            Ok(dashboard) => {
                if !dashboard.config.language.is_empty() {
                    self.language = dashboard.config.language.clone();
                }
                self.client.timetables(dashboard, every_quarter).await
            }
            // The token path reads everything else without the web session.
            Err(error) => (Vec::new(), vec![format!("timetable-unavailable: {error}")]),
        };
        if grids.is_empty() || !errors.is_empty() {
            self.miss(part::TIMETABLE);
        }
        (grids, errors)
    }

    async fn read_courses(&self) -> Result<(Vec<Value>, bool), MoodleError> {
        let mut courses = Vec::new();
        let mut offset = 0i64;
        for _ in 0..20 {
            let data = self
                .transport
                .call(
                    MoodleFunction::EnrolledCoursesByTimeline,
                    json!({"classification": "all", "limit": 0, "offset": offset, "sort": "fullname"}),
                )
                .await?;
            let batch = data
                .get("courses")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let room = MAX_COURSES - courses.len();
            courses.extend(batch.iter().take(room).map(course_row));
            let next = data.get("nextoffset").and_then(Value::as_i64);
            match next {
                Some(next) if next > offset && !batch.is_empty() => offset = next,
                _ => return Ok((courses, true)),
            }
            if courses.len() >= MAX_COURSES {
                return Err(MoodleError::Api("course-limit".into()));
            }
        }
        Ok((courses, false))
    }

    async fn read_action_events(&self) -> Result<Vec<Value>, MoodleError> {
        let mut events = Vec::new();
        let mut after: Option<i64> = None;
        for _ in 0..100 {
            let mut args = json!({"timesortfrom": 0, "limitnum": 50});
            if let Some(after) = after {
                args["aftereventid"] = after.into();
            }
            let function = match self.course {
                Some(course) => {
                    args["courseid"] = course.into();
                    MoodleFunction::ActionEventsByCourse
                }
                None => MoodleFunction::ActionEventsByTimesort,
            };
            let data = self.transport.call(function, args).await?;
            let batch = data
                .get("events")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let room = MAX_EVENTS - events.len();
            events.extend(
                batch
                    .iter()
                    .take(room)
                    .map(|event| normalize_event(event, &ACTION_EVENT_KEYS)),
            );
            let next = data.get("lastid").and_then(Value::as_i64).or_else(|| {
                batch
                    .last()
                    .and_then(|event| event.get("id"))
                    .and_then(Value::as_i64)
            });
            match next {
                Some(next) if batch.len() >= 50 && Some(next) != after => after = Some(next),
                _ => break,
            }
            if events.len() >= MAX_EVENTS {
                return Err(MoodleError::Api("event-limit".into()));
            }
        }
        Ok(events)
    }

    /// Non-action calendar entries Today uses.
    async fn read_upcoming(&self) -> Vec<Value> {
        match self
            .transport
            .call(
                MoodleFunction::CalendarUpcomingView,
                json!({"courseid": 1, "categoryid": 0}),
            )
            .await
        {
            Ok(view) => view
                .get("events")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(MAX_EVENTS)
                .map(|event| normalize_event(event, &UPCOMING_EVENT_KEYS))
                .collect(),
            Err(_) => {
                self.miss(part::UPCOMING_EVENTS);
                Vec::new()
            }
        }
    }

    async fn read_structures(&self, selected: &[&Value]) -> Vec<CourseRead> {
        let total = selected.len();
        let first = selected.first().map(|course| {
            (
                id_of(course.get("id")).unwrap_or(0),
                course_title(course),
                course_term(course),
            )
        });
        self.report(
            "course",
            0,
            total,
            first
                .as_ref()
                .map(|(id, title, term)| (*id, title.as_str(), term.as_str())),
            "",
        );
        let done = AtomicUsize::new(0);
        let this = self;
        let done_ref = &done;
        stream::iter(selected.iter().copied())
            .map(move |course| async move {
                let id = id_of(course.get("id")).unwrap_or(0);
                let (title, term) = (course_title(course), course_term(course));
                let result = match &this.transport {
                    Transport::Token(transport) => rest::read_structure(this, transport, id).await,
                    Transport::Cookie(transport) => ajax::read_structure(this, transport, id).await,
                };
                let read = match result {
                    Ok((state, modules)) => CourseRead {
                        id,
                        title: title.clone(),
                        term: term.clone(),
                        state: Some(state),
                        error: None,
                        modules,
                    },
                    Err(error) => {
                        this.miss(part::COURSE_STRUCTURE);
                        CourseRead {
                            id,
                            title: title.clone(),
                            term: term.clone(),
                            state: None,
                            error: Some(error.to_string()),
                            modules: HashMap::new(),
                        }
                    }
                };
                let completed = done_ref.fetch_add(1, Ordering::SeqCst) + 1;
                this.report(
                    "course",
                    completed,
                    total,
                    Some((id, title.as_str(), term.as_str())),
                    "",
                );
                read
            })
            .buffer_unordered(WORKERS)
            .collect()
            .await
    }

    /// The upcoming view and open-work events forget finished work, so a
    /// full pull reads each month of the enrolled academic years up to now;
    /// past due/close events still carry the deadlines of finished work.
    async fn read_months(&self, courses: &[Value]) -> Vec<Value> {
        let years: Vec<i32> = courses.iter().filter_map(course_year).collect();
        let today = FixedOffset::east_opt(9 * 3600)
            .map(|offset| Utc::now().with_timezone(&offset).date_naive())
            .unwrap_or_else(|| Utc::now().date_naive());
        let this = self;
        let lists: Vec<Vec<Value>> = stream::iter(months_to_read(&years, today))
            .map(move |(year, month)| async move {
                match this
                    .transport
                    .call(
                        MoodleFunction::CalendarMonthlyView,
                        json!({"year": year, "month": month, "courseid": 1, "categoryid": 0, "day": 1}),
                    )
                    .await
                {
                    Ok(view) => month_deadlines(&view),
                    Err(_) => {
                        // A month that cannot be read keeps the stored deadlines.
                        this.miss(part::MONTH_CALENDAR);
                        Vec::new()
                    }
                }
            })
            .buffer_unordered(WORKERS)
            .collect()
            .await;
        lists.into_iter().flatten().collect()
    }
}
