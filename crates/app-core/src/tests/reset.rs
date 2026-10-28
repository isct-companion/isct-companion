use super::*;
use crate::{
    ports::{SecretError, SecretVault},
    school_auth::{MoodleToken, SchoolCredentials},
    sync_state::{self, Commit, Dataset, Due, Key, Outcome},
};
use rusqlite::params;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    sync::atomic::{AtomicUsize, Ordering},
};

#[derive(Clone, Default)]
struct MemoryVault(Arc<Mutex<HashMap<String, String>>>);

impl SecretVault for MemoryVault {
    fn store(&self, account: &str, secret: &str) -> Result<(), SecretError> {
        self.0.lock().unwrap().insert(account.into(), secret.into());
        Ok(())
    }
    fn load(&self, account: &str) -> Result<Option<String>, SecretError> {
        Ok(self.0.lock().unwrap().get(account).cloned())
    }
    fn delete(&self, account: &str) -> Result<(), SecretError> {
        self.0.lock().unwrap().remove(account);
        Ok(())
    }
}

/// The shell's course folder and WebView store. The WebView has no headless
/// mode, so this port is the one fake: it records the sign-out request.
struct Device {
    root: PathBuf,
    signed_out: AtomicUsize,
}

impl ResetDevice for Device {
    fn course_files_root(&self) -> Option<PathBuf> {
        Some(self.root.clone())
    }
    fn can_sign_out_school_web(&self) -> bool {
        true
    }
    fn sign_out_school_web(&self) -> BoxFuture<'_, Result<(), String>> {
        self.signed_out.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }
}

struct Fixture {
    core: Core,
    auth: Arc<SchoolAuthService>,
    vault: MemoryVault,
}

fn fixture(name: &str) -> Fixture {
    let path = std::env::temp_dir().join(format!("isct-reset-{name}-{}", uuid::Uuid::new_v4()));
    let store = DataStore::initialize_dir(path).unwrap().0;
    let vault = MemoryVault::default();
    let auth = Arc::new(SchoolAuthService::new(vault.clone()));
    let core = Core::new(store, auth.clone());
    Fixture { core, auth, vault }
}

const KEYS: [(Dataset, &str); 4] = [
    (Dataset::LmsCourse, "7"),
    (Dataset::KyomuPage, "grades"),
    (Dataset::CatalogOfficial, "2026/LAS"),
    (Dataset::Reference, "calendar/2026"),
];

const SCHOOL_TABLES: [&str; 9] = [
    "lms_courses",
    "assignments",
    "registrations",
    "grades",
    "offerings",
    "offerings_fts",
    "academic_calendar",
    "dataset_state",
    "sync_runs",
];

/// One row in every kind of school cache, each key committed, one run
/// record, the cache settings rows and an LMS token; plus the person's plan,
/// assignment mark, course override, subscription and saved login.
fn seed(fixture: &Fixture) {
    let store = &fixture.core.store;
    let connection = store.test_connection();
    let row = "VALUES(?1,?2,'hash','2026-10-01T00:00:00+00:00','school',1)";
    for (table, id, value) in [
        (
            "lms_courses",
            "lms-course:7",
            json!({"id":"lms-course:7","lmsId":7}),
        ),
        (
            "assignments",
            "lms-assignment:42",
            json!({"id":"lms-assignment:42"}),
        ),
        (
            "registrations",
            "kyomu-registration:1",
            json!({"id":"kyomu-registration:1"}),
        ),
        ("grades", "kyomu-grade:1", json!({"id":"kyomu-grade:1"})),
        (
            "offerings",
            "2026-202601234",
            json!({"id":"2026-202601234"}),
        ),
        (
            "academic_calendar",
            "calendar:2026-10-13",
            json!({"id":"calendar:2026-10-13"}),
        ),
    ] {
        connection
            .execute(
                &format!("INSERT INTO {table}(id,value,hash,updated_at,updated_by,rev) {row}"),
                params![id, value.to_string()],
            )
            .unwrap();
    }
    connection
        .execute_batch(
            "INSERT INTO offerings_fts(offering_id,code,title_ja,title_en,instructors_ja,instructors_en)
               VALUES('2026-202601234','MTH.A201','線形代数学第一','Linear Algebra I','','');
             INSERT INTO settings(key,value,hash,updated_at,updated_by,rev)
               VALUES('kyomuSearchOptions:2026','[]','hash','2026-10-01T00:00:00Z','catalog',1);
             INSERT INTO settings(key,value,hash,updated_at,updated_by,rev,source)
               VALUES('kyomuAcademicIdentity','{}','hash','2026-10-01T00:00:00Z','kyomu',1,'kyomu');
             INSERT INTO sync_runs(run,dataset,key,started_at,finished_at,outcome)
               VALUES('run-1','lms.course','7','2026-10-01T00:00:00Z','2026-10-01T00:00:01Z','complete');",
        )
        .unwrap();
    for (dataset, key) in KEYS {
        sync_state::commit(
            &connection,
            &Key::new(dataset, key),
            Commit {
                account: "science:student",
                validator: Some("etag"),
                outcome: Outcome::Complete,
                message: None,
            },
            chrono::Utc::now(),
        )
        .unwrap();
    }
    store
        .put_user_record(
            "personal_events",
            json!({"title":"Study group","kind":"event","termId":"2026-3Q","room":"","days":[1],"startTime":"10:00","endTime":"11:00"}),
        )
        .unwrap();
    store
        .put_user_record(
            "assignment_marks",
            json!({"sourceKey":"lms:default:assignment:42","hidden":true}),
        )
        .unwrap();
    store
        .put_user_record(
            "course_overrides",
            json!({"sourceKey":"lms:default:course:7","title":"Linear algebra (mine)"}),
        )
        .unwrap();
    store
        .put_user_record(
            "subscriptions",
            json!({"id":"subscription:public:calendar","kind":"calendar","enabled":false,
                   "url":"https://example.test/index.json","fetchedAt":"2026-10-01T00:00:00Z",
                   "dataUrl":"https://example.test/calendar-2026.json","year":2026}),
        )
        .unwrap();
    fixture
        .auth
        .stage(
            SchoolAuthProfile::TokyoTech,
            SchoolCredentials {
                username: "student".into(),
                password: "password".into(),
                otp_secret: String::new(),
                matrix: "A".repeat(70),
            },
        )
        .unwrap();
    fixture
        .auth
        .store_moodle_token(
            SchoolAuthProfile::Science,
            &MoodleToken::new("token".into()),
        )
        .unwrap();
}

fn count(store: &DataStore, table: &str) -> i64 {
    store
        .test_connection()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn setting(store: &DataStore, key: &str) -> Option<String> {
    store
        .test_connection()
        .query_row("SELECT value FROM settings WHERE key=?1", [key], |row| {
            row.get(0)
        })
        .ok()
}

/// A commit through the planner's commit gate, the way an executor commits a
/// leased key: `true` when the gate let it write.
fn commit_leased_at(core: &Core, generation: u64, key: &Key) -> bool {
    core.sync
        .writer_for_test(generation)
        .write(None, |transaction| {
            sync_state::commit(
                transaction,
                key,
                Commit {
                    account: "science:student",
                    validator: None,
                    outcome: Outcome::Complete,
                    message: None,
                },
                chrono::Utc::now(),
            )
        })
        .is_ok()
}

/// A school-cache reset removes every school row, run record, cache setting
/// and the LMS token, so every key reads as missing; the person's plan, mark,
/// override, subscription choice, settings and saved login stay. The report
/// matches the preview, and a commit leased before the reset cannot write.
#[tokio::test]
async fn school_cache_reset_keeps_user_data_and_leaves_every_key_missing() {
    let fixture = fixture("school-caches");
    seed(&fixture);
    let core = &fixture.core;
    let store = &core.store;
    let preferences = setting(store, "userPreferences");
    let scope = ResetScope::SchoolCaches {
        sign_out_web: false,
        delete_downloads: false,
    };
    let preview = core.reset_preview(scope, None).await.unwrap();
    let generation = core.sync.generation();

    let report = core.reset(scope).await.unwrap();

    assert_eq!(report.removed_rows, preview.removed_rows);
    // 6 record rows, FTS, 4 keys, 1 run, 2 settings.
    assert_eq!(report.removed_rows, 14);
    for table in SCHOOL_TABLES {
        assert_eq!(count(store, table), 0, "{table}");
    }
    assert_eq!(setting(store, "kyomuSearchOptions:2026"), None);
    assert_eq!(setting(store, "kyomuAcademicIdentity"), None);
    for (dataset, key) in KEYS {
        let connection = store.test_connection();
        let state = sync_state::get(&connection, &Key::new(dataset, key)).unwrap();
        let ttl = sync_state::ttl(dataset, sync_state::Term::Current);
        assert_eq!(
            sync_state::due(state.as_ref(), ttl, "science:student", chrono::Utc::now()),
            Due::Missing,
            "{dataset:?}/{key}"
        );
    }
    assert_eq!(
        fixture
            .auth
            .load_moodle_token(SchoolAuthProfile::Science)
            .unwrap(),
        None
    );

    for table in [
        "personal_events",
        "assignment_marks",
        "course_overrides",
        "subscriptions",
    ] {
        assert_eq!(store.user_rows(table).unwrap().len(), 1, "{table}");
    }
    let subscription: Value = store.user_rows("subscriptions").unwrap().remove(0);
    assert_eq!(subscription["enabled"], false);
    assert_eq!(subscription["url"], "https://example.test/index.json");
    for field in ["fetchedAt", "dataUrl", "year"] {
        assert!(subscription.get(field).is_none(), "{field}");
    }
    assert_eq!(setting(store, "userPreferences"), preferences);
    assert!(fixture
        .auth
        .load(SchoolAuthProfile::TokyoTech)
        .unwrap()
        .is_some());

    assert_eq!(core.sync.generation(), generation + 1);
    let key = Key::new(Dataset::LmsCourse, "7");
    assert!(!commit_leased_at(core, generation, &key));
    assert_eq!(count(store, "dataset_state"), 0);
    assert!(commit_leased_at(core, core.sync.generation(), &key));
    let _ = fs::remove_dir_all(store.directory());
}

/// The two options remove what the panel promises and nothing else: the
/// WebView store is signed out, downloaded files go and their bytes are
/// counted, the notes folder is untouched. A shell that lends no device is
/// refused before anything closes.
#[tokio::test]
async fn school_cache_options_remove_downloads_and_web_session_but_never_notes() {
    let fixture = fixture("options");
    seed(&fixture);
    let core = &fixture.core;
    let scope = ResetScope::SchoolCaches {
        sign_out_web: true,
        delete_downloads: true,
    };
    let generation = core.sync.generation();
    assert_eq!(
        core.reset(scope).await,
        Err(ResetError::Unsupported("School web sign-in"))
    );
    assert_eq!(core.sync.generation(), generation);
    assert_eq!(count(&core.store, "lms_courses"), 1);

    let root = std::env::temp_dir().join(format!("isct-reset-files-{}", uuid::Uuid::new_v4()));
    let course = root.join("material").join("2026-3Q").join("MTH.A201");
    fs::create_dir_all(course.join("materials")).unwrap();
    fs::create_dir_all(course.join("notes")).unwrap();
    let material = course.join("materials").join("slides-0123abcd.pdf");
    fs::write(&material, vec![0u8; 4096]).unwrap();
    let note = course.join("notes").join("week-1-0123abcd.md");
    fs::write(&note, "my notes").unwrap();
    let device = Arc::new(Device {
        root: root.clone(),
        signed_out: AtomicUsize::new(0),
    });

    let preview = core
        .reset_preview(scope, Some(device.clone() as Arc<dyn ResetDevice>))
        .await
        .unwrap();
    let report = core
        .reset_with(scope, Some(device.clone() as Arc<dyn ResetDevice>))
        .await
        .unwrap();

    assert!(!material.exists());
    assert_eq!(fs::read_to_string(&note).unwrap(), "my notes");
    assert_eq!(device.signed_out.load(Ordering::SeqCst), 1);
    assert!(report.freed_bytes >= 4096);
    assert!(preview.freed_bytes >= 4096);
    assert_eq!(count(&core.store, "lms_courses"), 0);
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(core.store.directory());
}

/// A second request for the same reset joins the running one: one clear,
/// one generation step, the same report. A different reset meanwhile is
/// refused rather than silently answered with the first one's result.
#[tokio::test]
async fn concurrent_resets_join_the_running_one() {
    let fixture = fixture("join");
    seed(&fixture);
    let core = &fixture.core;
    let scope = ResetScope::SchoolCaches {
        sign_out_web: false,
        delete_downloads: false,
    };
    let generation = core.sync.generation();

    let (first, second, other) = tokio::join!(
        core.reset(scope),
        core.reset(scope),
        core.reset(ResetScope::Everything)
    );

    assert_eq!(first.as_ref().unwrap().removed_rows, 14);
    assert_eq!(first, second);
    assert_eq!(other, Err(ResetError::Busy));
    assert_eq!(core.sync.generation(), generation + 1);
    assert_eq!(store_events(core), 1);
    // The lease is released: the next reset runs on its own.
    assert_eq!(core.reset(scope).await.unwrap().removed_rows, 0);
    assert_eq!(core.sync.generation(), generation + 2);

    // A caller may leave before polling the shared result. The owned task
    // still finishes, and a different reset must then be admitted.
    drop(core.reset_flight(scope, None).unwrap());
    tokio::time::timeout(Duration::from_secs(5), async {
        while core.sync.generation() == generation + 2 || core.sync.snapshot().resetting {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    core.reset(ResetScope::Everything).await.unwrap();
    assert_eq!(core.sync.generation(), generation + 4);
    assert_eq!(store_events(core), 0);
    let _ = fs::remove_dir_all(core.store.directory());
}

fn store_events(core: &Core) -> usize {
    core.store.user_rows("personal_events").unwrap().len()
}

/// A full reset is the first-launch state: user data, settings, device
/// identity, saved logins and the token go, defaults come back, and no
/// tombstone is left for cloud sync to spread.
#[tokio::test]
async fn full_reset_returns_to_first_launch() {
    let fixture = fixture("everything");
    seed(&fixture);
    let core = &fixture.core;
    let device_id = setting(&core.store, "deviceId");

    core.reset(ResetScope::Everything).await.unwrap();

    for table in SCHOOL_TABLES.into_iter().chain([
        "personal_events",
        "assignment_marks",
        "course_overrides",
        "subscriptions",
    ]) {
        assert_eq!(count(&core.store, table), 0, "{table}");
    }
    assert!(setting(&core.store, "deviceId").is_some());
    assert_ne!(setting(&core.store, "deviceId"), device_id);
    assert!(setting(&core.store, "userPreferences").is_some());
    assert_eq!(setting(&core.store, "kyomuSearchOptions:2026"), None);
    assert!(fixture
        .auth
        .load(SchoolAuthProfile::TokyoTech)
        .unwrap()
        .is_none());
    assert!(!fixture
        .vault
        .0
        .lock()
        .unwrap()
        .contains_key("lms-token-science"));
    let _ = fs::remove_dir_all(core.store.directory());
}
