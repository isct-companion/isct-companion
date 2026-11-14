//! Per-key LMS reads, each over the transport its key names. A key read
//! returns the whole key or an error naming the part that failed; there is no
//! partial key. Each result passes the `adpt_isct::lms` trust boundary
//! for its key. Every read runs on one attempt of the LMS site session.
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
use crate::school_session::Attempt;
use adpt_isct::lms::{
    id_of, validate_assign, validate_course, validate_deadlines, validate_enrolment,
    validate_history, LmsAssignStates, LmsCourseContent, LmsDeadlines, LmsEnrolment,
    LmsHistoryMonth, StoredModule,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

/// Requests in flight per key read; the host policy spaces them.
pub(super) const WORKERS: usize = 4;
/// Assignment and quiz states read per course.
pub(super) const MAX_STATES: usize = 200;
const MAX_COURSES: usize = 500;
const MAX_EVENTS: usize = 5_000;

pub(super) enum Transport {
    Token(TokenTransport),
    Cookie(CookieTransport),
}

impl Transport {
    pub(super) async fn call(
        &self,
        attempt: &Attempt,
        function: MoodleFunction,
        args: Value,
    ) -> Result<Value, MoodleError> {
        match self {
            Self::Token(transport) => transport.call(attempt, function, &args).await,
            Self::Cookie(transport) => transport.call(attempt, function, &args).await,
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

/// Which transport a key reads with (`plan::school::via`, from the
/// key's credential path and the token's facts).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Via {
    /// The token; this key alone reads again with the session cookie when
    /// the token path cannot serve it.
    Token,
    /// The session cookie only.
    Cookie,
}

/// Reads one key over `$via`, and over the cookie once more when the token
/// path answered in a way the cookie may serve. A macro rather than a
/// closure: each read borrows the transport and its own arguments.
macro_rules! over {
    ($reader:ident, $attempt:ident, $via:expr, |$transport:ident| $read:expr) => {{
        let via = $reader.ready($attempt, $via).await?;
        let first = {
            let $transport = $reader.transport(via)?;
            $read.await
        };
        if via == Via::Token && falls_back(&first) {
            log::warn!(target: "app_core::net::moodle", "LMS token path cannot serve this key; reading it with the session cookie");
            $reader.cookie($attempt).await?;
            let $transport = $reader.transport(Via::Cookie)?;
            $read.await
        } else {
            first
        }
    }};
}

/// Whether this read over the token should be read again over the cookie.
fn falls_back<T>(result: &Result<T, KeyError>) -> bool {
    matches!(result, Err(error) if error.wants_cookie_fallback())
}

/// Reads the LMS keys a worker leases. Each key names its transport
/// (`Via`); both are opened lazily and kept across keys, so one key's
/// fallback to the cookie never moves another key off the token.
pub(crate) struct LmsReader {
    client: Arc<MoodleClient>,
    /// The token transport, proven once with `site_info`.
    token: Option<Transport>,
    /// The cookie transport and the site instance its sesskey belongs to.
    cookie: Option<(u64, Transport)>,
    /// The signed-in dashboard once read: the cookie path opens with it and
    /// the timetable grids come from it.
    dashboard: Option<Dashboard>,
    language: String,
}

impl LmsReader {
    pub(crate) fn new(client: Arc<MoodleClient>) -> Self {
        Self {
            client,
            token: None,
            cookie: None,
            dashboard: None,
            language: String::new(),
        }
    }

    /// Gets `via`'s transport ready. A token path that cannot be opened in
    /// a way the cookie may serve gives the cookie, for this key.
    async fn ready(&mut self, attempt: &Attempt, via: Via) -> Result<Via, KeyError> {
        if via == Via::Token {
            if let Some(Transport::Token(transport)) = &self.token {
                transport.next_key();
                return Ok(Via::Token);
            }
            match rest::TokenTransport::open(&self.client, attempt).await {
                Ok((transport, language)) => {
                    self.language = language;
                    self.token = Some(Transport::Token(transport));
                    return Ok(Via::Token);
                }
                Err(error) if error.wants_cookie_fallback() => {
                    log::warn!(target: "app_core::net::moodle", "LMS token transport unavailable ({error}); reading this key with the session cookie");
                }
                Err(error) => return Err(KeyError::new(MoodleFunction::SiteInfo.name(), error)),
            }
        }
        self.cookie(attempt).await?;
        Ok(Via::Cookie)
    }

    /// The cookie transport of this attempt's site instance, over its
    /// dashboard.
    async fn cookie(&mut self, attempt: &Attempt) -> Result<(), KeyError> {
        let instance = attempt
            .instance()
            .await
            .map_err(|error| KeyError::new("my/", error.into()).on_cookie())?;
        if matches!(&self.cookie, Some((held, _)) if *held == instance) {
            return Ok(());
        }
        let dashboard = self.dashboard(attempt, instance).await?;
        let transport = ajax::CookieTransport::new(&dashboard)
            .map_err(|error| KeyError::new("my/", error).on_cookie())?;
        self.cookie = Some((instance, Transport::Cookie(transport)));
        self.dashboard = Some(dashboard);
        Ok(())
    }

    /// The dashboard of `instance`: the one read, else read now.
    async fn dashboard(&mut self, attempt: &Attempt, instance: u64) -> Result<Dashboard, KeyError> {
        let dashboard = match self
            .dashboard
            .take()
            .filter(|dashboard| dashboard.instance == instance)
        {
            Some(dashboard) => dashboard,
            None => MoodleClient::load_dashboard(attempt)
                .await
                .map_err(|error| KeyError::new("my/", error).on_cookie())?,
        };
        if !dashboard.config.language.is_empty() {
            self.language = dashboard.config.language.clone();
        }
        Ok(dashboard)
    }

    fn transport(&self, via: Via) -> Result<&Transport, KeyError> {
        match via {
            Via::Token => self.token.as_ref(),
            Via::Cookie => self.cookie.as_ref().map(|(_, transport)| transport),
        }
        .ok_or_else(|| KeyError::new("open", MoodleError::Malformed("reader")))
    }

    // -- lms.enrolment -------------------------------------------------------

    /// `lms.enrolment/all`. Light: enrolled courses and the current
    /// quarter's grid (ingestion retains the stored grids of the other
    /// quarters). Full: every offered quarter's grid of the courses' years,
    /// which only the session cookie's dashboard reads, so the whole key
    /// reads over the cookie.
    pub(crate) async fn enrolment(
        &mut self,
        attempt: &Attempt,
        via: Via,
        full: bool,
    ) -> Result<LmsEnrolment, KeyError> {
        if full {
            self.cookie(attempt).await?;
            let courses = read_courses(self.transport(Via::Cookie)?, attempt).await?;
            let years: BTreeSet<i32> = courses.iter().filter_map(course_year).collect();
            let grids = self.cookie_timetables(attempt, &years).await?;
            return self.validated(courses, grids);
        }
        if self.ready(attempt, via).await? == Via::Token {
            let transport = self.transport(Via::Token)?;
            let read = async {
                let courses = read_courses(transport, attempt).await?;
                let grid = token_timetable(transport, attempt).await?;
                Ok::<_, KeyError>((courses, vec![grid]))
            }
            .await;
            match read {
                Ok((courses, grids)) => return self.validated(courses, grids),
                Err(error) if error.wants_cookie_fallback() => {
                    log::warn!(target: "app_core::net::moodle", "LMS token path cannot serve the enrolment ({error}); reading it with the session cookie");
                    self.cookie(attempt).await?;
                }
                Err(error) => return Err(error),
            }
        }
        let courses = read_courses(self.transport(Via::Cookie)?, attempt).await?;
        // With no years asked for, only the dashboard's own grid.
        let grids = self.cookie_timetables(attempt, &BTreeSet::new()).await?;
        self.validated(courses, grids)
    }

    fn validated(&self, courses: Vec<Value>, grids: Vec<Value>) -> Result<LmsEnrolment, KeyError> {
        log::info!(target: "app_core::net::moodle", "LMS enrolment read terms={:?}", grids.iter().map(super::grid_term).collect::<Vec<_>>());
        validate_enrolment(
            json!({"language": self.language, "courses": courses, "timetables": grids}),
        )
        .map_err(KeyError::parse)
    }

    /// The signed-in dashboard's grid and every other quarter's of `years`,
    /// read through the dashboard's timetable filter.
    async fn cookie_timetables(
        &mut self,
        attempt: &Attempt,
        years: &BTreeSet<i32>,
    ) -> Result<Vec<Value>, KeyError> {
        // A dashboard of another site instance is not this session's page.
        let instance = attempt
            .instance()
            .await
            .map_err(|error| KeyError::new("my/", error.into()).on_cookie())?;
        let dashboard = self.dashboard(attempt, instance).await?;
        let grids = self.client.timetables(attempt, &dashboard, years).await;
        self.dashboard = Some(dashboard);
        grids.map_err(KeyError::on_cookie)
    }

    // -- the other keys ------------------------------------------------------

    /// `lms.deadlines/all`: every open-work event and the upcoming view.
    pub(crate) async fn deadlines(
        &mut self,
        attempt: &Attempt,
        via: Via,
    ) -> Result<LmsDeadlines, KeyError> {
        let raw = over!(self, attempt, via, |transport| read_deadlines(
            transport, attempt
        ));
        validate_deadlines(raw?).map_err(KeyError::parse)
    }

    /// `lms.course/{id}`: the whole outline, with details of the modules
    /// that changed since `stored` (all of them when nothing is stored).
    pub(crate) async fn course(
        &mut self,
        attempt: &Attempt,
        via: Via,
        id: u64,
        stored: &HashMap<u64, StoredModule>,
    ) -> Result<LmsCourseContent, KeyError> {
        let state = over!(self, attempt, via, |transport| read_course(
            transport, attempt, id, stored
        ));
        validate_course(id, state?).map_err(KeyError::parse)
    }

    /// `lms.assign/{id}`: one's own state of each assignment and quiz.
    pub(crate) async fn assign(
        &mut self,
        attempt: &Attempt,
        via: Via,
        id: u64,
    ) -> Result<LmsAssignStates, KeyError> {
        let rows = over!(self, attempt, via, |transport| read_assign(
            transport, attempt, id
        ));
        validate_assign(id, rows?).map_err(KeyError::parse)
    }

    /// `lms.history/{YYYY-MM}`: the deadlines in one month's calendar. The
    /// upcoming view and open work forget finished work; past due and close
    /// events still carry its deadlines.
    pub(crate) async fn history(
        &mut self,
        attempt: &Attempt,
        via: Via,
        year: i32,
        month: u32,
    ) -> Result<LmsHistoryMonth, KeyError> {
        let events = over!(self, attempt, via, |transport| read_month(
            transport, attempt, year, month
        ));
        validate_history(&format!("{year:04}-{month:02}"), events?).map_err(KeyError::parse)
    }
}

/// The token dashboard's grid: the current quarter only.
async fn token_timetable(transport: &Transport, attempt: &Attempt) -> Result<Value, KeyError> {
    let function = MoodleFunction::DashboardBlocks;
    let data = transport
        .call(attempt, function, json!({"returncontents": true}))
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

async fn read_courses(transport: &Transport, attempt: &Attempt) -> Result<Vec<Value>, KeyError> {
    let function = MoodleFunction::EnrolledCoursesByTimeline;
    let mut courses = Vec::new();
    let mut offset = 0i64;
    for _ in 0..20 {
        let data = transport
            .call(
                attempt,
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

async fn read_deadlines(
    transport: &Transport,
    attempt: &Attempt,
) -> Result<LmsDeadlines, KeyError> {
    let action_events = read_action_events(transport, attempt).await?;
    let function = MoodleFunction::CalendarUpcomingView;
    let view = transport
        .call(attempt, function, json!({"courseid": 1, "categoryid": 0}))
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

async fn read_action_events(
    transport: &Transport,
    attempt: &Attempt,
) -> Result<Vec<Value>, KeyError> {
    let function = MoodleFunction::ActionEventsByTimesort;
    let mut events = Vec::new();
    let mut after: Option<i64> = None;
    for _ in 0..100 {
        let mut args = json!({"timesortfrom": 0, "limitnum": 50});
        if let Some(after) = after {
            args["aftereventid"] = after.into();
        }
        let data = transport
            .call(attempt, function, args)
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

async fn read_course(
    transport: &Transport,
    attempt: &Attempt,
    id: u64,
    stored: &HashMap<u64, StoredModule>,
) -> Result<Value, KeyError> {
    // Teacher text kept per course read.
    let budget = BodyBudget::new();
    match transport {
        Transport::Token(transport) => {
            rest::read_course(transport, attempt, id, stored, &budget).await
        }
        Transport::Cookie(transport) => {
            ajax::read_course(transport, attempt, id, stored, &budget).await
        }
    }
}

async fn read_assign(
    transport: &Transport,
    attempt: &Attempt,
    id: u64,
) -> Result<Vec<Value>, KeyError> {
    match transport {
        Transport::Token(transport) => rest::read_assign(transport, attempt, id).await,
        Transport::Cookie(transport) => ajax::read_assign(transport, attempt, id).await,
    }
}

async fn read_month(
    transport: &Transport,
    attempt: &Attempt,
    year: i32,
    month: u32,
) -> Result<Vec<Value>, KeyError> {
    let function = MoodleFunction::CalendarMonthlyView;
    let view = transport
        .call(
            attempt,
            function,
            json!({"year": year, "month": month, "courseid": 1, "categoryid": 0, "day": 1}),
        )
        .await
        .map_err(KeyError::at(function.name()))?;
    Ok(month_deadlines(&view))
}
