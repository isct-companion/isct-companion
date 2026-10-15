use serde::{Deserialize, Serialize};
/// Core's deliberately dynamic payload fields, carried as validated JSON text.
/// The service and intent/query discriminants themselves remain typed.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JsonValue(pub serde_json::Value);
uniffi::custom_type!(JsonValue, String, {
    try_lift: |text| Ok(JsonValue(serde_json::from_str(&text)?)),
    lower: |value| value.0.to_string(),
});
pub(crate) fn project<T: Serialize, U: serde::de::DeserializeOwned>(
    value: T,
) -> Result<U, crate::SdkError> {
    let value = serde_json::to_value(value).map_err(|e| crate::SdkError::Internal {
        detail: e.to_string(),
    })?;
    serde_json::from_value(value).map_err(|e| crate::SdkError::InvalidInput {
        detail: e.to_string(),
    })
}
