use super::*;
use crate::data::DataStore;
use crate::sync_state::{Commit, Due, Outcome};
use serde_json::json;

fn commit(connection: &Connection, key: &Key, at: DateTime<Utc>) {
    sync_state::commit(
        connection,
        key,
        Commit {
            account: "science:student",
            validator: None,
            outcome: Outcome::Complete,
            message: None,
        },
        at,
    )
    .unwrap();
}

fn course(connection: &Connection, lms_id: i64) {
    let value = json!({"id": format!("lms-course:{lms_id}"), "lmsId": lms_id,
        "year": 2025, "quarters": ["1Q"], "rev": 1,
        "sections": [{"id": 1, "title": "Week 1", "bodyHtml": "<p>Welcome</p>"}]});
    connection
        .execute(
            "INSERT INTO lms_courses(id,lms_id,value,hash,updated_at,updated_by,rev)
             VALUES(?1,?2,?3,'old','2025-04-01T00:00:00+00:00','lms',1)",
            params![format!("lms-course:{lms_id}"), lms_id, value.to_string()],
        )
        .unwrap();
    let item = json!({"id": format!("lms-item:{lms_id}0"), "lmsCourseId": lms_id,
        "name": "Notes", "rev": 1, "bodyHtml": "<p>Notes</p>",
        "resourceUrl": "https://lms.s.isct.ac.jp/2025/notes.pdf", "folderFiles": [{"name": "a.pdf"}]});
    connection
        .execute(
            "INSERT INTO lms_items(id,cmid,value,hash,updated_at,updated_by,rev)
             VALUES(?1,?2,?3,'old','2025-04-01T00:00:00+00:00','lms',1)",
            params![format!("lms-item:{lms_id}0"), lms_id * 10, item.to_string()],
        )
        .unwrap();
}

fn value(connection: &Connection, table: &str, id: String) -> Value {
    let raw: String = connection
        .query_row(
            &format!("SELECT value FROM {table} WHERE id=?1"),
            [id],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&raw).unwrap()
}

/// Past course details, old シラバス details and old run records go, each
/// key reading as missing afterwards; anything opened recently stays, and a
/// second pass the same day does nothing.
#[test]
fn gc_drops_unused_content_and_invalidates_its_key_but_keeps_recently_used_keys() {
    let path = std::env::temp_dir().join(format!("isct-sync-gc-{}", uuid::Uuid::new_v4()));
    let store = DataStore::initialize_dir(path).unwrap().0;
    let now: DateTime<Utc> = "2026-10-20T03:00:00Z".parse().unwrap();
    let days = |n: i64| now - Duration::days(n);
    let connection = store.test_connection();
    // Courses 7 and 8 are from the previous academic year; 8 was opened
    // ten days ago.
    for lms_id in [7, 8] {
        course(&connection, lms_id);
        commit(
            &connection,
            &Key::new(Dataset::LmsCourse, lms_id.to_string()),
            days(200),
        );
    }
    sync_state::touch(&connection, &[Key::new(Dataset::LmsCourse, "8")], days(10)).unwrap();
    for offering in ["202600001", "202600002"] {
        connection
            .execute(
                "INSERT INTO offering_content(offering_id,lang,content,fetched_at)
                 VALUES(?1,'ja','{}',?2)",
                params![offering, days(400).to_rfc3339()],
            )
            .unwrap();
        commit(
            &connection,
            &Key::new(Dataset::CatalogDetail, offering),
            days(400),
        );
    }
    sync_state::touch(
        &connection,
        &[Key::new(Dataset::CatalogDetail, "202600002")],
        days(30),
    )
    .unwrap();
    for (run, finished) in [("old", days(31)), ("recent", days(1))] {
        connection
            .execute(
                "INSERT INTO sync_runs(run,dataset,key,started_at,finished_at,outcome)
                 VALUES(?1,'kyomu.page','gpa',?2,?2,'complete')",
                params![run, finished.to_rfc3339()],
            )
            .unwrap();
    }

    let report = store
        .write(None, |transaction| gc(transaction, now))
        .unwrap();
    assert_eq!(
        report,
        GcReport {
            ran: true,
            courses_stripped: 1,
            details_deleted: 1,
            runs_deleted: 1,
        }
    );

    let stripped = value(&connection, "lms_items", "lms-item:70".into());
    assert_eq!(stripped["bodyHtml"], Value::Null);
    assert_eq!(stripped["resourceUrl"], Value::Null);
    assert_eq!(stripped["folderFiles"], Value::Null);
    assert_eq!(stripped["name"], "Notes");
    let section = &value(&connection, "lms_courses", "lms-course:7".into())["sections"][0];
    assert_eq!(section["title"], "Week 1");
    assert!(section.get("bodyHtml").is_none());
    let hash: String = connection
        .query_row(
            "SELECT hash FROM lms_items WHERE id='lms-item:70'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_ne!(hash, "old");
    let state = |dataset: Dataset, key: &str| {
        sync_state::get(&connection, &Key::new(dataset, key))
            .unwrap()
            .unwrap()
    };
    let missing = |dataset: Dataset, key: &str| {
        sync_state::due(Some(&state(dataset, key)), None, "science:student", now) == Due::Missing
    };
    assert!(missing(Dataset::LmsCourse, "7"));
    assert!(missing(Dataset::CatalogDetail, "202600001"));

    // Recently opened keys keep their content and freshness.
    let kept = value(&connection, "lms_items", "lms-item:80".into());
    assert_eq!(kept["bodyHtml"], "<p>Notes</p>");
    assert!(!missing(Dataset::LmsCourse, "8"));
    let details: Vec<String> = connection
        .prepare("SELECT offering_id FROM offering_content")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(details, ["202600002"]);
    assert!(!missing(Dataset::CatalogDetail, "202600002"));
    let runs: Vec<String> = connection
        .prepare("SELECT run FROM sync_runs WHERE dataset='kyomu.page'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(runs, ["recent"]);

    // At most once a day.
    assert_eq!(
        store
            .write(None, |transaction| gc(
                transaction,
                now + Duration::hours(23)
            ))
            .unwrap(),
        GcReport::default()
    );
}
