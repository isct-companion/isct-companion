use super::DataError;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionedValue {
    pub value: Value,
    pub hash: String,
    pub updated_at: String,
    pub updated_by: String,
    pub rev: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
}

fn canonical_number(number: &Number) -> Value {
    if let Some(value) = number.as_i64() {
        return Value::Number(value.into());
    }
    if let Some(value) = number.as_u64() {
        return Value::Number(value.into());
    }
    let value = number.as_f64().unwrap_or_default();
    if value.fract() == 0.0 && value >= i64::MIN as f64 && value <= i64::MAX as f64 {
        return Value::Number((value as i64).into());
    }
    Number::from_f64(value).map_or(Value::Null, Value::Number)
}

fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys: Vec<_> = object.keys().collect();
            keys.sort_unstable();
            let mut sorted = Map::new();
            for key in keys {
                sorted.insert(key.clone(), canonicalize(&object[key]));
            }
            Value::Object(sorted)
        }
        Value::Array(values) => Value::Array(values.iter().map(canonicalize).collect()),
        Value::Number(number) => canonical_number(number),
        other => other.clone(),
    }
}

fn canonical_json(value: &Value) -> Result<String, DataError> {
    serde_json::to_string(&canonicalize(value))
        .map_err(|_| DataError::storage("value cannot be serialized as canonical JSON"))
}

pub fn content_hash(value: &Value) -> Result<String, DataError> {
    let bytes = canonical_json(value)?;
    Ok(data_encoding::HEXLOWER.encode(&Sha256::digest(bytes.as_bytes())))
}

/// A school row a complete read found with its stored hash. A row that had
/// gone missing or been deleted is written, to bring it back; returns
/// whether it was. Otherwise nothing is written, so a read that finds
/// nothing new commits no record row and no shell hears of it. When such a
/// row was last seen follows from its key's freshness: a row that is not
/// missing was in the latest complete read of its key.
pub(crate) fn keep_unchanged(
    connection: &rusqlite::Connection,
    table: &str,
    id: &str,
    revive: bool,
    timestamp: &str,
) -> rusqlite::Result<bool> {
    if !revive {
        return Ok(false);
    }
    connection.execute(
        &format!(
            "UPDATE {table} SET last_seen_at=?2, missing_since=NULL, deleted_at=NULL,
             value=json_remove(value,'$.missingSince','$.deletedAt') WHERE id=?1"
        ),
        rusqlite::params![id, timestamp],
    )?;
    Ok(true)
}

#[cfg(test)]
#[path = "../tests/data__versioned.rs"]
mod tests;
