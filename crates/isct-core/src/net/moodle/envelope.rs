//! Moodle's two response envelopes and the REST form encoding. Pure: no I/O.
use serde_json::Value;

/// Why a Moodle response carried no usable data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum EnvelopeError {
    /// The body is not JSON: a login page, a gateway page or a truncated body.
    NotJson,
    /// The token was refused (`invalidtoken`, `webservice_access_exception`).
    /// The response is still HTTP 200.
    InvalidToken,
    /// The mobile service or this function is not offered to this client.
    ServiceUnavailable(String),
    /// The AJAX sesskey expired or does not belong to this session.
    InvalidSesskey,
    /// The session behind the AJAX call is gone.
    SignInRequired,
    /// The site is in maintenance mode.
    Maintenance,
    /// Any other Moodle exception, by its error code. `message` and
    /// `debuginfo` are never kept: they may echo request values.
    Api(String),
}

fn classify(exception: &str, errorcode: &str) -> EnvelopeError {
    match (exception, errorcode) {
        (_, "invalidtoken") | ("webservice_access_exception", _) | (_, "accessexception") => {
            EnvelopeError::InvalidToken
        }
        (_, "servicenotavailable" | "enablewsdescription" | "wsdocumentationdisable")
        | (_, "webservicesnotenabled" | "mobileserviceisnotenabled") => {
            EnvelopeError::ServiceUnavailable(errorcode.to_string())
        }
        (_, "invalidsesskey") => EnvelopeError::InvalidSesskey,
        (_, "servicerequireslogin" | "requireloginerror") => EnvelopeError::SignInRequired,
        (_, "sitemaintenance") => EnvelopeError::Maintenance,
        _ => EnvelopeError::Api(if errorcode.is_empty() {
            "unknown".into()
        } else {
            errorcode.chars().take(64).collect()
        }),
    }
}

/// A REST (`webservice/rest/server.php`) body. Errors arrive as HTTP 200 with
/// `{exception, errorcode, message}`, so this runs before any success parsing.
pub(crate) fn rest_result(body: &[u8]) -> Result<Value, EnvelopeError> {
    let value: Value = serde_json::from_slice(body).map_err(|_| EnvelopeError::NotJson)?;
    if let Some(object) = value.as_object() {
        let errorcode = object.get("errorcode").and_then(Value::as_str);
        let exception = object.get("exception").and_then(Value::as_str);
        if errorcode.is_some() || exception.is_some() {
            return Err(classify(exception.unwrap_or(""), errorcode.unwrap_or("")));
        }
    }
    Ok(value)
}

/// An AJAX (`lib/ajax/service.php`) body for a single-call batch.
pub(crate) fn ajax_result(body: &[u8]) -> Result<Value, EnvelopeError> {
    let value: Value = serde_json::from_slice(body).map_err(|_| EnvelopeError::NotJson)?;
    // A whole-request failure is a bare object, not a batch.
    if let Some(object) = value.as_object() {
        let errorcode = object
            .get("errorcode")
            .and_then(Value::as_str)
            .unwrap_or("");
        let exception = object
            .get("exception")
            .and_then(Value::as_str)
            .unwrap_or("");
        return Err(classify(exception, errorcode));
    }
    let first = value
        .as_array()
        .and_then(|batch| batch.first())
        .ok_or(EnvelopeError::NotJson)?;
    if first.get("error").and_then(Value::as_bool) == Some(true) {
        let exception = first.get("exception");
        let errorcode = exception
            .and_then(|e| e.get("errorcode"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let name = exception
            .and_then(|e| e.get("exception"))
            .and_then(Value::as_str)
            .unwrap_or("");
        return Err(classify(name, errorcode));
    }
    Ok(first.get("data").cloned().unwrap_or(Value::Null))
}

/// Whether a response's `warnings` mean data this key needed is missing.
///
/// The `*_by_courses` functions warn once per course the student cannot open
/// (`item: "course"`, e.g. a hidden or ended course). Those courses are not
/// readable by any transport, so they are expected rather than missing. Every
/// warning is logged by item, id and code; messages and values are not.
pub(crate) fn has_warnings(value: &Value) -> bool {
    let Some(warnings) = value.get("warnings").and_then(Value::as_array) else {
        return false;
    };
    let mut missing = false;
    for warning in warnings {
        let item = warning.get("item").and_then(Value::as_str).unwrap_or("");
        let code = warning
            .get("warningcode")
            .map(|code| code.to_string())
            .unwrap_or_default();
        let id = warning.get("itemid").map(|id| id.to_string()).unwrap_or_default();
        log::info!(target: "isct_core::net::moodle", "LMS warning item={item} itemid={id} code={code}");
        if item != "course" {
            missing = true;
        }
    }
    missing
}

/// Moodle's form encoding of external-function arguments: arrays as
/// `name[0]`, objects as `name[key]`, booleans as `1`/`0`, nulls omitted.
pub(crate) fn form_fields(args: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(object) = args.as_object() {
        for (key, value) in object {
            flatten(key.clone(), value, &mut out);
        }
    }
    out
}

fn flatten(name: String, value: &Value, out: &mut Vec<(String, String)>) {
    match value {
        Value::Null => {}
        Value::Bool(flag) => out.push((name, if *flag { "1" } else { "0" }.into())),
        Value::Number(number) => out.push((name, number.to_string())),
        Value::String(text) => out.push((name, text.clone())),
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                flatten(format!("{name}[{index}]"), item, out);
            }
        }
        Value::Object(fields) => {
            for (key, item) in fields {
                flatten(format!("{name}[{key}]"), item, out);
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/net__moodle__envelope.rs"]
mod tests;
