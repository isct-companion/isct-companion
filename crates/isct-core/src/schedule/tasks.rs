use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Duration, FixedOffset};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::{
    array, course_terms, parse_timestamp, placement_terms, scheduled_courses, text, visible,
    ScheduleError,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TaskDeadlineView {
    pub at: String,
    pub label: String,
    pub tone: String,
    pub countdown_label: String,
    pub countdown_tone: String,
    pub overdue: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TermTaskView {
    pub pending_ids: Vec<String>,
    pub pinned_ids: Vec<String>,
    pub overdue_ids: Vec<String>,
    pub due_this_week_ids: Vec<String>,
    pub later_ids: Vec<String>,
    pub earlier_due_this_week_ids: Vec<String>,
    pub earlier_later_ids: Vec<String>,
    pub lecture_open_ids: Vec<String>,
    pub lecture_hidden_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TaskView {
    pub deadlines: BTreeMap<String, TaskDeadlineView>,
    pub nearest_by_course: BTreeMap<String, String>,
    pub by_term: BTreeMap<String, TermTaskView>,
}

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
    let submitted = |row: &Value| {
        matches!(text(row, "submission"), Some("submitted" | "graded"))
            || text(row, "submissionStatus") == Some("submitted")
    };
    let mut nearest_by_course = BTreeMap::<String, (DateTime<FixedOffset>, String)>::new();
    for row in array(workspace, "assignments").iter().filter(|row| {
        visible(row) && row.get("hidden").and_then(Value::as_bool) != Some(true) && !submitted(row)
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
    // Quarters each assignment's course belongs to (a 3Q–4Q course's work
    // counts in both); `None` when the course is unknown.
    let assignment_terms: BTreeMap<&str, Option<Vec<String>>> = array(workspace, "assignments")
        .iter()
        .filter_map(|row| {
            let course_terms = text(row, "courseId")
                .and_then(|course| {
                    array(workspace, "courses")
                        .iter()
                        .find(|item| visible(item) && text(item, "id") == Some(course))
                })
                .map(|course| course_terms(workspace, course));
            Some((text(row, "id")?, course_terms))
        })
        .collect();
    let mut by_term = BTreeMap::new();
    for term in terms {
        let course_ids: HashSet<String> = scheduled_courses(workspace, &term)
            .iter()
            .filter_map(|row| text(row, "id").map(str::to_owned))
            .collect();
        let mut view = TermTaskView::default();
        let mut today = Vec::new();
        for row in array(workspace, "assignments")
            .iter()
            .filter(|row| visible(row))
        {
            let Some(id) = text(row, "id") else { continue };
            let in_course = text(row, "courseId").is_some_and(|course| course_ids.contains(course));
            let hidden = row.get("hidden").and_then(Value::as_bool) == Some(true);
            if in_course {
                if hidden {
                    view.lecture_hidden_ids.push(id.into());
                } else if !submitted(row)
                    && text(row, "dueAt")
                        .is_none_or(|_| parsed.get(id).is_some_and(|due| *due >= now))
                {
                    view.lecture_open_ids.push(id.into());
                }
            }
            let Some(due) = parsed.get(id) else { continue };
            let in_term = match assignment_terms.get(id).and_then(Option::as_ref) {
                Some(terms) => terms.contains(&term),
                None => *due >= now,
            };
            if hidden || !in_term {
                continue;
            }
            today.push((*due, id.to_owned(), row));
        }
        today.sort_by_key(|(due, id, _)| (*due, id.clone()));
        let week = now + Duration::days(7);
        for (due, id, row) in &today {
            let is_submitted = submitted(row);
            let pinned = row.get("pinned").and_then(Value::as_bool) == Some(true);
            if !is_submitted && *due >= now {
                view.pending_ids.push(id.clone());
            }
            if pinned {
                view.pinned_ids.push(id.clone());
                continue;
            }
            if *due < now {
                view.overdue_ids.push(id.clone());
                continue;
            }
            let target = if *due <= week {
                &mut view.earlier_due_this_week_ids
            } else {
                &mut view.earlier_later_ids
            };
            target.push(id.clone());
            if !is_submitted {
                let target = if *due <= week {
                    &mut view.due_this_week_ids
                } else {
                    &mut view.later_ids
                };
                target.push(id.clone());
            }
        }
        view.overdue_ids.reverse();
        let sort_due = |ids: &mut Vec<String>| {
            ids.sort_by_key(|id| (!parsed.contains_key(id), parsed.get(id).copied()))
        };
        sort_due(&mut view.lecture_open_ids);
        sort_due(&mut view.lecture_hidden_ids);
        by_term.insert(term, view);
    }
    Ok(TaskView {
        deadlines,
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
        label,
        tone: tone.into(),
        countdown_label,
        countdown_tone: countdown_tone.into(),
        overdue: ms < 0,
    }
}
