//! Tools that change the person's own local data: the plan, weekly personal
//! events and locally hidden assignments. Each write runs in one
//! transaction and rechecks agent access there.
use super::reads::IdArgs;
use super::registry::{Effect, Tool};
use super::{non_empty, parse_term, text, visible_rows, AgentError, Context};
use crate::data::DataError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct PlanArgs {
    /// A catalog offering id from `search_catalog`.
    offering_id: String,
    /// YYYY-nQ. Adding uses one of the offering's quarters.
    term: String,
}

pub(super) struct PlanAdd;

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct PlanAdded {
    course_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    plan_item_id: Option<String>,
    /// Every quarter the offering was planned into.
    terms: Vec<String>,
}

impl Tool for PlanAdd {
    const NAME: &'static str = "plan_add";
    const DESCRIPTION: &'static str = "Add a catalog offering from search_catalog to the person's local plan. term is YYYY-nQ and must be one of the offering's quarters; an offering running over several quarters (e.g. 3Q–4Q) is planned into all of them.";
    const EFFECT: Effect = Effect::LocalWrite;
    const IDEMPOTENT: bool = true;
    type Args = PlanArgs;
    type Output = PlanAdded;

    fn run(cx: &Context<'_>, args: PlanArgs) -> Result<PlanAdded, AgentError> {
        let offering_id = non_empty(&args.offering_id, "offeringId")?;
        let term = non_empty(&args.term, "term")?;
        let (year, quarter) = parse_term(term)?;
        Ok(cx.store.agent_write(|write, latest_workspace| {
            let offering = write.catalog_row(offering_id)?.ok_or_else(|| {
                DataError::InvalidInput("offering was not found in the local catalog".into())
            })?;
            if offering.get("year").and_then(Value::as_i64) != Some(year) {
                return Err(DataError::InvalidInput(
                    "offering does not belong to the requested academic year".into(),
                ));
            }
            let quarters = offering
                .get("quarters")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>();
            if !quarters.is_empty() && !quarters.iter().any(|value| value == quarter) {
                return Err(DataError::InvalidInput(format!(
                    "offering is taught in {}, not {quarter}",
                    quarters.join(", ")
                )));
            }
            let receipt = crate::workspace::plan_catalog_courses(
                write,
                latest_workspace,
                term,
                vec![offering],
                false,
            )?;
            let placement = receipt
                .placements
                .iter()
                .find(|placement| placement.term_id == term)
                .ok_or_else(|| DataError::storage("the requested term was not planned"))?;
            Ok(PlanAdded {
                course_id: placement.course_id.clone(),
                plan_item_id: Some(placement.plan_id.clone()),
                terms: receipt
                    .placements
                    .into_iter()
                    .map(|placement| placement.term_id)
                    .collect(),
            })
        })?)
    }
}

pub(super) struct PlanRemove;

#[derive(Serialize, JsonSchema)]
pub(super) struct PlanRemoved {
    removed: usize,
}

impl Tool for PlanRemove {
    const NAME: &'static str = "plan_remove";
    const DESCRIPTION: &'static str = "Remove an offering from the person's local plan.";
    const EFFECT: Effect = Effect::LocalWrite;
    const DESTRUCTIVE: bool = true;
    type Args = PlanArgs;
    type Output = PlanRemoved;

    fn run(cx: &Context<'_>, args: PlanArgs) -> Result<PlanRemoved, AgentError> {
        let offering_id = non_empty(&args.offering_id, "offeringId")?;
        let term = non_empty(&args.term, "term")?;
        parse_term(term)?;
        let removed = cx.store.agent_write(|write, _| {
            let plans = write.rows("plan_items")?;
            let removed = crate::workspace::remove_catalog_plans(
                write,
                plans
                    .iter()
                    .filter(|row| row["offeringId"] == offering_id && row["termId"] == term),
            )?;
            if removed == 0 {
                return Err(DataError::InvalidInput(format!(
                    "offering {offering_id} is not planned for {term}"
                )));
            }
            Ok(removed)
        })?;
        Ok(PlanRemoved { removed })
    }
}

/// The event a write touched.
#[derive(Serialize, JsonSchema)]
pub(super) struct EventWritten {
    id: String,
}

pub(super) struct WeeklyEventCreate;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct NewEvent {
    /// YYYY-nQ.
    term: String,
    title: String,
    /// Weekdays, 1 = Monday … 7 = Sunday; at least one.
    days: Vec<u8>,
    /// HH:MM, Japan time.
    start_time: String,
    /// HH:MM, Japan time.
    end_time: String,
    #[serde(default)]
    room: Option<String>,
}

impl Tool for WeeklyEventCreate {
    const NAME: &'static str = "weekly_event_create";
    const DESCRIPTION: &'static str = "Create a weekly personal event (a lab meeting, club meeting or seminar) in one quarter and return its id. term is YYYY-nQ; times are HH:MM in Japan time. One-off events belong in the person's own calendar app.";
    const EFFECT: Effect = Effect::LocalWrite;
    type Args = NewEvent;
    type Output = EventWritten;

    fn run(cx: &Context<'_>, event: NewEvent) -> Result<EventWritten, AgentError> {
        let term = non_empty(&event.term, "term")?;
        parse_term(term)?;
        let draft = json!({
            "title": non_empty(&event.title, "title")?,
            "days": event.days,
            "startTime": non_empty(&event.start_time, "startTime")?,
            "endTime": non_empty(&event.end_time, "endTime")?,
            "room": event.room,
        });
        let id = cx.store.agent_write(|write, current| {
            crate::workspace::save_event(write, current, &draft, term, None)
        })?;
        Ok(EventWritten { id })
    }
}

pub(super) struct WeeklyEventUpdate;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct EventChange {
    id: String,
    #[serde(default)]
    title: Option<String>,
    /// Weekdays, 1 = Monday … 7 = Sunday; at least one.
    #[serde(default)]
    days: Option<Vec<u8>>,
    /// HH:MM, Japan time.
    #[serde(default)]
    start_time: Option<String>,
    /// HH:MM, Japan time.
    #[serde(default)]
    end_time: Option<String>,
    #[serde(default)]
    room: Option<String>,
}

impl Tool for WeeklyEventUpdate {
    const NAME: &'static str = "weekly_event_update";
    const DESCRIPTION: &'static str =
        "Update a weekly personal event; fields left out keep their value. Times are HH:MM in Japan time.";
    const EFFECT: Effect = Effect::LocalWrite;
    const DESTRUCTIVE: bool = true;
    const IDEMPOTENT: bool = true;
    type Args = EventChange;
    type Output = EventWritten;

    fn run(cx: &Context<'_>, change: EventChange) -> Result<EventWritten, AgentError> {
        let id = non_empty(&change.id, "id")?;
        cx.store.agent_write(|write, current| {
            let mut row = live_event(current, id)?;
            let fields = [
                ("title", change.title.clone().map(Value::from)),
                ("days", change.days.clone().map(Value::from)),
                ("startTime", change.start_time.clone().map(Value::from)),
                ("endTime", change.end_time.clone().map(Value::from)),
                ("room", change.room.clone().map(Value::from)),
            ];
            for (key, value) in fields {
                if let Some(value) = value {
                    row[key] = value;
                }
            }
            let term = text(&row, "termId").unwrap_or_default().to_string();
            crate::workspace::save_event(write, current, &row, &term, Some(id.into()))
        })?;
        Ok(EventWritten { id: id.into() })
    }
}

pub(super) struct WeeklyEventDelete;

impl Tool for WeeklyEventDelete {
    const NAME: &'static str = "weekly_event_delete";
    const DESCRIPTION: &'static str =
        "Delete a weekly personal event. A deleted event can be neither changed nor deleted again.";
    const EFFECT: Effect = Effect::LocalWrite;
    const DESTRUCTIVE: bool = true;
    type Args = IdArgs;
    type Output = EventWritten;

    fn run(cx: &Context<'_>, IdArgs { id }: IdArgs) -> Result<EventWritten, AgentError> {
        let id = non_empty(&id, "id")?;
        cx.store.agent_write(|write, current| {
            live_event(current, id)?;
            crate::workspace::delete_event(write, current, id)
        })?;
        Ok(EventWritten { id: id.into() })
    }
}

/// A weekly personal event that has not been deleted, in the write's own
/// snapshot.
fn live_event(workspace: &Value, id: &str) -> Result<Value, DataError> {
    visible_rows(workspace, "events")
        .into_iter()
        .find(|row| text(row, "id") == Some(id))
        .ok_or_else(|| DataError::InvalidInput("event was not found".into()))
}

/// The assignment a hide or restore touched, and whether it is now hidden.
#[derive(Serialize, JsonSchema)]
pub(super) struct AssignmentMarked {
    id: String,
    hidden: bool,
}

fn mark_assignment(
    cx: &Context<'_>,
    id: &str,
    hidden: bool,
) -> Result<AssignmentMarked, AgentError> {
    let id = non_empty(id, "id")?;
    cx.store.agent_write(|write, current| {
        crate::workspace::set_assignment_mark(write, current, id, Some(hidden), None, None)
    })?;
    Ok(AssignmentMarked {
        id: id.into(),
        hidden,
    })
}

pub(super) struct HideAssignment;

impl Tool for HideAssignment {
    const NAME: &'static str = "hide_assignment";
    const DESCRIPTION: &'static str = "Hide an assignment locally without changing LMS.";
    const EFFECT: Effect = Effect::LocalWrite;
    const IDEMPOTENT: bool = true;
    type Args = IdArgs;
    type Output = AssignmentMarked;

    fn run(cx: &Context<'_>, IdArgs { id }: IdArgs) -> Result<AssignmentMarked, AgentError> {
        mark_assignment(cx, &id, true)
    }
}

pub(super) struct RestoreAssignment;

impl Tool for RestoreAssignment {
    const NAME: &'static str = "restore_assignment";
    const DESCRIPTION: &'static str = "Restore a locally hidden assignment.";
    const EFFECT: Effect = Effect::LocalWrite;
    const IDEMPOTENT: bool = true;
    type Args = IdArgs;
    type Output = AssignmentMarked;

    fn run(cx: &Context<'_>, IdArgs { id }: IdArgs) -> Result<AssignmentMarked, AgentError> {
        mark_assignment(cx, &id, false)
    }
}
