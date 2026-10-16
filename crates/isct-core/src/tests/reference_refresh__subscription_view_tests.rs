use super::*;
#[test]
fn follows_selected_year_without_study_guide_or_bad_fetch_dates() {
    let mut workspace = json!({"selectedTerm":"2026-3Q","currentTerm":"2025-4Q","academicProfile":{"admissionYear":2024},"subscriptions":[{"kind":"calendar","year":2026,"fetchedAt":"not-a-date"}]});
    let views = subscription_views(&workspace);
    assert!(views.get("departments").is_none());
    assert_eq!(views["calendar"]["year"], 2026);
    assert_eq!(views["calendar"]["state"]["state"], "pending");
    workspace["subscriptions"][0]["fetchedAt"] = json!("2026-10-06T12:00:00Z");
    assert_eq!(
        subscription_views(&workspace)["calendar"]["state"]["state"],
        "current"
    );
    workspace["subscriptions"][0]["year"] = json!(2025);
    assert_eq!(
        subscription_views(&workspace)["calendar"]["state"]["state"],
        "pending"
    );
    workspace["subscriptions"][0]["enabled"] = json!(false);
    assert_eq!(
        subscription_views(&workspace)["calendar"]["state"]["state"],
        "off"
    );
}
