use super::*;

impl DataStore {
    pub fn lms_material_for_url(&self, url: &str) -> Result<Option<Value>, DataError> {
        let connection = self.open()?;
        let raw: Option<String> = connection
            .query_row(
                "SELECT value FROM lms_items
                 WHERE missing_since IS NULL AND deleted_at IS NULL
                   AND (json_extract(value,'$.url')=?1 OR json_extract(value,'$.resourceUrl')=?1)
                 LIMIT 1",
                [url],
                |row| row.get(0),
            )
            .optional()
            .map_err(DataError::from)?;
        raw.map(|raw| {
            serde_json::from_str(&raw)
                .map_err(|_| DataError::storage("LMS material contains invalid JSON"))
        })
        .transpose()
    }

    /// Store a lazy `view.php` resolution exactly like an LMS ingest update:
    /// unchanged content keeps its revision, while a changed remote URL gets
    /// one new version. Local cache paths never enter the database.
    pub fn update_lms_resource_url(
        &self,
        cmid: u64,
        source_url: &str,
        resource_url: &str,
    ) -> Result<bool, DataError> {
        self.update_lms_item_detail(
            cmid,
            source_url,
            "resourceUrl",
            Value::String(resource_url.into()),
        )
    }

    /// Keeps a folder's resolved file list on its material, so reopening it
    /// needs no LMS request. A later crawl keeps it until a new read replaces it.
    pub fn update_lms_folder_files(
        &self,
        cmid: u64,
        source_url: &str,
        files: &[crate::contracts::FolderFile],
    ) -> Result<bool, DataError> {
        let files = serde_json::to_value(files).map_err(DataError::from)?;
        self.update_lms_item_detail(cmid, source_url, "folderFiles", files)
    }

    /// Writes one resolved detail onto a live LMS material, as a versioned
    /// change; an unchanged value is not rewritten.
    pub(super) fn update_lms_item_detail(
        &self,
        cmid: u64,
        source_url: &str,
        key: &str,
        detail: Value,
    ) -> Result<bool, DataError> {
        self.write(Some(WorkspaceChangeSource::SchoolPull), |transaction| {
        let stored: Option<(String, String, u64)> = transaction
            .query_row(
                "SELECT id,value,rev FROM lms_items WHERE cmid=?1 AND missing_since IS NULL LIMIT 1",
                [cmid as i64],
                |row| Ok((row.get(0)?, row.get(1)?, row.get::<_, i64>(2)? as u64)),
            )
            .optional()
            .map_err(DataError::from)?;
        let Some((id, raw, revision)) = stored else {
            return Ok(false);
        };
        let mut value: Value = serde_json::from_str(&raw)
            .map_err(|_| DataError::storage("LMS material contains invalid JSON"))?;
        if value.get("url").and_then(Value::as_str) != Some(source_url) {
            return Ok(false);
        }
        if value.get(key) == Some(&detail) {
            return Ok(false);
        }
        value[key] = detail;
        let timestamp = now();
        let next_revision = revision + 1;
        value["rev"] = next_revision.into();
        value["updatedAt"] = timestamp.clone().into();
        value["updatedBy"] = "lms".into();
        value["source"] = json!({"provider":"lms"});
        value["lastSeenAt"] = timestamp.clone().into();
        let mut content = value.clone();
        if let Some(object) = content.as_object_mut() {
            for key in [
                "rev",
                "updatedAt",
                "updatedBy",
                "source",
                "lastSeenAt",
                "missingSince",
                "deletedAt",
            ] {
                object.remove(key);
            }
        }
        let hash = content_hash(&content)?;
        transaction
            .execute(
                "UPDATE lms_items SET value=?2,hash=?3,updated_at=?4,updated_by='lms',
                 rev=?5,source='lms',last_seen_at=?4,missing_since=NULL,deleted_at=NULL WHERE id=?1",
                params![
                    id,
                    serde_json::to_string(&value).map_err(DataError::from)?,
                    hash,
                    timestamp,
                    next_revision as i64
                ],
            )
            .map_err(DataError::from)?;
        Ok(true)
        })
    }
}
