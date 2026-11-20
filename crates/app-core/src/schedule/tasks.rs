use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Duration, FixedOffset};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::{
    array, parse_timestamp, placement_terms, scheduled_courses, text, visible, ScheduleError,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum DeadlineState {
    Remaining,
    Overdue,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum CountdownUnit {
    Days,
    Hours,
    Minutes,
    Seconds,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CountdownComponent {
    pub unit: CountdownUnit,
    pub count: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TaskDeadlineView {
    pub at: String,
    pub state: DeadlineState,
    pub units: Vec<CountdownComponent>,
    pub countdown_tone: String,
    pub next_change_at: Option<String>,
}

/// One quarter's assignments, grouped the way every list shows them: what is
/// pinned, due this week, due later, without a deadline, then past deadlines
/// newest first. Upcoming groups are ordered by deadline.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AssignmentGroups {
    pub pinned_ids: Vec<String>,
    pub due_this_week_ids: Vec<String>,
    pub later_ids: Vec<String>,
    pub undated_ids: Vec<String>,
    /// Past deadlines shown at once: the recent window of the current
    /// quarter, or every past deadline of any other quarter.
    pub past_recent_ids: Vec<String>,
    /// Older past deadlines of the current quarter, back to its start; a list
    /// folds these until asked.
    pub past_earlier_ids: Vec<String>,
    pub hidden_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TermTaskView {
    /// Unsubmitted work with a future deadline.
    pub pending_ids: Vec<String>,
    /// What is left to do: unsubmitted upcoming or pinned work, and work
    /// missed during the last week.
    pub today: AssignmentGroups,
    /// Every assignment of the quarter, whatever its submission state.
    pub record: AssignmentGroups,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TaskView {
    pub deadlines: BTreeMap<String, TaskDeadlineView>,
    /// Visible assignments Core treats as submitted (`submission` submitted
    /// or graded, or LMS `submissionStatus` submitted). Shells render this
    /// instead of applying their own rule.
    pub submitted_ids: Vec<String>,
    pub states: BTreeMap<String, AssignmentState>,
    pub hand_in_availability: BTreeMap<String, HandInAvailability>,
    pub nearest_by_course: BTreeMap<String, String>,
    pub by_term: BTreeMap<String, TermTaskView>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum AssignmentState {
    Open,
    Overdue,
    Closed,
    Submitted,
    Graded,
}

impl AssignmentState {
    pub fn is_submitted(self) -> bool {
        matches!(self, Self::Submitted | Self::Graded)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum HandInAvailability {
    Unavailable,
    OpenLms,
    PrepareDraft,
}

pub fn assignment_state(row: &Value, now: DateTime<FixedOffset>) -> AssignmentState {
    if let Some(state) = assignment_completion(row) {
        return state;
    }
    if text(row, "dueAt")
        .and_then(|at| parse_timestamp(at).ok())
        .is_some_and(|due| due <= now)
    {
        if row.get("acceptsSubmission").and_then(Value::as_bool) == Some(true) {
            AssignmentState::Overdue
        } else {
            AssignmentState::Closed
        }
    } else {
        AssignmentState::Open
    }
}

pub(crate) fn hand_in_availability(row: &Value, state: AssignmentState) -> HandInAvailability {
    match (text(row, "sourceUrl").filter(|url| !url.is_empty()), state) {
        (None, _) => HandInAvailability::Unavailable,
        (Some(_), AssignmentState::Open | AssignmentState::Overdue) => {
            HandInAvailability::PrepareDraft
        }
        (Some(_), _) => HandInAvailability::OpenLms,
    }
}

fn assignment_completion(row: &Value) -> Option<AssignmentState> {
    match text(row, "submission") {
        Some("graded") => Some(AssignmentState::Graded),
        Some("submitted") => Some(AssignmentState::Submitted),
        _ if text(row, "submissionStatus") == Some("submitted") => Some(AssignmentState::Submitted),
        _ => None,
    }
}

pub(crate) fn assignment_submitted(row: &Value) -> bool {
    assignment_completion(row).is_some()
}

/// Missed work stays on Today for a week.
const TODAY_MISSED_DAYS: i64 = 7;
/// The current quarter's record shows the last two weeks of past work.
const RECORD_RECENT_DAYS: i64 = 14;

/// Instants at which Today can move or remove an open assignment.
pub(super) fn assignment_boundaries(
    workspace: &Value,
) -> impl Iterator<Item = DateTime<FixedOffset>> + '_ {
    array(workspace, "assignments")
        .iter()
        .filter(|row| visible(row))
        .filter_map(|row| {
            let due = parse_timestamp(text(row, "dueAt")?).ok()?;
            // Missed work leaves the recent list just after the grace date.
            Some([
                due,
                due + Duration::days(TODAY_MISSED_DAYS) + Duration::milliseconds(1),
            ])
        })
        .flatten()
}

pub fn task_view(workspace: &Value, now: DateTime<FixedOffset>) -> Result<TaskView, ScheduleError> {
    let mut parsed = BTreeMap::new();
    let mut deadlines = BTreeMap::new();
    let mut states = BTreeMap::new();
    let mut availability = BTreeMap::new();
    for row in array(workspace, "assignments")
        .iter()
        .filter(|row| visible(row))
    {
        let Some(id) = text(row, "id") else { continue };
        let state = assignment_state(row, now);
        states.insert(id.to_owned(), state);
        availability.insert(id.to_owned(), hand_in_availability(row, state));
        if let Some(due) = text(row, "dueAt").and_then(|raw| parse_timestamp(raw).ok()) {
            parsed.insert(id.to_owned(), due);
            deadlines.insert(id.to_owned(), deadline_view(due, now));
        }
    }
    let submitted_ids = states
        .iter()
        .filter(|(_, state)| state.is_submitted())
        .map(|(id, _)| id.clone())
        .collect();
    let mut nearest_by_course = BTreeMap::<String, (DateTime<FixedOffset>, String)>::new();
    for row in array(workspace, "assignments")
        .iter()
        .filter(|row| visible(row) && row.get("hidden").and_then(Value::as_bool) != Some(true))
    {
        let (Some(id), Some(course), Some(due)) = (
            text(row, "id"),
            text(row, "courseId"),
            text(row, "id").and_then(|id| parsed.get(id)),
        ) else {
            continue;
        };
        if *due <= now || states.get(id).is_some_and(|state| state.is_submitted()) {
            continue;
        }
        if nearest_by_course
            .get(course)
            .is_none_or(|(current, _)| due < current)
        {
            nearest_by_course.insert(course.into(), (*due, id.into()));
        }
    }
    let mut terms: HashSet<String> = placement_terms(workspace).map(str::to_owned).collect();
    for key in ["currentTerm", "selectedTerm"] {
        if let Some(term) = text(workspace, key) {
            terms.insert(term.to_owned());
        }
    }
    let current_term = text(workspace, "currentTerm");
    let week = now + Duration::days(7);
    let missed_since = now - Duration::days(TODAY_MISSED_DAYS);
    let recent_since = now - Duration::days(RECORD_RECENT_DAYS);
    let known_courses: HashSet<&str> = array(workspace, "courses")
        .iter()
        .filter(|row| visible(row))
        .filter_map(|row| text(row, "id"))
        .collect();
    let mut by_term = BTreeMap::new();
    for term in terms {
        // A course on several quarters' timetables (3Q–4Q) lists its work in
        // each. Work whose course is unknown joins the current quarter while
        // it is still recent.
        let course_ids: HashSet<String> = scheduled_courses(workspace, &term)
            .iter()
            .filter_map(|row| text(row, "id").map(str::to_owned))
            .collect();
        let current = current_term == Some(term.as_str());
        let mut view = TermTaskView::default();
        let mut upcoming = Vec::new();
        let mut undated = Vec::new();
        let mut past = Vec::new();
        let mut hidden = Vec::new();
        // Pinned work stays on Today until it is submitted or unpinned.
        let mut today_pinned = Vec::new();
        for row in array(workspace, "assignments")
            .iter()
            .filter(|row| visible(row))
        {
            let Some(id) = text(row, "id") else { continue };
            let due = parsed.get(id).copied();
            let belongs = match text(row, "courseId") {
                Some(course) if known_courses.contains(course) => course_ids.contains(course),
                _ => current && due.is_some_and(|due| due >= missed_since),
            };
            if !belongs {
                continue;
            }
            if row.get("hidden").and_then(Value::as_bool) == Some(true) {
                hidden.push((due, id));
                continue;
            }
            let submitted = states.get(id).is_some_and(|state| state.is_submitted());
            let pinned = row.get("pinned").and_then(Value::as_bool) == Some(true);
            if pinned && !submitted {
                today_pinned.push((due, id));
            }
            match due {
                Some(due) if due <= now => past.push((due, id, submitted, pinned)),
                Some(due) => upcoming.push((due, id, submitted, pinned)),
                None => undated.push((id, pinned)),
            }
        }
        upcoming.sort_by_key(|(due, id, ..)| (*due, *id));
        for (due, id, submitted, pinned) in upcoming {
            if !submitted {
                view.pending_ids.push(id.into());
            }
            if pinned {
                view.record.pinned_ids.push(id.into());
                continue;
            }
            let groups: &mut [&mut AssignmentGroups] = if submitted {
                &mut [&mut view.record]
            } else {
                &mut [&mut view.today, &mut view.record]
            };
            for group in groups.iter_mut() {
                if due <= week {
                    group.due_this_week_ids.push(id.into());
                } else {
                    group.later_ids.push(id.into());
                }
            }
        }
        for (id, pinned) in undated {
            if pinned {
                view.record.pinned_ids.push(id.into());
            } else {
                view.record.undated_ids.push(id.into());
            }
        }
        today_pinned.sort_by_key(|(due, id)| (due.is_none(), *due, *id));
        view.today.pinned_ids = today_pinned.into_iter().map(|(_, id)| id.into()).collect();
        past.sort_by_key(|(due, id, ..)| std::cmp::Reverse((*due, *id)));
        for (due, id, submitted, pinned) in past {
            if !submitted && !pinned && due >= missed_since {
                view.today.past_recent_ids.push(id.into());
            }
            if !current || due >= recent_since {
                view.record.past_recent_ids.push(id.into());
            } else {
                view.record.past_earlier_ids.push(id.into());
            }
        }
        hidden.sort_by_key(|(due, id)| (due.is_none(), *due, *id));
        view.record.hidden_ids = hidden.into_iter().map(|(_, id)| id.into()).collect();
        by_term.insert(term, view);
    }
    Ok(TaskView {
        deadlines,
        submitted_ids,
        states,
        hand_in_availability: availability,
        nearest_by_course: nearest_by_course
            .into_iter()
            .map(|(k, (_, v))| (k, v))
            .collect(),
        by_term,
    })
}

pub fn deadline_view(due: DateTime<FixedOffset>, now: DateTime<FixedOffset>) -> TaskDeadlineView {
    const UNITS: [(CountdownUnit, i64); 4] = [
        (CountdownUnit::Days, 86_400),
        (CountdownUnit::Hours, 3_600),
        (CountdownUnit::Minutes, 60),
        (CountdownUnit::Seconds, 1),
    ];
    let elapsed = now >= due;
    let seconds = if elapsed { now - due } else { due - now }.num_seconds();
    let largest = UNITS
        .iter()
        .position(|(_, size)| seconds >= *size)
        .unwrap_or(UNITS.len() - 1);
    let (unit, size) = UNITS[largest];
    let count = seconds / size;
    let component = |unit, count| CountdownComponent {
        unit,
        count: u32::try_from(count).unwrap_or(u32::MAX),
    };
    let mut units = vec![component(unit, count)];
    let step = UNITS.get(largest + 1).map_or(1, |(unit, smaller)| {
        let next = (seconds % size) / smaller;
        if next > 0 {
            units.push(component(*unit, next));
        }
        *smaller
    });
    let bucket = seconds / step * step;
    let next = if elapsed {
        due.checked_add_signed(Duration::seconds(bucket + step))
    } else if seconds == 0 {
        Some(due)
    } else {
        // When counting down, the old value includes the exact boundary.
        // Refresh on the first representable UI millisecond after it.
        let boundary = due - Duration::seconds(bucket);
        DateTime::from_timestamp_millis(boundary.timestamp_millis() + 1)
            .map(|at| at.with_timezone(due.offset()))
    };
    let tone = if elapsed {
        "past"
    } else if seconds < 86_400 {
        "urgent"
    } else if seconds < 259_200 {
        "soon"
    } else {
        "normal"
    };
    TaskDeadlineView {
        at: due.to_rfc3339(),
        state: if elapsed {
            DeadlineState::Overdue
        } else {
            DeadlineState::Remaining
        },
        units,
        countdown_tone: tone.into(),
        next_change_at: next.map(|at| at.to_utc().to_rfc3339()),
    }
}
