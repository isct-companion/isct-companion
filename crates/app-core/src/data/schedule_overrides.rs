use super::*;
use std::collections::BTreeMap;

/// A device or a course alias can bring another record for the same target.
/// Keep every id for backup and restore. The schedule reads one winning edit,
/// including its tombstone, so an older mark cannot come back after deletion.
pub(super) fn read_effective(connection: &Connection) -> Result<Vec<Value>, DataError> {
    records::read_json_rows(
        connection,
        "schedule_overrides",
        "SELECT value FROM schedule_overrides WHERE target_key IS NOT NULL ORDER BY id",
        [],
    )
}

fn by_target(rows: Vec<Value>) -> Result<BTreeMap<String, Value>, DataError> {
    let mut targets = BTreeMap::<String, Value>::new();
    for row in rows {
        let key = target_key(&row)?;
        if targets
            .get(key)
            .is_none_or(|old| cloud_merge::compare(&row, old).is_gt())
        {
            targets.insert(key.into(), row);
        }
    }
    Ok(targets)
}

pub(super) fn target_key(row: &Value) -> Result<&str, DataError> {
    row["targetKey"]
        .as_str()
        .ok_or_else(|| DataError::storage("schedule override target is missing"))
}

/// Restoration first writes by record id. For a target with multiple ids,
/// a tombstone of an omitted id can outrank the snapshot's winning record.
/// Stamp the snapshot's winner above the target's current revision in that case.
pub(super) fn restore_effective(
    transaction: &Transaction<'_>,
    restored: Vec<Value>,
    stamp: impl Fn(&mut Value, u64),
) -> Result<(), DataError> {
    if restored.is_empty() {
        return Ok(());
    }
    let current = by_target(read_rows(transaction, "schedule_overrides")?)?;
    for (key, mut desired) in by_target(restored)? {
        let winner = current
            .get(&key)
            .ok_or_else(|| DataError::storage("restored schedule override is missing"))?;
        if winner["id"] != desired["id"] {
            let revision = |row: &Value| {
                row["rev"]
                    .as_u64()
                    .ok_or_else(|| DataError::storage("schedule override revision is missing"))
            };
            let revision = revision(winner)?.max(revision(&desired)?);
            stamp(&mut desired, revision);
            upsert_row(transaction, "schedule_overrides", &desired, true)?;
        }
    }
    Ok(())
}

pub(super) fn effective_id(
    connection: &Connection,
    key: &str,
) -> Result<Option<String>, DataError> {
    Ok(connection
        .query_row(
            "SELECT id FROM schedule_overrides WHERE target_key=?1",
            [key],
            |row| row.get(0),
        )
        .optional()?)
}

/// Build the lookup once for stores whose older importer left the column
/// empty. The expression index also restricts later winner selection to the
/// records naming the changed target.
pub(super) fn initialize_lookup(transaction: &Transaction<'_>) -> Result<(), DataError> {
    let indexed: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='schedule_override_candidates')",
        [],
        |row| row.get(0),
    )?;
    if !indexed {
        transaction.execute_batch(
            "CREATE INDEX schedule_override_candidates
            ON schedule_overrides(json_extract(value, '$.targetKey'));
            UPDATE schedule_overrides SET target_key=NULL WHERE target_key IS NOT NULL;",
        )?;
        for (key, winner) in by_target(read_rows(transaction, "schedule_overrides")?)? {
            transaction.execute(
                "UPDATE schedule_overrides SET target_key=?2 WHERE id=?1",
                params![string_field(&winner, "id")?, key],
            )?;
        }
    }
    Ok(())
}

fn winner_id(connection: &Connection, key: &str) -> Result<Option<String>, DataError> {
    records::read_json_rows(
        connection,
        "schedule_overrides",
        "SELECT value FROM schedule_overrides WHERE json_extract(value, '$.targetKey')=?1",
        [key],
    )?
    .into_iter()
    .max_by(cloud_merge::compare)
    .map(|row| string_field(&row, "id").map(str::to_owned))
    .transpose()
}

/// Maintain the unique lookup column on the winning row. Stored JSON is
/// authoritative because older cloud imports did not populate this column.
pub(super) fn update_lookup(transaction: &Transaction<'_>, key: &str) -> Result<(), DataError> {
    let owner = winner_id(transaction, key)?;
    transaction.execute(
        "UPDATE schedule_overrides SET target_key=NULL WHERE target_key=?1 AND id IS NOT ?2",
        params![key, owner],
    )?;
    if let Some(id) = owner {
        transaction.execute(
            "UPDATE schedule_overrides SET target_key=?2 WHERE id=?1",
            params![id, key],
        )?;
    }
    Ok(())
}
