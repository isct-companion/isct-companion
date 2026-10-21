use super::*;

impl DataStore {
    pub fn catalog_upsert_offerings(&self, offerings: &[Value]) -> Result<usize, DataError> {
        for offering in offerings {
            string_field(offering, "id")?;
        }
        self.write(None, |transaction| {
            let mut changed = 0;
            for offering in offerings {
                changed += usize::from(upsert_offering_in(transaction, offering)?);
            }
            Ok(changed)
        })
    }

    /// Merges offerings inside the caller's transaction, with the same
    /// official-field ownership as [`Self::catalog_upsert_offerings`], so a
    /// `catalog.official` key commits its rows and its freshness together.
    pub(crate) fn catalog_upsert_offerings_in(
        transaction: &Transaction<'_>,
        offerings: &[Value],
    ) -> Result<usize, DataError> {
        for offering in offerings {
            string_field(offering, "id")?;
        }
        let mut changed = 0;
        for offering in offerings {
            changed += usize::from(upsert_offering_in(transaction, offering)?);
        }
        Ok(changed)
    }

    /// Adds the English title and instructor names of 教務's English page
    /// to stored offerings, by offering id. A row not stored yet is skipped
    /// (its Japanese read creates it); instructor names land only when the
    /// two pages list as many.
    pub(crate) fn catalog_merge_english_in(
        transaction: &Transaction<'_>,
        rows: &[Value],
    ) -> Result<usize, DataError> {
        let mut changed = 0;
        for row in rows {
            let id = string_field(row, "id")?;
            let Some(mut offering) = catalog_offering_in(transaction, id)? else {
                continue;
            };
            if let Some(object) = offering.as_object_mut() {
                for field in ROW_METADATA {
                    object.remove(field);
                }
            }
            if let Some(title) = row.pointer("/title/en").filter(|title| title != &"") {
                offering["title"]["en"] = title.clone();
            }
            let english = row["instructors"].as_array().cloned().unwrap_or_default();
            if let Some(names) = offering["instructors"]
                .as_array_mut()
                .filter(|names| names.len() == english.len())
            {
                for (name, en) in names.iter_mut().zip(&english) {
                    name["en"] = en["en"].clone();
                }
            }
            changed += usize::from(upsert_offering_in(transaction, &offering)?);
        }
        Ok(changed)
    }

    /// The `dataset_state` rows of these keys, in order.
    pub(crate) fn catalog_key_states(
        &self,
        keys: &[crate::sync_state::Key],
    ) -> Result<Vec<Option<crate::sync_state::KeyState>>, DataError> {
        let connection = self.open()?;
        keys.iter()
            .map(|key| crate::sync_state::get(&connection, key))
            .collect()
    }

    /// The official search form's filter choices last captured for `year`,
    /// or an empty list. Device-local public metadata, never backed up.
    pub fn kyomu_search_options(&self, year: u16) -> Result<Value, DataError> {
        Ok(read_setting(&*self.open()?, &search_options_setting(year))?
            .unwrap_or_else(|| json!([])))
    }

    /// Stores the captured filter choices of `year` and marks
    /// `catalog.official/{year}/options` fresh; unchanged choices only renew it.
    pub(crate) fn record_kyomu_search_options(
        &self,
        year: u16,
        options: &Value,
        now: DateTime<Utc>,
    ) -> Result<(), DataError> {
        let key = crate::sync_state::Key::new(
            crate::sync_state::Dataset::CatalogOfficial,
            format!("{year}/options"),
        );
        let setting = search_options_setting(year);
        self.write(None, |transaction| {
            if read_setting(transaction, &setting)?.as_ref() == Some(options)
                && crate::sync_state::renew(transaction, &key, "", now)?
            {
                return Ok(());
            }
            save_setting(transaction, &setting, options, "catalog")?;
            crate::sync_state::commit(
                transaction,
                &key,
                crate::sync_state::Commit {
                    account: "",
                    validator: Some(&content_hash(options)?),
                    outcome: crate::sync_state::Outcome::Complete,
                    message: None,
                },
                now,
            )
        })
    }
}

fn search_options_setting(year: u16) -> String {
    format!("kyomuSearchOptions:{year}")
}

/// Bookkeeping fields every write sets anew; they do not make a row differ.
const ROW_METADATA: [&str; 4] = ["rev", "updatedAt", "updatedBy", "lastSeenAt"];

fn same_content(incoming: &Value, stored: &Value) -> bool {
    let strip = |value: &Value| {
        let mut value = value.clone();
        if let Some(object) = value.as_object_mut() {
            for field in ROW_METADATA {
                object.remove(field);
            }
        }
        value
    };
    strip(incoming) == strip(stored)
}

/// Merges one offering into `offerings` and its search index.
fn upsert_offering_in(transaction: &Transaction<'_>, offering: &Value) -> Result<bool, DataError> {
    let mut offering = offering.clone();
    let offering_id = string_field(&offering, "id")?.to_string();
    let existing: Option<String> = transaction
        .query_row(
            "SELECT value FROM offerings WHERE id=?1",
            [&offering_id],
            |row| row.get(0),
        )
        .optional()?;
    let existing = existing
        .map(|raw| serde_json::from_str::<Value>(&raw))
        .transpose()?;
    if let Some(existing) = &existing {
        merge_official_fields(&mut offering, existing);
    }
    if offering.get("kyomu").is_some() {
        let departments: Vec<Value> =
            serde_json::from_str(include_str!("../course_query/departments.json"))?;
        if let Some(department) = departments.iter().find(|d| {
            offering["code"].as_str().is_some_and(|code| {
                code.starts_with(&format!("{}.", d["codePrefix"].as_str().unwrap_or("?")))
            })
        }) {
            offering["unitCode"] = department["unitCode"].clone();
        }
    }
    let timestamp = now();
    let source_key = format!(
        "kyomu:{}:offering:{}",
        offering
            .get("year")
            .and_then(Value::as_i64)
            .unwrap_or_default(),
        offering
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or_default()
    );
    let object = offering
        .as_object_mut()
        .ok_or_else(|| "offering must be an object".to_string())?;
    object
        .entry("sourceKey")
        .or_insert(Value::String(source_key));
    // A source that lists a row again brings it back from missing; a row
    // whose content did not change is not written.
    let revived = transaction.execute(
        "UPDATE offerings SET missing_since=NULL WHERE id=?1 AND missing_since IS NOT NULL",
        [&offering_id],
    )? > 0;
    if existing
        .as_ref()
        .is_some_and(|existing| same_content(&offering, existing))
    {
        return Ok(revived);
    }
    let object = offering
        .as_object_mut()
        .ok_or_else(|| "offering must be an object".to_string())?;
    object.entry("rev").or_insert(Value::from(1));
    object
        .entry("updatedAt")
        .or_insert(Value::String(timestamp.clone()));
    object
        .entry("updatedBy")
        .or_insert(Value::String("catalog".into()));
    object
        .entry("lastSeenAt")
        .or_insert(Value::String(timestamp.clone()));
    serde_json::from_value::<crate::contracts::Offering>(offering.clone())
        .map_err(|error| format!("offering validation failed: {error}"))?;
    let changed = upsert_row(transaction, "offerings", &offering, true)?;
    index_offering(transaction, &offering)?;
    Ok(changed || revived)
}

/// Replaces one offering's search index row.
fn index_offering(transaction: &Transaction<'_>, offering: &Value) -> Result<(), DataError> {
    let id = string_field(offering, "id")?;
    let code = offering
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let title_ja = offering
        .pointer("/title/ja")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let title_en = offering
        .pointer("/title/en")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let instructors_ja = offering
        .get("instructors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|name| name.get("ja").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    let instructors_en = offering
        .get("instructors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|name| name.get("en").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    transaction
        .execute("DELETE FROM offerings_fts WHERE offering_id=?1", [id])
        .map_err(DataError::from)?;
    transaction
        .execute(
            "INSERT INTO offerings_fts
                     (offering_id, code, title_ja, title_en, instructors_ja, instructors_en)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, code, title_ja, title_en, instructors_ja, instructors_en],
        )
        .map_err(DataError::from)?;
    Ok(())
}

impl DataStore {
    pub fn catalog_search(&self, query: &str) -> Result<Vec<Value>, DataError> {
        let connection = self.open()?;
        if query.trim().is_empty() {
            return catalog_list(&connection);
        }
        let expression = query
            .split_whitespace()
            .map(|token| format!("\"{}\"", token.replace('"', "\"\"")))
            .map(|token| format!("{token}*"))
            .collect::<Vec<_>>()
            .join(" ");
        let mut statement = connection
            .prepare(
                "SELECT o.value FROM offerings_fts f
                 JOIN offerings o ON o.id=f.offering_id AND o.deleted_at IS NULL AND o.missing_since IS NULL
                 WHERE offerings_fts MATCH ?1
                 ORDER BY rank LIMIT 100",
            )
            .map_err(DataError::from)?;
        let rows = statement
            .query_map([expression], |row| row.get::<_, String>(0))
            .map_err(DataError::from)?
            .map(|row| {
                serde_json::from_str(&row.map_err(DataError::from)?)
                    .map_err(|_| DataError::storage("offering contains invalid JSON"))
            })
            .collect();
        rows
    }

    pub fn catalog_offering(&self, id: &str) -> Result<Option<Value>, DataError> {
        let connection = self.open()?;
        catalog_offering_in(&connection, id)
    }

}

/// The 教務 Japanese page owns every field of an offering; the English
/// page adds only the English title and instructor names, which a later
/// Japanese read (a row without them) keeps.
fn merge_official_fields(incoming: &mut Value, stored: &Value) {
    if let Some(en) = stored
        .pointer("/title/en")
        .filter(|_| incoming.pointer("/title/en").is_none())
    {
        incoming["title"]["en"] = en.clone();
    }
    let stored_names = stored["instructors"].as_array().cloned().unwrap_or_default();
    for (index, name) in incoming["instructors"]
        .as_array_mut()
        .into_iter()
        .flatten()
        .enumerate()
    {
        if let Some(en) = stored_names
            .get(index)
            .filter(|stored| name.get("en").is_none() && stored["ja"] == name["ja"])
            .and_then(|stored| stored.get("en"))
        {
            name["en"] = en.clone();
        }
    }
}
