use super::*;
#[test]
fn stable_update_admission_and_wakeups_respect_version_and_cadence() {
    assert!(newer_release("1.0.0-rc.1", "v1.0.0").unwrap());
    assert!(!newer_release("1.1.0", "v1.0.9").unwrap());
    assert!(!newer_release("1.0.0", "v1.0.0").unwrap());
    assert!(matches!(
        newer_release("1.0.0", "broken"),
        Err(AppUpdateError::Version(_))
    ));
    let now = "2026-10-16T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
    assert!(check_due(None, now, false));
    assert!(!check_due(Some("2026-10-16T01:00:00Z"), now, false));
    assert!(check_due(Some("2026-10-16T00:00:00Z"), now, false));
    assert!(check_due(Some("2026-10-16T11:59:00Z"), now, true));
    assert!(!automatic_updates(&serde_json::json!({"settings":{}})));
    assert!(automatic_updates(
        &serde_json::json!({"settings":{"automaticAppUpdates":true}})
    ));
}
