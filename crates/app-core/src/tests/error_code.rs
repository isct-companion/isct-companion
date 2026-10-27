use super::*;

/// Shells show `error.<code>` from the shared catalogs and nothing else, so
/// every code needs copy in every language, and no copy may outlive its code.
#[test]
fn every_code_has_copy_in_every_locale() {
    let catalogs = [
        ("en", include_str!("../../../../resources/locales/en.json")),
        ("ja", include_str!("../../../../resources/locales/ja.json")),
        ("zh", include_str!("../../../../resources/locales/zh.json")),
    ];
    for (language, raw) in catalogs {
        let catalog: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(raw).unwrap();
        for code in ErrorCode::ALL {
            let key = format!("error.{}", code.name());
            assert!(
                catalog
                    .get(&key)
                    .and_then(|copy| copy.as_str())
                    .is_some_and(|copy| !copy.is_empty()),
                "{language} has no copy for {key}"
            );
        }
        for key in catalog.keys().filter(|key| key.starts_with("error.")) {
            assert!(
                ErrorCode::ALL
                    .iter()
                    .any(|code| key == &format!("error.{}", code.name())),
                "{language} keeps copy for a code Core no longer raises: {key}"
            );
        }
    }
}

/// The wire form is the code name, the values copy interpolates, and the
/// diagnostic text; serde and `name` must agree, or shells look up the
/// wrong key.
#[test]
fn failures_serialize_their_stable_code_and_params() {
    for code in ErrorCode::ALL {
        assert_eq!(serde_json::to_value(code).unwrap(), code.name());
    }
    let failure = CoreFailure::of(&StudyProfileError::DuplicateAdmissionTerm {
        term: "2026-1Q".into(),
    });
    assert_eq!(
        serde_json::to_value(&failure).unwrap(),
        serde_json::json!({
            "code": "study-duplicate-term",
            "params": { "term": "2026-1Q" },
            "detail": "study stages cannot share admission term 2026-1Q",
        })
    );
    let plain = CoreFailure::of(&PlanError::InvalidTitle);
    assert_eq!(
        serde_json::to_value(&plain).unwrap(),
        serde_json::json!({ "code": "plan-invalid-title", "detail": "Plan title is required." })
    );
    // A wrapped error reports the code of the failure inside it.
    let nested = ReadModelError::Schedule(ScheduleError::InvalidInput("bad".into()));
    assert_eq!(nested.code(), ErrorCode::InvalidInput);
}
