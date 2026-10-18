use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Duration, FixedOffset};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::{
    array, parse_timestamp, placement_terms, scheduled_courses, text, visible, ScheduleError,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TaskDeadlineView {
    pub at: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub remaining_milliseconds: i64,
    pub label: String,
    pub tone: String,
    pub countdown_label: String,
    pub countdown_tone: String,
    pub overdue: bool,
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
    pub nearest_by_course: BTreeMap<String, String>,
    pub by_term: BTreeMap<String, TermTaskView>,
}

pub(crate) fn assignment_submitted(row: &Value) -> bool {
    matches!(text(row, "submission"), Some("submitted" | "graded"))
        || text(row, "submissionStatus") == Some("submitted")
}

/// Missed work stays on Today for a week.
const TODAY_MISSED_DAYS: i64 = 7;
/// The current quarter's record shows the last two weeks of past work.
const RECORD_RECENT_DAYS: i64 = 14;

pub fn task_view(workspace: &Value, now: DateTime<FixedOffset>) -> Result<TaskView, ScheduleError> {
    let mut parsed = BTreeMap::new();
    let mut deadlines = BTreeMap::new();
    for row in array(workspace, "assignments")
        .iter()
        .filter(|row| visible(row))
    {
        let (Some(id), Some(raw)) = (text(row, "id"), text(row, "dueAt")) else {
            continue;
        };
        let due = match parse_timestamp(raw) {
            Ok(value) => value,
            Err(_) => continue,
        };
        parsed.insert(id.to_owned(), due);
        deadlines.insert(id.to_owned(), deadline_view(due, now));
    }
    let submitted_ids = array(workspace, "assignments")
        .iter()
        .filter(|row| visible(row) && assignment_submitted(row))
        .filter_map(|row| text(row, "id").map(str::to_owned))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut nearest_by_course = BTreeMap::<String, (DateTime<FixedOffset>, String)>::new();
    for row in array(workspace, "assignments").iter().filter(|row| {
        visible(row)
            && row.get("hidden").and_then(Value::as_bool) != Some(true)
            && !assignment_submitted(row)
    }) {
        let (Some(id), Some(course), Some(due)) = (
            text(row, "id"),
            text(row, "courseId"),
            text(row, "id").and_then(|id| parsed.get(id)),
        ) else {
            continue;
        };
        if *due < now {
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
            let submitted = assignment_submitted(row);
            let pinned = row.get("pinned").and_then(Value::as_bool) == Some(true);
            if pinned && !submitted {
                today_pinned.push((due, id));
            }
            match due {
                Some(due) if due < now => past.push((due, id, submitted, pinned)),
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
        nearest_by_course: nearest_by_course
            .into_iter()
            .map(|(k, (_, v))| (k, v))
            .collect(),
        by_term,
    })
}

fn deadline_view(due: DateTime<FixedOffset>, now: DateTime<FixedOffset>) -> TaskDeadlineView {
    let ms = (due - now).num_milliseconds();
    let abs = ms.saturating_abs();
    let relative = |value: i64| {
        let hours = (value / 3_600_000).max(1);
        if hours < 24 {
            format!("{hours}h")
        } else {
            format!("{}d", (hours + 12) / 24)
        }
    };
    let (label, tone) = if ms < 0 {
        (format!("{} ago", relative(abs)), "normal")
    } else if ms < 86_400_000 {
        (format!("Due in {}", relative(ms)), "warning")
    } else {
        let local = due.with_timezone(&FixedOffset::east_opt(9 * 3600).unwrap_or(*due.offset()));
        (format!("Due {}", local.format("%-d %b, %H:%M")), "normal")
    };
    let hours = ms.max(0) / 3_600_000;
    let days = hours / 24;
    let countdown_label = if ms < 0 {
        format!("{} ago", relative(abs))
    } else if hours < 1 {
        format!("{}m", (ms / 60_000).max(1))
    } else if hours < 24 {
        format!("{hours}h")
    } else if hours < 72 && hours % 24 != 0 {
        format!("{days}d {}h", hours % 24)
    } else {
        format!("{days}d")
    };
    let countdown_tone = if ms < 0 {
        "past"
    } else if hours < 24 {
        "urgent"
    } else if hours < 72 {
        "soon"
    } else {
        "normal"
    };
    TaskDeadlineView {
        at: due.to_rfc3339(),
        remaining_milliseconds: ms,
        label,
        tone: tone.into(),
        countdown_label,
        countdown_tone: countdown_tone.into(),
        overdue: ms < 0,
    }
}
