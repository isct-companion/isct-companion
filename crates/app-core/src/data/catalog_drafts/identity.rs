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

/// Imported random ids are a baseline. Only an explicit edit or history
/// restore starts the revision clock for this offering and quarter.
pub(crate) fn baseline_revision(row: &Value) -> Option<u64> {
    row[BASELINE_REVISION].as_u64().filter(|_| row["rev"] == 0)
}

pub(in crate::data) const BASELINE_REVISION: &str = "baselineRevision";

/// Before the cutover, any live copy placed the offering. Each legacy id
/// had its own revision clock. Keep their combined revision solely for
/// comparing old snapshots; an actual edit always outranks this baseline.
pub(in crate::data) fn document_plans(rows: &[Value]) -> Cow<'_, [Value]> {
    if rows
        .iter()
        .all(|row| catalog_plan_id(row).is_none_or(|id| row["id"] == id))
    {
        return Cow::Borrowed(rows);
    }
    let mut plans = BTreeMap::<String, Value>::new();
    let mut legacy = BTreeMap::<String, Vec<&Value>>::new();
    for row in rows {
        let record_id = super::text(row, "id");
        match catalog_plan_id(row) {
            Some(id) if id != record_id => legacy.entry(id).or_default().push(row),
            _ => {
                plans.insert(record_id.to_owned(), row.clone());
            }
        }
    }
    for (id, copies) in legacy {
        let winner = copies.iter().copied().max_by(|a, b| {
            a["deletedAt"]
                .is_null()
                .cmp(&b["deletedAt"].is_null())
                .then_with(|| cloud_merge::compare(a, b))
        });
        if let Some(winner) = winner {
            let mut baseline = winner.clone();
            baseline["id"] = id.clone().into();
            baseline["rev"] = 0.into();
            baseline[BASELINE_REVISION] = copies
                .iter()
                .filter_map(|row| row["rev"].as_u64())
                .sum::<u64>()
                .into();
            if plans
                .get(&id)
                .is_none_or(|current| cloud_merge::compare(&baseline, current).is_gt())
            {
                plans.insert(id, baseline);
            }
        }
    }
    Cow::Owned(plans.into_values().collect())
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
