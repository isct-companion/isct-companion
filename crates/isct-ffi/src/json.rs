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
    serde_json::from_value(value).map_err(|e| crate::SdkError::Internal {
        detail: e.to_string(),
    })
}

/// Foreign records can contain values Core cannot represent (for example a
/// vector of the wrong length for a fixed array). Only this direction is input.
pub(crate) fn project_input<T: Serialize, U: serde::de::DeserializeOwned>(
    value: T,
) -> Result<U, crate::SdkError> {
    let value = serde_json::to_value(value).map_err(|e| crate::SdkError::Internal {
        detail: e.to_string(),
    })?;
    serde_json::from_value(value).map_err(|e| crate::SdkError::InvalidInput {
        detail: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projection_errors_preserve_boundary_direction() {
        assert!(matches!(
            project::<_, [u8; 2]>(vec![1]),
            Err(crate::SdkError::Internal { .. })
        ));
        assert!(matches!(
            project_input::<_, [u8; 2]>(vec![1]),
            Err(crate::SdkError::InvalidInput { .. })
        ));
        assert_eq!(project_input::<_, [u8; 2]>(vec![1, 2]).unwrap(), [1, 2]);
    }
}
