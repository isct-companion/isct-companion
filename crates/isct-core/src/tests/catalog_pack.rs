use super::*;
use serde_json::json;

fn offering(id: &str, code: &str, title_en: &str) -> Value {
    json!({
        "id": id, "year": 2026, "quarters": ["3Q"], "code": code,
        "title": {"ja": format!("{title_en}（シラバス）"), "en": title_en},
        "instructors": [{"ja": "教員", "en": "Teacher"}], "instructionLanguage": "en",
        "format": {"kinds": ["lecture"]}, "media": {"ja": "対面", "en": "Face-to-face"},
        "credits": {"lecture": 2, "exercise": 0, "lab": 0},
        "schedule": [{"kind": "weekly", "day": 5, "periods": [5, 6], "room": {"en": "Old room"}}],
        "academicUnit": {"ja": "情報工学系", "en": "Computer Science"}, "intensive": false,
        "syllabusUrl": format!("https://syllabus.s.isct.ac.jp/courses/2026/4/0-904-342300-0-0/{id}"),
        "fetchedAt": {"ja": "2026-09-01T00:00:00Z", "en": "2026-09-01T00:00:00Z"}
    })
}

fn pack(offerings: &[Value]) -> (CatalogPack, PackEntry) {
    let pack: CatalogPack = serde_json::from_value(json!({
        "schemaVersion": 1, "academicYear": 2026, "generatedAt": "2026-09-01T00:00:00Z",
        "source": {"title": "Science Tokyo シラバス", "url": "https://syllabus.s.isct.ac.jp/courses/2026"},
        "scopes": [{"scope": "4/0-904-342300-0-0", "listed": offerings.len(), "truncated": false}],
        "offerings": offerings
    }))
    .unwrap();
    // The published file decodes to the same pack under its index hash, and
    // under no other.
    let (bytes, hash) = pack.encode().unwrap();
    assert!(matches!(
        CatalogPack::decode(&bytes, &"0".repeat(64), 2026),
        Err(PackError::HashMismatch)
    ));
    let decoded = CatalogPack::decode(&bytes, &hash, 2026).unwrap();
    assert_eq!(decoded.offerings.len(), offerings.len());
    let entry = PackEntry {
        source: "catalog/2026.json.gz".into(),
        hash,
        ts: "2026-09-01T00:00:00Z".into(),
        count: offerings.len(),
    };
    (decoded, entry)
}

#[test]
fn a_pack_imports_once_renews_on_its_hash_holds_a_shrink_and_keeps_kyomu_fields() {
    let root = std::env::temp_dir().join(format!("isct-pack-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let store = DataStore::initialize(root.join("school.db")).unwrap().0;
    // 教務 listed one course before the pack arrived.
    let mut official = offering("202602431", "CSC.T273", "");
    official["title"] = json!({"ja": "オブジェクト指向プログラミング"});
    official["credits"] = json!({"lecture": 1, "exercise": 1, "lab": 0});
    official["schedule"] =
        json!([{"kind": "weekly", "day": 1, "periods": [3, 4], "room": {"ja": "M-B07"}}]);
    official["fetchedAt"] = json!({});
    official["kyomu"] = json!({"department": "情報工学系", "fetchedAt": "2026-10-16T00:00:00Z"});
    store.catalog_upsert_offerings(&[official.clone()]).unwrap();

    let rows: Vec<Value> = (0..10)
        .map(|index| {
            offering(
                &format!("2026024{:02}", 31 + index),
                &format!("CSC.T{}", 273 + index),
                &format!("Pack course {index}"),
            )
        })
        .collect();
    let (full, full_entry) = pack(&rows);
    let key = pack_key(2026);
    let lease = Lease::detached(&store, key.clone(), "");
    let writer = lease.writer();
    let apply = |pack: &CatalogPack, entry: &PackEntry, at: DateTime<Utc>| {
        writer
            .write(None, |transaction| import(transaction, &key, entry, pack, at))
            .unwrap()
    };
    let state = || {
        store
            .write(None, |transaction| sync_state::get(transaction, &key))
            .unwrap()
            .unwrap()
    };
    let live = || crate::catalog::offerings(&store, 2026).unwrap();
    let t0: DateTime<Utc> = "2026-10-20T00:00:00Z".parse().unwrap();

    // The pack imports in one commit; 教務 still owns timing, credits and
    // the Japanese title, the pack adds the English title, and the year's
    // search index finds the pack's rows.
    assert!(matches!(
        apply(&full, &full_entry, t0),
        Applied::Imported(PackImport { changed: 10, missing: 0 })
    ));
    assert_eq!(live().len(), 10);
    let merged = store.catalog_offering("202602431").unwrap().unwrap();
    assert_eq!(merged["schedule"], official["schedule"]);
    assert_eq!(merged["credits"], official["credits"]);
    assert_eq!(merged["title"]["ja"], official["title"]["ja"]);
    assert_eq!(merged["title"]["en"], "Pack course 0");
    assert_eq!(store.catalog_search("Pack course").unwrap().len(), 10);
    assert_eq!(state().validator.as_deref(), Some(full_entry.hash.as_str()));
    // Importing the same pack again writes nothing.
    assert!(matches!(
        apply(&full, &full_entry, t0),
        Applied::Imported(PackImport { changed: 0, missing: 0 })
    ));

    // The same hash only renews the key.
    let t1 = t0 + chrono::Duration::days(8);
    let due = store
        .write(None, |transaction| pack_due(transaction, &key, t1))
        .unwrap();
    assert_eq!(due, Due::Stale);
    assert!(store
        .write(None, |transaction| renew(transaction, &key, &full_entry, t1))
        .unwrap());
    assert_eq!(state().fetched_at, Some(t1));

    // A pack that lost 40% of the rows is held, with a backoff; the same
    // pack read again is accepted and marks the dropped rows missing,
    // except the course 教務 lists.
    let kept: Vec<Value> = rows[..6].to_vec();
    let (shrunk, shrunk_entry) = pack(&kept);
    assert_eq!(apply(&shrunk, &shrunk_entry, t1), Applied::Held);
    assert_eq!(live().len(), 10);
    let held = state();
    assert_eq!(
        (held.failures, held.last_error.as_deref()),
        (1, Some(SUSPICIOUS_SHRINK))
    );
    assert_eq!(held.validator.as_deref(), Some(full_entry.hash.as_str()));
    assert!(matches!(
        apply(&shrunk, &shrunk_entry, t1),
        Applied::Imported(PackImport { changed: 0, missing: 4 })
    ));
    assert_eq!(live().len(), 6);
    assert_eq!(store.catalog_search("Pack course").unwrap().len(), 6);

    // Missing content beats the validator: with the rows gone, the stored
    // hash no longer renews the key.
    store
        .write(None, |transaction| {
            transaction.execute("DELETE FROM offerings", [])?;
            Ok::<_, DataError>(())
        })
        .unwrap();
    assert!(!store
        .write(None, |transaction| renew(transaction, &key, &shrunk_entry, t1))
        .unwrap());
    std::fs::remove_dir_all(root).unwrap();
}
