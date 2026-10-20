use super::*;
use serde_json::json;

#[test]
fn rest_errors_arrive_as_http_200_bodies_and_are_classified_before_parsing() {
    let cases = [
        (
            json!({"exception":"moodle_exception","errorcode":"invalidtoken","message":"Invalid token"}),
            EnvelopeError::InvalidToken,
        ),
        (
            json!({"exception":"webservice_access_exception","errorcode":"accessexception","message":"x"}),
            EnvelopeError::InvalidToken,
        ),
        (
            json!({"exception":"moodle_exception","errorcode":"servicenotavailable"}),
            EnvelopeError::ServiceUnavailable("servicenotavailable".into()),
        ),
        (
            json!({"exception":"moodle_exception","errorcode":"sitemaintenance"}),
            EnvelopeError::Maintenance,
        ),
        (
            json!({"exception":"invalid_parameter_exception","errorcode":"invalidparameter","message":"secret echo","debuginfo":"more"}),
            EnvelopeError::Api("invalidparameter".into()),
        ),
    ];
    for (body, expected) in cases {
        assert_eq!(
            rest_result(body.to_string().as_bytes()).unwrap_err(),
            expected
        );
    }
    // A gateway or login page is not JSON.
    assert_eq!(
        rest_result(b"<!doctype html><title>403</title>").unwrap_err(),
        EnvelopeError::NotJson
    );
    // Success shapes: arrays, objects, and functions that return null.
    assert_eq!(rest_result(b"[{\"id\":1}]").unwrap(), json!([{"id": 1}]));
    assert_eq!(rest_result(b"null").unwrap(), serde_json::Value::Null);
    let partial =
        rest_result(br#"{"courses":[],"warnings":[{"warningcode":"1","message":"m"}]}"#).unwrap();
    assert!(has_warnings(&partial));
    assert!(!has_warnings(&json!({"courses": [], "warnings": []})));
    // A course the student cannot open is expected, not missing data.
    assert!(!has_warnings(&json!({"warnings": [
        {"item": "course", "itemid": 7, "warningcode": "1", "message": "No access rights"}
    ]})));
}

#[test]
fn ajax_batches_report_the_first_call_and_whole_request_failures() {
    assert_eq!(
        ajax_result(br#"[{"error":false,"data":{"courses":[]}}]"#).unwrap(),
        json!({"courses": []})
    );
    assert_eq!(
        ajax_result(
            br#"[{"error":true,"exception":{"errorcode":"invalidsesskey","message":"m"}}]"#
        )
        .unwrap_err(),
        EnvelopeError::InvalidSesskey
    );
    assert_eq!(
        ajax_result(br#"[{"error":true,"exception":{"errorcode":"servicenotavailable"}}]"#)
            .unwrap_err(),
        EnvelopeError::ServiceUnavailable("servicenotavailable".into())
    );
    assert_eq!(
        ajax_result(br#"{"error":"x","errorcode":"servicerequireslogin"}"#).unwrap_err(),
        EnvelopeError::SignInRequired
    );
    assert_eq!(ajax_result(b"[]").unwrap_err(), EnvelopeError::NotJson);
    assert_eq!(ajax_result(b"<html>").unwrap_err(), EnvelopeError::NotJson);
}

#[test]
fn rest_arguments_use_moodles_indexed_form_names() {
    let fields = form_fields(&json!({
        "courseids": [3, 5],
        "options": [{"name": "excludemodules", "value": true}],
        "status": "all",
        "skipped": null,
    }));
    let mut fields: Vec<(&str, &str)> = fields
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    fields.sort();
    assert_eq!(
        fields,
        [
            ("courseids[0]", "3"),
            ("courseids[1]", "5"),
            ("options[0][name]", "excludemodules"),
            ("options[0][value]", "1"),
            ("status", "all"),
        ]
    );
}
