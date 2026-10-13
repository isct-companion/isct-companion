use super::DataError;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldChange {
    pub path: String,
    pub current: Option<Value>,
    pub incoming: Option<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeMethod {
    UseLatest,
    UseCurrent,
    ResolveManually,
}

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

#[derive(Clone, Debug, PartialEq)]
pub struct MergeResult {
    pub result: VersionedValue,
    pub changed: bool,
    pub conflict: Vec<FieldChange>,
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

pub fn canonicalize(value: &Value) -> Value {
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

pub fn canonical_json(value: &Value) -> Result<String, DataError> {
    serde_json::to_string(&canonicalize(value))
        .map_err(|_| DataError::storage("value cannot be serialized as canonical JSON"))
}

pub fn content_hash(value: &Value) -> Result<String, DataError> {
    let bytes = canonical_json(value)?;
    Ok(format!("{:x}", Sha256::digest(bytes.as_bytes())))
}

fn join_path(parent: &str, child: &str) -> String {
    if parent.is_empty() {
        child.to_string()
    } else {
        format!("{parent}.{child}")
    }
}

fn collect_diff(
    current: Option<&Value>,
    incoming: Option<&Value>,
    path: &str,
    out: &mut Vec<FieldChange>,
) {
    match (current, incoming) {
        (Some(Value::Object(a)), Some(Value::Object(b))) => {
            let keys: BTreeSet<_> = a.keys().chain(b.keys()).collect();
            for key in keys {
                collect_diff(a.get(key), b.get(key), &join_path(path, key), out);
            }
        }
        (Some(Value::Array(a)), Some(Value::Array(b))) => {
            let length = a.len().max(b.len());
            for index in 0..length {
                collect_diff(
                    a.get(index),
                    b.get(index),
                    &join_path(path, &index.to_string()),
                    out,
                );
            }
        }
        (a, b) if a == b => {}
        (a, b) => out.push(FieldChange {
            path: path.to_string(),
            current: a.cloned(),
            incoming: b.cloned(),
        }),
    }
}

pub fn diff(current: &Value, incoming: &Value) -> Vec<FieldChange> {
    let mut changes = Vec::new();
    collect_diff(Some(current), Some(incoming), "", &mut changes);
    changes
}

pub fn update(
    current: &VersionedValue,
    incoming: &VersionedValue,
    method: MergeMethod,
) -> MergeResult {
    if current.hash == incoming.hash && current.deleted_at == incoming.deleted_at {
        return MergeResult {
            result: current.clone(),
            changed: false,
            conflict: Vec::new(),
        };
    }
    let conflict = diff(&current.value, &incoming.value);
    let use_incoming = match method {
        MergeMethod::UseCurrent | MergeMethod::ResolveManually => false,
        MergeMethod::UseLatest => {
            incoming.updated_at > current.updated_at
                || (incoming.updated_at == current.updated_at && incoming.rev > current.rev)
        }
    };
    MergeResult {
        result: if use_incoming {
            incoming.clone()
        } else {
            current.clone()
        },
        changed: use_incoming,
        conflict: if method == MergeMethod::ResolveManually {
            conflict
        } else {
            Vec::new()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(value: Value, updated_at: &str, rev: u64) -> VersionedValue {
        VersionedValue {
            hash: content_hash(&value).unwrap(),
            value,
            updated_at: updated_at.into(),
            updated_by: "test".into(),
            rev,
            source: None,
            deleted_at: None,
        }
    }

    #[test]
    fn canonical_hash_is_stable() {
        let first: Value =
            serde_json::from_str(r#"{ "b": 2.0, "a": { "y": true, "x": 1 } }"#).unwrap();
        let second: Value = serde_json::from_str(r#"{"a":{"x":1.0,"y":true},"b":2}"#).unwrap();
        assert_eq!(
            content_hash(&first).unwrap(),
            content_hash(&second).unwrap()
        );
    }

    #[test]
    fn diff_has_stable_field_order() {
        let changes = diff(
            &json!({"z": 1, "profile": {"name": "old", "year": 1}}),
            &json!({"a": true, "profile": {"name": "new", "year": 1}}),
        );
        assert_eq!(
            changes
                .iter()
                .map(|change| change.path.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "profile.name", "z"]
        );
    }

    #[test]
    fn update_honors_both_automatic_methods() {
        let current = row(json!({"name": "old"}), "2026-01-01T00:00:00Z", 1);
        let incoming = row(json!({"name": "new"}), "2026-01-02T00:00:00Z", 2);
        assert!(update(&current, &incoming, MergeMethod::UseLatest).changed);
        assert!(!update(&current, &incoming, MergeMethod::UseCurrent).changed);
        let manual = update(&current, &incoming, MergeMethod::ResolveManually);
        assert_eq!(manual.conflict[0].path, "name");
    }
}
