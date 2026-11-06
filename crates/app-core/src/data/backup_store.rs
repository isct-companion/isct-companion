use super::*;

impl DataStore {
    /// Read every backup table from one SQLite snapshot.
    pub fn export_document(&self) -> Result<Value, DataError> {
        let mut database = self.open()?;
        let connection = database.transaction()?;
        let mut tables = Map::new();
        for table in USER_BACKUP_TABLES {
            tables.insert(
                (*table).into(),
                Value::Array(synced_rows(&connection, table)?),
            );
        }
        let settings = read_settings_export(&connection)?
            .into_iter()
            .filter(|row| {
                row.get("key")
                    .and_then(Value::as_str)
                    .is_some_and(|key| USER_BACKUP_SETTINGS.contains(&key))
            })
            .collect();
        tables.insert("settings".into(), Value::Array(settings));
        Ok(backup_document(Value::String(now()), Value::Object(tables)))
    }

    /// Merge under an IMMEDIATE transaction, including edits committed during
    /// the network round trip. Never replace the device's settings or identity.
    pub fn merge_cloud_data(
        &self,
        remote: &Value,
        source: WorkspaceChangeSource,
    ) -> Result<(), DataError> {
        self.write(Some(source), |transaction| {
            for table in USER_BACKUP_TABLES {
                // Compared as synced copies, which name courses by external key.
                let local = synced_rows(transaction, table)?;
                for incoming in remote["tables"][table].as_array().into_iter().flatten() {
                    validate_user_record(table, incoming)?;
                    let old = local.iter().find(|row| row["id"] == incoming["id"]);
                    if old.is_none_or(|old| cloud_merge::compare(incoming, old).is_gt()) {
                        let mut stored = incoming.clone();
                        course_keys::from_synced(transaction, table, &mut stored)?;
                        upsert_row(transaction, table, &stored, true)?;
                    }
                }
            }
            let local = read_settings_export(transaction)?;
            for incoming in backup_settings(remote) {
                let key = string_field(&incoming, "key")?;
                if local
                    .iter()
                    .find(|row| row["key"] == key)
                    .is_none_or(|old| cloud_merge::compare(&incoming, old).is_gt())
                {
                    import_setting(transaction, &incoming)?;
                }
            }
            Ok(())
        })
    }

    /// Restores `old` as this device's newest edits, the way document apps
    /// restore a version: each record it holds is written back above every
    /// revision seen here, and each record it lacks is deleted. Other devices
    /// then merge it record by record, so edits they made meanwhile to other
    /// records survive.
    pub fn restore_records(
        &self,
        old: &Value,
        source: WorkspaceChangeSource,
    ) -> Result<(), DataError> {
        self.write(Some(source), |transaction| {
            let timestamp = now();
            let device = read_setting(transaction, "deviceId")?
                .and_then(|id| id.as_str().map(str::to_owned))
                .ok_or_else(|| DataError::storage("device id missing"))?;
            let stamp = |row: &mut Value, rev: u64| {
                row["rev"] = Value::from(rev + 1);
                row["updatedAt"] = Value::String(timestamp.clone());
                row["updatedBy"] = Value::String(device.clone());
            };
            for table in USER_BACKUP_TABLES {
                let local = synced_rows(transaction, table)?;
                let restored = old["tables"][table].as_array().cloned().unwrap_or_default();
                for incoming in &restored {
                    validate_user_record(table, incoming)?;
                    let current = local.iter().find(|row| row["id"] == incoming["id"]);
                    if current.is_some_and(|current| same_record(current, incoming)) {
                        continue;
                    }
                    let mut stored = incoming.clone();
                    stamp(
                        &mut stored,
                        revision(incoming).max(current.map_or(0, revision)),
                    );
                    course_keys::from_synced(transaction, table, &mut stored)?;
                    upsert_row(transaction, table, &stored, true)?;
                }
                for current in &local {
                    if current.get("deletedAt").is_some_and(Value::is_string)
                        || restored.iter().any(|row| row["id"] == current["id"])
                    {
                        continue;
                    }
                    let mut stored = current.clone();
                    stamp(&mut stored, revision(current));
                    stored["deletedAt"] = Value::String(timestamp.clone());
                    course_keys::from_synced(transaction, table, &mut stored)?;
                    upsert_row(transaction, table, &stored, true)?;
                }
            }
            // A setting the old copy lacks keeps its current value: settings
            // have no deleted state to restore.
            let local = read_settings_export(transaction)?;
            for mut incoming in backup_settings(old) {
                let key = string_field(&incoming, "key")?;
                let current = local.iter().find(|row| row["key"] == key);
                if current.is_some_and(|current| current["hash"] == incoming["hash"]) {
                    continue;
                }
                let rev = revision(&incoming).max(current.map_or(0, revision));
                stamp(&mut incoming, rev);
                import_setting(transaction, &incoming)?;
            }
            Ok(())
        })
    }
}

/// The settings rows a backup may carry, sanitized as every merge does.
fn backup_settings(document: &Value) -> Vec<Value> {
    document["tables"]["settings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| {
            row["key"]
                .as_str()
                .is_some_and(|key| USER_BACKUP_SETTINGS.contains(&key))
        })
        .map(|row| {
            let mut row = row.clone();
            cloud_merge::sanitize_setting(&mut row);
            if let Ok(hash) = content_hash(&row["value"]) {
                row["hash"] = Value::String(hash);
            }
            row
        })
        .collect()
}

fn revision(row: &Value) -> u64 {
    row["rev"].as_u64().unwrap_or(0)
}

fn same_record(left: &Value, right: &Value) -> bool {
    content_value(left) == content_value(right) && left.get("deletedAt") == right.get("deletedAt")
}

/// `table`'s rows as synced copies carry them (`course_keys::to_synced`).
fn synced_rows(connection: &Connection, table: &str) -> Result<Vec<Value>, DataError> {
    let mut rows = read_rows(connection, table)?;
    for row in &mut rows {
        course_keys::to_synced(connection, table, row)?;
    }
    Ok(rows)
}
