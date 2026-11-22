//! A user's draft for an offering in one quarter has the same identity on
//! every device. Earlier writers minted separate record IDs for that intent.
use super::super::*;
use std::{borrow::Cow, collections::BTreeMap};

pub(crate) fn plan_id(term: &str, offering: &str) -> String {
    format!("catalog-plan:{term}:{offering}")
}

pub(in crate::data) fn catalog_plan_id(row: &Value) -> Option<String> {
    Some(plan_id(
        row["termId"].as_str()?,
        row["offeringId"].as_str().filter(|s| !s.is_empty())?,
    ))
}

/// Convert a snapshot as a whole: before the cutover, any live copy placed
/// the offering. A consolidated record advances beyond all copies it saw.
pub(in crate::data) fn document_plans(rows: &[Value]) -> Cow<'_, [Value]> {
    if rows
        .iter()
        .all(|row| catalog_plan_id(row).is_none_or(|id| row["id"] == id))
    {
        return Cow::Borrowed(rows);
    }
    let mut groups = BTreeMap::<String, Vec<&Value>>::new();
    for row in rows {
        let id = catalog_plan_id(row).unwrap_or_else(|| super::text(row, "id").to_owned());
        groups.entry(id).or_default().push(row);
    }
    let mut plans = Vec::with_capacity(groups.len());
    for (id, group) in groups {
        let canonical = group.iter().copied().find(|row| row["id"] == id);
        let winner = canonical.or_else(|| {
            group.iter().copied().max_by(|a, b| {
                a["deletedAt"]
                    .is_null()
                    .cmp(&b["deletedAt"].is_null())
                    .then_with(|| cloud_merge::compare(a, b))
            })
        });
        if let Some(winner) = winner {
            let mut plan = winner.clone();
            plan["id"] = id.into();
            if canonical.is_none() && group.len() > 1 {
                // Each independent legacy id has its own revision clock. An
                // edit to any one of them must advance the consolidated clock.
                plan["rev"] = group
                    .iter()
                    .filter_map(|row| row["rev"].as_u64())
                    .sum::<u64>()
                    .into();
            }
            plans.push(plan);
        }
    }
    Cow::Owned(plans)
}

/// No other table references plan record IDs. Install the new identities in
/// the opening transaction, before any reader or undo receipt can observe them.
pub(in crate::data) fn cut_over_plans(transaction: &Transaction<'_>) -> Result<(), DataError> {
    let stored = read_rows(transaction, "plan_items")?;
    if let Cow::Owned(plans) = document_plans(&stored) {
        for row in &stored {
            if catalog_plan_id(row).is_some_and(|id| row["id"] != id) {
                transaction.execute(
                    "DELETE FROM plan_items WHERE id=?1",
                    [string_field(row, "id")?],
                )?;
            }
        }
        for plan in &plans {
            if !stored.contains(plan) {
                upsert_row(transaction, "plan_items", plan, true)?;
            }
        }
    }
    Ok(())
}
