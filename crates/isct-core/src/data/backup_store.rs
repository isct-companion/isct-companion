use super::*;

impl DataStore {
    /// Read every backup table from one SQLite snapshot.
    pub fn export_document(&self, notes: Value) -> Result<Value, DataError> {
        let mut database = self.open()?;
        let connection = database.transaction()?;
        let mut tables = Map::new();
        for table in USER_BACKUP_TABLES {
            tables.insert(
                (*table).into(),
                Value::Array(read_rows(&connection, table)?),
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

        Ok(json!({
            "schemaVersion": BACKUP_SCHEMA_VERSION,
            "exportedAt": now(),
            "kind": BACKUP_KIND,
            "deviceId": "local-backup",
            "sync": read_setting(&connection, "cloudSyncMetadata")?,
            "tables": tables,
            "localNotes": notes
        }))
    }

    /// Merge under an IMMEDIATE transaction, including edits committed during
    /// the network round trip. Never replace the device's settings or identity.
    pub fn merge_cloud_data(
        &self,
        remote: &Value,
        source: WorkspaceChangeSource,
    ) -> Result<(), DataError> {
        self.write(Some(source), |transaction| {
        let local_meta = json!({"sync":read_setting(transaction, "cloudSyncMetadata")?});
        if cloud_merge::generation(remote) < cloud_merge::generation(&local_meta) {
            return Ok(());
        }
        if cloud_merge::generation(remote) > cloud_merge::generation(&local_meta) {
            for table in USER_BACKUP_TABLES {
                transaction
                    .execute(&format!("DELETE FROM {table}"), [])
                    .map_err(DataError::from)?;
            }
            for key in USER_BACKUP_SETTINGS {
                transaction
                    .execute("DELETE FROM settings WHERE key=?1", [key])
                    .map_err(DataError::from)?;
            }
        }
        for table in USER_BACKUP_TABLES {
            let local = read_rows(transaction, table)?;
            for incoming in remote["tables"][table].as_array().into_iter().flatten() {
                validate_user_record(table, incoming)?;
                let old = local.iter().find(|row| row["id"] == incoming["id"]);
                if old.is_none_or(|old| cloud_merge::compare(incoming, old).is_gt()) {
                    upsert_row(transaction, table, incoming, true)?;
                }
            }
        }
        let local = read_settings_export(transaction)?;
        for incoming in remote["tables"]["settings"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let mut incoming = incoming.clone();
            cloud_merge::sanitize_setting(&mut incoming);
            let incoming = &incoming;
            let key = string_field(incoming, "key")?;
            if !USER_BACKUP_SETTINGS.contains(&key) {
                continue;
            }
            if local
                .iter()
                .find(|row| row["key"] == key)
                .is_none_or(|old| cloud_merge::compare(incoming, old).is_gt())
            {
                import_setting(transaction, incoming)?;
            }
        }
        if remote["sync"].is_object() {
            transaction.execute("INSERT INTO settings(key,value,hash,updated_at,updated_by,rev) VALUES('cloudSyncMetadata',?1,?3,?2,'sync',1) ON CONFLICT(key) DO UPDATE SET value=excluded.value,hash=excluded.hash,updated_at=excluded.updated_at",
                params![remote["sync"].to_string(), now(), content_hash(&remote["sync"])?]).map_err(DataError::from)?;
        }
        Ok(())
        })
    }
}
