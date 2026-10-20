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

    /// One syllabus detail read: the offering, its content rows and its
    /// `catalog.detail/{id}` freshness commit together, with the read's
    /// `sync_runs` row. Returns the merged offering.
    pub(crate) fn catalog_store_detail(
        &self,
        offering: &Value,
        content: &[Value],
        run: &crate::sync_state::RunRecord<'_>,
    ) -> Result<Option<Value>, DataError> {
        let id = string_field(offering, "id")?;
        self.write(None, |transaction| {
            upsert_offering_in(transaction, offering)?;
            upsert_offering_content_in(transaction, content)?;
            crate::sync_state::commit(
                transaction,
                run.key,
                crate::sync_state::Commit {
                    account: run.account,
                    validator: None,
                    outcome: crate::sync_state::Outcome::Complete,
                    message: None,
                },
                run.finished_at,
            )?;
            crate::sync_state::record_run(transaction, run)?;
            catalog_offering_in(transaction, id)
        })
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
        Ok(
            read_setting(&self.open()?, &search_options_setting(year))?
                .unwrap_or_else(|| json!([])),
        )
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

/// Merges one offering into `offerings` and its search index.
fn upsert_offering_in(transaction: &Transaction<'_>, offering: &Value) -> Result<bool, DataError> {
    let mut offering = offering.clone();
    let offering_id = string_field(&offering, "id")?.to_string();
    let search_item: Option<String> = transaction
        .query_row(
            "SELECT value FROM catalog_search_items WHERE id=?1",
            [&offering_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(DataError::from)?;
    if let Some(search_item) = search_item.and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
    {
        for field in ["title", "instructors", "academicUnit"] {
            if let (Some(current), Some(fallback)) =
                (offering.get_mut(field), search_item.get(field))
            {
                fill_missing(current, fallback);
            }
        }
    }
    let existing: Option<String> = transaction
        .query_row(
            "SELECT value FROM offerings WHERE id=?1",
            [&offering_id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(existing) = existing
        .map(|raw| serde_json::from_str::<Value>(&raw))
        .transpose()?
    {
        merge_official_fields(&mut offering, &existing);
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
            offering["academicUnit"] = department["name"].clone();
        }
    }
    let timestamp = now();
    let source_key = format!(
        "{}:{}:offering:{}",
        if offering.get("kyomu").is_some() {
            "kyomu"
        } else {
            "syllabus"
        },
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
    object.entry("rev").or_insert(Value::from(1));
    object
        .entry("updatedAt")
        .or_insert(Value::String(timestamp.clone()));
    object
        .entry("updatedBy")
        .or_insert(Value::String("catalog".into()));
    object
        .entry("sourceKey")
        .or_insert(Value::String(source_key));
    object
        .entry("lastSeenAt")
        .or_insert(Value::String(timestamp.clone()));
    serde_json::from_value::<crate::contracts::Offering>(offering.clone())
        .map_err(|error| format!("offering validation failed: {error}"))?;
    let changed = upsert_row(transaction, "offerings", &offering, true)?;

    let id = string_field(&offering, "id")?;
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
    Ok(changed)
}

fn upsert_offering_content_in(
    transaction: &Transaction<'_>,
    rows: &[Value],
) -> Result<(), DataError> {
    for row in rows {
        let offering_id = string_field(row, "offeringId")?;
        let lang = string_field(row, "lang")?;
        if !matches!(lang, "ja" | "en") {
            return Err("offering content language must be ja or en".into());
        }
        let content = row
            .get("content")
            .ok_or_else(|| "offering content is missing content".to_string())?;
        let fetched_at = content
            .get("fetchedAt")
            .and_then(Value::as_str)
            .unwrap_or("");
        transaction
            .execute(
                "INSERT INTO offering_content(offering_id, lang, content, fetched_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(offering_id, lang) DO UPDATE SET
                 content=excluded.content, fetched_at=excluded.fetched_at",
                params![
                    offering_id,
                    lang,
                    serde_json::to_string(content).map_err(DataError::from)?,
                    fetched_at
                ],
            )
            .map_err(DataError::from)?;
    }
    Ok(())
}

impl DataStore {
    /// Search rows are deliberately stored apart from complete offerings.
    /// A partial result can therefore never masquerade as a persisted entity.
    pub fn catalog_upsert_search_items(&self, items: &[Value]) -> Result<(), DataError> {
        self.write(None, |transaction| {
        for item in items {
            validate_contract::<crate::contracts::CatalogSearchItem>("catalog_search_items", item)?;
            let id = string_field(item, "id")?;
            let code = item.get("code").and_then(Value::as_str).unwrap_or_default();
            let title_ja = item
                .pointer("/title/ja")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let title_en = item
                .pointer("/title/en")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let instructors_ja = item
                .get("instructors")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|name| name.get("ja").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(" ");
            let instructors_en = item
                .get("instructors")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|name| name.get("en").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(" ");
            transaction
                .execute(
                    "INSERT INTO catalog_search_items(id,value,updated_at) VALUES(?1,?2,?3)
                 ON CONFLICT(id) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at",
                    params![
                        id,
                        serde_json::to_string(item).map_err(DataError::from)?,
                        now()
                    ],
                )
                .map_err(DataError::from)?;
            transaction
                .execute("DELETE FROM offerings_fts WHERE offering_id=?1", [id])
                .map_err(DataError::from)?;
            transaction.execute(
                "INSERT INTO offerings_fts(offering_id,code,title_ja,title_en,instructors_ja,instructors_en)
                 VALUES(?1,?2,?3,?4,?5,?6)",
                params![id,code,title_ja,title_en,instructors_ja,instructors_en],
            ).map_err(DataError::from)?;
        }
        Ok(())
        })
    }

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
                "SELECT COALESCE(o.value,s.value) FROM offerings_fts f
                 LEFT JOIN offerings o ON o.id=f.offering_id AND o.deleted_at IS NULL AND o.missing_since IS NULL
                 LEFT JOIN catalog_search_items s ON s.id=f.offering_id
                 WHERE offerings_fts MATCH ?1 AND (o.id IS NOT NULL OR s.id IS NOT NULL)
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

    /// A syllabus search row stored before its offering's details are read.
    pub fn catalog_search_item(&self, id: &str) -> Result<Option<Value>, DataError> {
        let connection = self.open()?;
        let raw: Option<String> = connection
            .query_row(
                "SELECT value FROM catalog_search_items WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(DataError::from)?;
        raw.map(|raw| {
            serde_json::from_str(&raw)
                .map_err(|_| DataError::storage("search row contains invalid JSON"))
        })
        .transpose()
    }
}

/// 教務 owns delivery facts. Syllabus can add bilingual names and content,
/// but a later detail refresh can never restore stale times or credits.
fn merge_official_fields(incoming: &mut Value, stored: &Value) {
    let incoming_official = incoming.get("kyomu").is_some();
    if incoming_official {
        for field in [
            "format",
            "media",
            "classLabel",
            "fetchedAt",
            "sourceUpdated",
            "unitCode",
            "academicUnit",
        ] {
            if let Some(value) = stored.get(field) {
                incoming[field] = value.clone();
            }
        }
        if let Some(en) = stored.pointer("/title/en") {
            incoming["title"]["en"] = en.clone();
        }
    } else if stored.get("kyomu").is_some() {
        for field in [
            "year",
            "quarters",
            "code",
            "credits",
            "schedule",
            "instructionLanguage",
            "intensive",
            "kyomu",
            "unitCode",
            "academicUnit",
        ] {
            if let Some(value) = stored.get(field) {
                incoming[field] = value.clone();
            }
        }
        if let Some(ja) = stored.pointer("/title/ja") {
            incoming["title"]["ja"] = ja.clone();
        }
        incoming["instructors"] = stored["instructors"].clone();
    }
}
