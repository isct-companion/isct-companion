//! The `moodle_envelope_captures` test reads Moodle's answers to requests
//! without an account, saved with `curl -s` into the directory in
//! `MOODLE_ENVELOPE_CAPTURES`, and is compiled only when it holds all three
//! (`build.rs`):
//!
//! - `rest-invalidtoken.json`: `-d wstoken=0 -d wsfunction=core_webservice_get_site_info
//!   -d moodlewsrestformat=json https://lms.s.isct.ac.jp/2025/webservice/rest/server.php`;
//! - `ajax-requireslogin.json`: `-H 'Content-Type: application/json'
//!   -d '[{"index":0,"methodname":"core_calendar_get_action_events_by_timesort","args":{}}]'
//!   'https://lms.s.isct.ac.jp/2025/lib/ajax/service.php?sesskey=0'`;
//! - `login-page.html`: `https://lms.s.isct.ac.jp/2025/login/index.php`.
//!
//! None carries an account's data.
//!
//! `MOODLE_ENVELOPE_CAPTURES=/path/to/dir cargo test -p app-core --lib net::moodle::envelope`
use super::*;
use serde_json::json;

#[cfg(moodle_envelope_captures)]
fn capture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("MOODLE_ENVELOPE_CAPTURES")).join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// Moodle's errors arrive as HTTP 200 bodies and are classified before any
/// success parsing: a refused token sends the client to launch a new one,
/// a lost session to sign in again, and a page that is not JSON (a sign-in
/// or gateway page) is neither.
#[test]
#[cfg(moodle_envelope_captures)]
fn captured_moodle_refusals_are_classified() {
    assert_eq!(
        rest_result(&capture("rest-invalidtoken.json")).unwrap_err(),
        EnvelopeError::InvalidToken
    );
    assert_eq!(
        ajax_result(&capture("ajax-requireslogin.json")).unwrap_err(),
        EnvelopeError::SignInRequired
    );
    let page = capture("login-page.html");
    assert_eq!(rest_result(&page).unwrap_err(), EnvelopeError::NotJson);
    assert_eq!(ajax_result(&page).unwrap_err(), EnvelopeError::NotJson);
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
