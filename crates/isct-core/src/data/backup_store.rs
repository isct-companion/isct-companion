use super::*;

impl DataStore {
    pub fn reset_workspace(&self) -> Result<Value, DataError> {
        let mut connection = self.open()?;
        let transaction = connection.transaction().map_err(DataError::from)?;
        let reset_tables = COLLECTIONS
            .iter()
            .map(|(_, table)| *table)
            .chain(SCHOOL_CACHE_TABLES.iter().copied())
            .chain(USER_BACKUP_TABLES.iter().copied())
            .chain([
                "offering_content",
                "offerings_fts",
                "catalog_search_items",
                "manual_courses",
                "manual_memberships",
                "course_overrides",
                "assignment_marks",
                "course_identity_map",
            ])
            .collect::<HashSet<_>>();
        for table in reset_tables {
            transaction
                .execute(&format!("DELETE FROM {table}"), [])
                .map_err(DataError::from)?;
        }
        transaction
            .execute("DELETE FROM settings", [])
            .map_err(DataError::from)?;
        transaction.commit().map_err(DataError::from)?;
        self.ensure_defaults()?;
        self.load_workspace()
    }

    pub fn export_data(&self, notes: Value) -> Result<String, DataError> {
        serde_json::to_string_pretty(&self.export_document(notes)?).map_err(DataError::from)
    }

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
    pub fn merge_cloud_data(&self, remote: &Value) -> Result<(), DataError> {
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(DataError::from)?;
        let local_meta = json!({"sync":read_setting(&transaction, "cloudSyncMetadata")?});
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
            let local = read_rows(&transaction, table)?;
            for incoming in remote["tables"][table].as_array().into_iter().flatten() {
                validate_user_record(table, incoming)?;
                let old = local.iter().find(|row| row["id"] == incoming["id"]);
                if old.is_none_or(|old| cloud_merge::compare(incoming, old).is_gt()) {
                    upsert_row(&transaction, table, incoming, true)?;
                }
            }
        }
        let local = read_settings_export(&transaction)?;
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
                import_setting(&transaction, incoming)?;
            }
        }
        if remote["sync"].is_object() {
            transaction.execute("INSERT INTO settings(key,value,hash,updated_at,updated_by,rev) VALUES('cloudSyncMetadata',?1,?3,?2,'sync',1) ON CONFLICT(key) DO UPDATE SET value=excluded.value,hash=excluded.hash,updated_at=excluded.updated_at",
                params![remote["sync"].to_string(), now(), content_hash(&remote["sync"])?]).map_err(DataError::from)?;
        }
        transaction.commit().map_err(DataError::from)
    }

    /// Restores a document that passed [`parse_import`].
    pub fn import_data(&self, parsed: &Value) -> Result<Value, DataError> {
        let tables = parsed
            .get("tables")
            .and_then(Value::as_object)
            .ok_or_else(|| DataError::compatibility("import tables must be an object"))?;
        let mut connection = self.open()?;
        let transaction = connection.transaction().map_err(DataError::from)?;
        let allowed: HashSet<_> = USER_BACKUP_TABLES.iter().copied().collect();
        // A local backup restore is an explicit replacement, not sync/merge.
        for table in USER_BACKUP_TABLES {
            transaction
                .execute(&format!("DELETE FROM {table}"), [])
                .map_err(DataError::from)?;
        }
        for (table, rows) in tables {
            if table == "settings" {
                continue;
            }
            if !allowed.contains(table.as_str()) {
                return Err(DataError::compatibility(format!(
                    "import contains unsupported table {table}"
                )));
            }
            let incoming_rows = rows.as_array().ok_or_else(|| {
                DataError::compatibility(format!("import table {table} must be an array"))
            })?;
            for incoming in incoming_rows {
                validate_user_record(table, incoming)?;
                upsert_row(&transaction, table, incoming, true)?;
            }
        }
        if let Some(settings) = tables.get("settings").and_then(Value::as_array) {
            for key in USER_BACKUP_SETTINGS {
                transaction
                    .execute("DELETE FROM settings WHERE key=?1", [key])
                    .map_err(DataError::from)?;
            }
            for row in settings {
                let key = string_field(row, "key")?;
                if !USER_BACKUP_SETTINGS.contains(&key) {
                    return Err(DataError::compatibility(format!(
                        "import contains device or secret setting {key}"
                    )));
                }
                import_setting(&transaction, row)?;
            }
        }
        transaction.commit().map_err(DataError::from)?;
        self.load_workspace()
    }
}
