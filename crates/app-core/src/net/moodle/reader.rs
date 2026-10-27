//! Per-key LMS reads over one transport, opened once per pull. A key read
//! returns the whole key or an error naming the part that failed; there is no
//! partial key. Each result passes the `adpt_isct::lms` trust boundary
//! for its key.
use super::{
    ajax::{self, CookieTransport},
    envelope::has_warnings,
    mapping::{
        course_row, course_year, dashboard_timetable, month_deadlines, normalize_event, BodyBudget,
        ACTION_EVENT_KEYS, UPCOMING_EVENT_KEYS,
    },
    rest::{self, TokenTransport},
    Dashboard, KeyError, MoodleClient, MoodleError, MoodleFunction,
};
use adpt_isct::lms::{
    id_of, validate_assign, validate_course, validate_deadlines, validate_enrolment,
    validate_history, LmsAssignStates, LmsCourseContent, LmsDeadlines, LmsEnrolment,
    LmsHistoryMonth, StoredModule,
};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};

/// Requests in flight per key read; the host policy spaces them.
pub(super) const WORKERS: usize = 4;
/// Assignment and quiz states read per course.
pub(super) const MAX_STATES: usize = 200;
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

fn entry_mut<'v>(state: &'v mut Value, list: &str, id: u64) -> Option<&'v mut Value> {
    state
        .get_mut(list)
        .and_then(Value::as_array_mut)
        .and_then(|entries| {
            entries
                .iter_mut()
                .find(|entry| id_of(entry.get("id")) == Some(id))
        })
}

/// Writes `key` onto the entry `id` of the outline's `list` (`cm`, `section`).
pub(super) fn set_detail(state: &mut Value, list: &str, id: u64, key: &str, value: Value) {
    if let Some(entry) = entry_mut(state, list, id) {
        entry[key] = value;
    }
}

/// Removes `key` from the entry `id`, so the stored value stays.
pub(super) fn remove_detail(state: &mut Value, list: &str, id: u64, key: &str) {
    if let Some(entry) = entry_mut(state, list, id).and_then(Value::as_object_mut) {
        entry.remove(key);
    }
}

/// Reads the LMS keys of one pull. Token transport first; after a token
/// failure the cookie path can serve, the rest of the pull reads with the
/// session cookie.
pub(crate) struct LmsReader<'a> {
    client: &'a MoodleClient,
    transport: Transport<'a>,
    /// The signed-in dashboard once read: the cookie path opens with it and
    /// the timetable grids come from it.
    dashboard: Option<Dashboard>,
    language: String,
}

impl<'a> LmsReader<'a> {
    pub(super) fn new(
        client: &'a MoodleClient,
        transport: Transport<'a>,
        language: String,
        dashboard: Option<Dashboard>,
    ) -> Self {
        Self {
            client,
            transport,
            dashboard,
            language,
        }
    }

    fn on_token(&self) -> bool {
        matches!(self.transport, Transport::Token(_))
    }

    /// Whether this failed read should be read again over the cookie.
    fn falls_back<T>(&self, result: &Result<T, KeyError>) -> bool {
        self.on_token() && matches!(result, Err(error) if error.wants_cookie_fallback())
    }

    /// Moves the rest of the pull to the cookie transport.
    async fn fall_back(&mut self) -> bool {
        match self.client.cookie_transport().await {
            Ok((transport, dashboard)) => {
                log::warn!(target: "app_core::net::moodle", "LMS token transport failed; reading the rest of this pull with the session cookie");
                if !dashboard.config.language.is_empty() {
                    self.language = dashboard.config.language.clone();
                }
                self.transport = Transport::Cookie(transport);
                self.dashboard = Some(dashboard);
                true
            }
            Err(error) => {
                log::warn!(target: "app_core::net::moodle", "LMS cookie transport unavailable ({error})");
                false
            }
        }
    }

    // -- lms.enrolment -------------------------------------------------------

    /// `lms.enrolment/all`, light: enrolled courses and the current
    /// quarter's grid over this pull's transport. Ingestion retains the
    /// stored grids of the other quarters.
    pub(crate) async fn current_enrolment(&mut self) -> Result<LmsEnrolment, KeyError> {
        let courses = self.read_courses().await?;
        let grids = if self.on_token() {
            vec![self.token_timetable().await?]
        } else {
            // With no years asked for, only the dashboard's own grid.
            self.cookie_timetables(&BTreeSet::new()).await?
        };
        self.enrolment(courses, grids)
    }

    /// `lms.enrolment/all`, full: enrolled courses and every offered quarter's
    /// grid of their years, which only the session cookie's dashboard reads.
    pub(crate) async fn full_enrolment(&mut self) -> Result<LmsEnrolment, KeyError> {
        let courses = self.read_courses().await?;
        let years: BTreeSet<i32> = courses.iter().filter_map(course_year).collect();
        let grids = self.cookie_timetables(&years).await?;
        self.enrolment(courses, grids)
    }

    fn enrolment(&self, courses: Vec<Value>, grids: Vec<Value>) -> Result<LmsEnrolment, KeyError> {
        log::info!(target: "app_core::net::moodle", "LMS enrolment read terms={:?}", grids.iter().map(super::grid_term).collect::<Vec<_>>());
        validate_enrolment(
            json!({"language": self.language, "courses": courses, "timetables": grids}),
        )
        .map_err(KeyError::parse)
    }

    /// The token dashboard's grid: the current quarter only.
    async fn token_timetable(&self) -> Result<Value, KeyError> {
        let function = MoodleFunction::DashboardBlocks;
        let data = self
            .transport
            .call(function, json!({"returncontents": true}))
            .await
            .map_err(KeyError::at(function.name()))?;
        if has_warnings(&data) {
            return Err(KeyError::new(function.name(), MoodleError::Partial));
        }
        dashboard_timetable(&data).ok_or_else(|| {
            KeyError::new(
                function.name(),
                MoodleError::Malformed("dashboard timetable"),
            )
        })
    }

    /// The signed-in dashboard's grid and every other quarter's of `years`,
    /// read through the dashboard's timetable filter.
    async fn cookie_timetables(&mut self, years: &BTreeSet<i32>) -> Result<Vec<Value>, KeyError> {
        let dashboard = match self.dashboard.take() {
            Some(dashboard) => dashboard,
            None => self
                .client
                .dashboard()
                .await
                .map_err(|error| KeyError::new("my/", error).on_cookie())?,
        };
        if !dashboard.config.language.is_empty() {
            self.language = dashboard.config.language.clone();
        }
        let grids = self.client.timetables(&dashboard, years).await;
        self.dashboard = Some(dashboard);
        grids.map_err(KeyError::on_cookie)
    }

    async fn read_courses(&self) -> Result<Vec<Value>, KeyError> {
        let function = MoodleFunction::EnrolledCoursesByTimeline;
        let mut courses = Vec::new();
        let mut offset = 0i64;
        for _ in 0..20 {
            let data = self
                .transport
                .call(
                    function,
                    json!({"classification": "all", "limit": 0, "offset": offset, "sort": "fullname"}),
                )
                .await
                .map_err(KeyError::at(function.name()))?;
            let batch = data
                .get("courses")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if courses.len() + batch.len() > MAX_COURSES {
                return Err(KeyError::new(
                    function.name(),
                    MoodleError::Malformed("course limit"),
                ));
            }
            courses.extend(batch.iter().map(course_row));
            match data.get("nextoffset").and_then(Value::as_i64) {
                Some(next) if next > offset && !batch.is_empty() => offset = next,
                _ => return Ok(courses),
            }
        }
        Err(KeyError::new(
            function.name(),
            MoodleError::Malformed("course list did not end"),
        ))
    }

    // -- lms.deadlines -------------------------------------------------------

    /// `lms.deadlines/all`: every open-work event and the upcoming view.
    pub(crate) async fn deadlines(&mut self) -> Result<LmsDeadlines, KeyError> {
        let mut raw = self.read_deadlines().await;
        if self.falls_back(&raw) && self.fall_back().await {
            raw = self.read_deadlines().await;
        }
        validate_deadlines(raw?).map_err(KeyError::parse)
    }

    async fn read_deadlines(&self) -> Result<LmsDeadlines, KeyError> {
        let action_events = self.read_action_events().await?;
        let function = MoodleFunction::CalendarUpcomingView;
        let view = self
            .transport
            .call(function, json!({"courseid": 1, "categoryid": 0}))
            .await
            .map_err(KeyError::at(function.name()))?;
        let dated_events = view
            .get("events")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(MAX_EVENTS)
            .map(|event| normalize_event(event, &UPCOMING_EVENT_KEYS))
            .collect();
        Ok(LmsDeadlines {
            action_events,
            dated_events,
        })
    }

    async fn read_action_events(&self) -> Result<Vec<Value>, KeyError> {
        let function = MoodleFunction::ActionEventsByTimesort;
        let mut events = Vec::new();
        let mut after: Option<i64> = None;
        for _ in 0..100 {
            let mut args = json!({"timesortfrom": 0, "limitnum": 50});
            if let Some(after) = after {
                args["aftereventid"] = after.into();
            }
            let data = self
                .transport
                .call(function, args)
                .await
                .map_err(KeyError::at(function.name()))?;
            let batch = data
                .get("events")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if events.len() + batch.len() > MAX_EVENTS {
                return Err(KeyError::new(
                    function.name(),
                    MoodleError::Malformed("event limit"),
                ));
            }
            events.extend(
                batch
                    .iter()
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
                _ => return Ok(events),
            }
        }
        Err(KeyError::new(
            function.name(),
            MoodleError::Malformed("event list did not end"),
        ))
    }

    // -- lms.course ----------------------------------------------------------

    /// `lms.course/{id}`: the whole outline, with details of the modules
    /// that changed since `stored` (all of them when nothing is stored).
    pub(crate) async fn course(
        &mut self,
        id: u64,
        stored: &HashMap<u64, StoredModule>,
    ) -> Result<LmsCourseContent, KeyError> {
        let mut state = self.read_course(id, stored).await;
        if self.falls_back(&state) && self.fall_back().await {
            state = self.read_course(id, stored).await;
        }
        validate_course(id, state?).map_err(KeyError::parse)
    }

    async fn read_course(
        &self,
        id: u64,
        stored: &HashMap<u64, StoredModule>,
    ) -> Result<Value, KeyError> {
        // Teacher text kept per course read.
        let budget = BodyBudget::new();
        match &self.transport {
            Transport::Token(transport) => rest::read_course(transport, id, stored, &budget).await,
            Transport::Cookie(transport) => ajax::read_course(transport, id, stored, &budget).await,
        }
    }

    // -- lms.assign ----------------------------------------------------------

    /// `lms.assign/{id}`: one's own state of each assignment and quiz.
    pub(crate) async fn assign(&mut self, id: u64) -> Result<LmsAssignStates, KeyError> {
        let mut rows = self.read_assign(id).await;
        if self.falls_back(&rows) && self.fall_back().await {
            rows = self.read_assign(id).await;
        }
        validate_assign(id, rows?).map_err(KeyError::parse)
    }

    async fn read_assign(&self, id: u64) -> Result<Vec<Value>, KeyError> {
        match &self.transport {
            Transport::Token(transport) => rest::read_assign(transport, id).await,
            Transport::Cookie(transport) => ajax::read_assign(transport, id).await,
        }
    }

    // -- lms.history ---------------------------------------------------------

    /// `lms.history/{YYYY-MM}`: the deadlines in one month's calendar. The
    /// upcoming view and open work forget finished work; past due and close
    /// events still carry its deadlines.
    pub(crate) async fn history(
        &mut self,
        year: i32,
        month: u32,
    ) -> Result<LmsHistoryMonth, KeyError> {
        let mut events = self.read_month(year, month).await;
        if self.falls_back(&events) && self.fall_back().await {
            events = self.read_month(year, month).await;
        }
        validate_history(&format!("{year:04}-{month:02}"), events?).map_err(KeyError::parse)
    }

    async fn read_month(&self, year: i32, month: u32) -> Result<Vec<Value>, KeyError> {
        let function = MoodleFunction::CalendarMonthlyView;
        let view = self
            .transport
            .call(
                function,
                json!({"year": year, "month": month, "courseid": 1, "categoryid": 0, "day": 1}),
            )
            .await
            .map_err(KeyError::at(function.name()))?;
        Ok(month_deadlines(&view))
    }
}
