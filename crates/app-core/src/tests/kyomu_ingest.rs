//! 教務Web's student pages carry a person's grades and records, so none is
//! committed. This test reads the pages `adpt-isct`'s
//! `captured_student_pages_read_through_the_menu_and_the_trust_boundary`
//! reads, from the same `KYOMU_CAPTURES` directory: `student-top.html` and
//! `<kind>.html` for each kind in `adpt_isct::kyomu::PAGES`, saved as
//! `crates/adpt-isct/src/tests/kyomu.rs` describes. It is compiled only when
//! the directory exists, and a file
//! missing from it fails its test (`kyomu_captures`, `build.rs`).
//!
//! `KYOMU_CAPTURES=/path/to/dir cargo test -p app-core --lib kyomu_ingest::tests`
use super::*;
use adpt_isct::kyomu::{extract_page, parse_payload, KyomuStage, Menu, PAGES, STUDENT_TOP};
use uuid::Uuid;

fn page_state(store: &crate::data::DataStore, kind: &str) -> Option<sync_state::KeyState> {
    let connection = store.test_connection();
    sync_state::get(&connection, &Key::new(Dataset::KyomuPage, kind)).unwrap()
}

fn capture(name: &str) -> String {
    let path = std::path::Path::new(env!("KYOMU_CAPTURES")).join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// The number of grade rows a grades page lists, over all its groups.
fn grade_rows(data: &Value) -> usize {
    data["groups"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|group| group["rows"].as_array().map_or(0, Vec::len))
        .sum()
}

/// Each declared category with a stated total adds up to it (two decimals
/// per row, so a third of a credit may lose a hundredth per row).
fn assert_declared_categories_add_up(data: &Value) {
    let rows = declared_credits(data);
    let mut units: HashMap<&str, (f64, f64, f64, f64)> = HashMap::new();
    for row in &rows {
        let Some(total) = row["unitTotal"].as_f64() else {
            continue;
        };
        let listed: f64 = ["lecture", "exercise", "lab"]
            .iter()
            .filter_map(|key| row["credits"][*key].as_f64())
            .sum();
        let unit = units
            .entry(row["unit"].as_str().unwrap_or_default())
            .or_default();
        unit.0 = total;
        unit.1 += listed;
        unit.2 += row["effectiveCredits"].as_f64().unwrap();
        unit.3 += 1.0;
    }
    for (unit, (total, listed, effective, count)) in units {
        if listed > 0.0 {
            assert!(
                (effective - total).abs() <= 0.01 * count,
                "{unit}: {effective} credits counted of {total}"
            );
        }
    }
}

/// One account's real pages committed as a 教務 read commits them. Each page
/// the school offers commits its rows and its freshness together, and the
/// same page read again changes nothing (2026-11-22: every GPA read rewrote
/// its rows and pushed them to every shell). Every grade row keeps an
/// identity of its own (a repeated one rolls the whole page back), each
/// declared category adds up to the total the page states, and the GPA table
/// gives the whole-programme scope. A page the school does not offer now is
/// refused as rows and recorded as unavailable, so no emptiness is
/// manufactured. No page writes the person's courses, memberships or plans
/// (a source projection is ephemeral), and the official academic identity
/// fills a fresh profile without overwriting what the person saved.
#[test]
fn captured_student_pages_commit_once_and_keep_the_persons_choices() {
    let root = std::env::temp_dir().join(format!("isct-kyomu-captures-{}", Uuid::new_v4()));
    let store = crate::data::DataStore::initialize_dir(root.clone())
        .unwrap()
        .0;
    let menu = Menu::read(STUDENT_TOP, &capture("student-top.html")).unwrap();
    let mut profile = None;
    for page in &PAGES {
        let kind = page.kind;
        let url = menu
            .url(kind)
            .unwrap_or_else(|error| panic!("{kind}: {error}"));
        let read = extract_page(&url, &capture(&format!("{kind}.html")));
        let (stage, payload) = parse_payload(&serde_json::to_string(&read).unwrap())
            .unwrap_or_else(|error| panic!("{kind}: {error}"));
        let payload = payload.unwrap_or_else(|| panic!("{kind}: no payload"));
        let apply =
            |payload: &KyomuPayload| store.write(None, |tx| apply_kyomu_payload(tx, payload));

        if stage == KyomuStage::Unavailable {
            assert!(apply(&payload).is_err(), "{kind}");
            store
                .write(None, |tx| record_kyomu_unavailable(tx, &payload))
                .unwrap();
            let state = page_state(&store, kind).unwrap();
            assert_eq!(state.outcome, Some(Outcome::Unavailable), "{kind}");
            continue;
        }
        apply(&payload).unwrap_or_else(|error| panic!("{kind}: {error}"));
        let state = page_state(&store, kind).unwrap();
        assert_eq!(state.outcome, Some(Outcome::Complete), "{kind}");
        assert!(state.fetched_at.is_some(), "{kind}");
        assert_eq!(apply(&payload).unwrap(), 0, "{kind} read again");
        match kind {
            "grades" => assert_eq!(
                store.table_rows("grades").unwrap().len(),
                grade_rows(&payload.data)
            ),
            "declared" => assert_declared_categories_add_up(&payload.data),
            "gpa" => assert!(store
                .table_rows("gpa_summary")
                .unwrap()
                .iter()
                .any(|row| row["scope"] == "all")),
            "profile" => profile = Some(payload),
            _ => {}
        }
    }
    for table in ["manual_courses", "manual_memberships", "plan_items"] {
        assert!(store.user_rows(table).unwrap().is_empty(), "{table}");
    }

    let profile = profile.expect("the profile page is offered in every season");
    let workspace = store.load_workspace().unwrap();
    let department = workspace["settings"]["departmentId"]
        .as_str()
        .expect("the official department names one of the app's")
        .to_owned();
    let admitted = workspace["academicProfile"]["admissionYear"]
        .as_i64()
        .expect("the official admission date gives the admission year");
    let chosen = if department == "cs" { "mcs" } else { "cs" };
    store
        .put_workspace_setting("userPreferences", json!({"departmentId": chosen}))
        .unwrap();
    let mut saved = workspace["academicProfile"].clone();
    saved["admissionYear"] = json!(admitted - 1);
    store
        .put_workspace_setting("academicProfile", saved)
        .unwrap();
    store
        .write(None, |tx| apply_kyomu_payload(tx, &profile))
        .unwrap();
    let updated = store.load_workspace().unwrap();
    assert_eq!(updated["settings"]["departmentId"], chosen);
    assert_eq!(updated["academicProfile"]["admissionYear"], admitted - 1);
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}
