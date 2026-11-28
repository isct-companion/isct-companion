//! How fresh each key is kept, by the source-division rules (`d/0mzmh8`
//! and the decisions under it, slotledger table t59): which term a key
//! belongs to, the threshold a visible page renews it at, and the shorter
//! values a key takes while its content is moving. The base TTLs are
//! `sync_state::ttl`; what a key's change makes due elsewhere is in the
//! keyspace (`KeyState::due_marks`, `d/azfvr2`). `planner::evaluate` and
//! `planner::next_due` share these rules.
use super::View;
use crate::lms_ingest::{current_lms_term, lms_course_term, StoredLmsCourse};
use crate::sync_state::{self, Dataset, Key, KeyState, Term};
use chrono::{DateTime, Datelike, Duration, Utc};

/// A key whose content changed this recently is on its busy-period values
/// (`d/p26m6s`): registration and grading periods show as changes before
/// any published date does.
const RECENT_CHANGE: Duration = Duration::days(3);

/// Whether `state`'s content changed within [`RECENT_CHANGE`].
pub(crate) fn busy(state: Option<&KeyState>, now: DateTime<Utc>) -> bool {
    state
        .and_then(|state| state.changed_at)
        .is_some_and(|changed| now - changed < RECENT_CHANGE)
}

/// The term `key` belongs to (`d/wtgqyz`): an LMS course by its quarter, a
/// month of LMS history by whether it is over, a catalog department by
/// whether its academic year is over, `None` when nothing can read it any
/// more (a course the stored enrolment list no longer holds).
pub(crate) fn term(
    key: &Key,
    course: Option<&StoredLmsCourse>,
    enrolment_read: bool,
    now: DateTime<Utc>,
) -> Option<Term> {
    match key.dataset {
        Dataset::LmsCourse | Dataset::LmsAssign => match course {
            Some(course) => Some(lms_course_term(course, current_lms_term(now))),
            // Not (or no longer) the person's course: the ingest would refuse
            // its rows. It is wanted again once a list that holds it lands.
            None if enrolment_read => None,
            None => Some(Term::Current),
        },
        Dataset::LmsHistory => {
            let (year, month) = key.key.split_once('-')?;
            let month = (year.parse::<i32>().ok()?, month.parse::<u32>().ok()?);
            let today = crate::schedule::japan_today(now);
            Some(if month < (today.year(), today.month()) {
                Term::Frozen
            } else {
                Term::Current
            })
        }
        Dataset::CatalogOfficial => {
            let year: i32 = key.key.split('/').next()?.parse().ok()?;
            Some(if year < academic_year(now) {
                Term::Frozen
            } else {
                Term::Current
            })
        }
        Dataset::LmsEnrolment
        | Dataset::LmsDeadlines
        | Dataset::KyomuPage
        | Dataset::Reference
        | Dataset::Announcements => Some(Term::Current),
    }
}

/// The academic year `now` falls in: April to March, in Japan.
fn academic_year(now: DateTime<Utc>) -> i32 {
    let today = crate::schedule::japan_today(now);
    today.year() - i32::from(today.month() < 4)
}

/// `sync_state::ttl` with the busy-period values: while the grades or the
/// declared courses change, they are read daily.
pub(crate) fn ttl(key: &Key, term: Term, busy: bool) -> Option<Duration> {
    let base = sync_state::ttl(key, term)?;
    let busy_ttl = match (key.dataset, key.key.as_str()) {
        (Dataset::KyomuPage, "grades" | "declared") if busy => Some(Duration::days(1)),
        _ => None,
    };
    Some(busy_ttl.map_or(base, |busy| busy.min(base)))
}

/// How old `key` may be while `view` shows it under `Rule::Open` before the
/// page renews it. The finder reads by TTL only and never asks.
pub(crate) fn open_threshold(view: &View, key: &Key, busy: bool) -> Duration {
    match (view, key.dataset, key.key.as_str()) {
        // The registered list confirms what LMS already shows: a day,
        // fifteen minutes while it is changing.
        (View::Timetable, Dataset::KyomuPage, "registrations") if busy => Duration::minutes(15),
        (View::Timetable, Dataset::KyomuPage, "registrations") => Duration::days(1),
        (View::Grades, Dataset::KyomuPage, "gpa" | "requirements") => Duration::days(1),
        // One request per course: a moved deadline marks its course due
        // (`KeyState::due_marks`), the rest wait an hour.
        (View::Assignments, Dataset::LmsAssign, _) => Duration::hours(1),
        _ => Duration::minutes(15),
    }
}

#[cfg(test)]
#[path = "../tests/plan__freshness.rs"]
mod tests;
