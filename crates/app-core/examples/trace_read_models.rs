//! Local profiling against a disposable copy of the data folder; never prints workspace contents.
use app_core::{
    data::DataStore,
    schedule::WorkspaceQuery,
    workspace::{WorkspaceIntent, WorkspaceMutation, WorkspaceService},
};
use std::time::Instant;

/// The desktop calendar's read: the week shown with four weeks on either
/// side, from 3Q's toolbar context, here around 2Q's middle week.
fn schedule_view() -> WorkspaceQuery {
    WorkspaceQuery::ScheduleView {
        term: "2026-3Q".into(),
        from: "2026-05-18".into(),
        to: "2026-07-17".into(),
        include_personal: false,
    }
}

/// No secrets: the probe never signs in.
struct NoVault;
impl app_core::ports::SecretVault for NoVault {
    fn store(&self, _: &str, _: &str) -> Result<(), app_core::ports::SecretError> {
        Ok(())
    }
    fn load(&self, _: &str) -> Result<Option<String>, app_core::ports::SecretError> {
        Ok(None)
    }
    fn delete(&self, _: &str) -> Result<(), app_core::ports::SecretError> {
        Ok(())
    }
}

/// The queries the desktop pages and the phone send, around 2Q's middle week.
fn page_queries(course: &str) -> Vec<serde_json::Value> {
    let now = "2026-06-01T10:00:00+09:00";
    vec![
        serde_json::json!({"kind":"workspaceOverview","term":"2026-2Q"}),
        serde_json::json!({"kind":"planningWeek","term":"2026-2Q"}),
        serde_json::json!({"kind":"scheduleView","term":"2026-3Q","from":"2026-05-18","to":"2026-07-17","includePersonal":false}),
        serde_json::json!({"kind":"taskView","now":now}),
        serde_json::json!({"kind":"today","now":now}),
        serde_json::json!({"kind":"assignmentBoard","now":now}),
        serde_json::json!({"kind":"courseDetail","courseId":course}),
        serde_json::json!({"kind":"courseContent","courseId":course,"now":now}),
        serde_json::json!({"kind":"lectureSearch","term":"2026-2Q","query":"report"}),
        serde_json::json!({"kind":"timetableYear","year":2026}),
        serde_json::json!({"kind":"glance","now":now}),
        serde_json::json!({"kind":"notificationPlan","now":now,"horizon":"2026-06-08T10:00:00+09:00"}),
    ]
}

/// A digest, never the contents, so two builds can be compared.
fn digest(bytes: &[u8]) -> u64 {
    let mut digest = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(bytes, &mut digest);
    std::hash::Hasher::finish(&digest)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--disposable-db") {
        return Err(
            "Use --disposable-db <copy of the data folder>; this probe changes quarter selection and session marks"
                .into(),
        );
    }
    let path = args
        .next()
        .ok_or("Supply a disposable copy of the data folder")?;
    let start = Instant::now();
    // The first open of an older folder upgrades it and merges every course.
    let store = DataStore::initialize_dir(path.clone().into())?.0;
    println!("open: open_ms={}", start.elapsed().as_millis());
    let service = WorkspaceService::new(store.clone());
    // Each page's queries read only their tables; each answer must equal the
    // one every field read together gives.
    let core = app_core::Core::new(
        store.clone(),
        std::sync::Arc::new(app_core::school_auth::SchoolAuthService::with_status_cache(
            NoVault,
            std::path::Path::new(&path).join("probe-credential-status.json"),
        )),
    );
    let start = Instant::now();
    let (whole, school) = store.load_reads()?;
    println!(
        "every field: read_ms={} bytes={}",
        start.elapsed().as_millis(),
        serde_json::to_vec(&*whole)?.len()
    );
    let course = whole["courses"]
        .as_array()
        .and_then(|courses| {
            courses
                .iter()
                .find(|course| course["lmsCourseId"].is_number())
        })
        .and_then(|course| course["id"].as_str())
        .unwrap_or_default()
        .to_owned();
    for raw in page_queries(&course) {
        let kind = raw["kind"].as_str().unwrap_or_default().to_owned();
        let request: WorkspaceQuery = serde_json::from_value(raw)?;
        let old = serde_json::to_vec(&app_core::schedule::query(
            &whole,
            &school,
            request.clone(),
        )?)?;
        let start = Instant::now();
        let cold = core.schedule_query(vec![request.clone()])?;
        let cold_ms = start.elapsed().as_millis();
        let start = Instant::now();
        let warm = core.schedule_query(vec![request])?;
        let warm_ms = start.elapsed().as_millis();
        let new = serde_json::to_vec(&cold.values[0])?;
        println!(
            "view {kind}: equal={} bytes={} cold_ms={cold_ms} warm_ms={warm_ms} digest={:x}",
            old == new && new == serde_json::to_vec(&warm.values[0])?,
            new.len(),
            digest(&new),
        );
    }
    // The calendar's read as the desktop sends it, after a commit (cold) and
    // again (warm).
    let mut session_key = None;
    for _ in 0..3 {
        let start = Instant::now();
        let answer = core.schedule_query(vec![schedule_view()])?;
        let query_ms = start.elapsed().as_millis();
        let start = Instant::now();
        let encoded = serde_json::to_vec(&answer.values[0])?;
        let encode_ms = start.elapsed().as_millis();
        session_key = serde_json::from_slice::<serde_json::Value>(&encoded)?
            .pointer("/value/agendas")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .flat_map(|agenda| {
                agenda["occurrences"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            })
            .find(|occurrence| occurrence["kind"] == "course")
            .and_then(|occurrence| occurrence["key"].as_str().map(str::to_owned));
        println!(
            "schedule view: query_ms={query_ms} encode_ms={encode_ms} bytes={} digest={:x}",
            encoded.len(),
            digest(&encoded),
        );
    }
    for term in ["2026-1Q", "2026-2Q", "2026-3Q", "2026-4Q"] {
        let start = Instant::now();
        let revision = service.revision()?;
        let read_ms = start.elapsed().as_millis();
        let start = Instant::now();
        service.mutate_commit(WorkspaceMutation {
            expected_revision: revision,
            intent: WorkspaceIntent::SelectTerm {
                term_id: term.into(),
            },
        })?;
        let mutation_ms = start.elapsed().as_millis();
        let start = Instant::now();
        let _ = core.schedule_query(vec![WorkspaceQuery::PlanningWeek { term: term.into() }])?;
        println!(
            "{term}: revision_ms={read_ms} mutation_ms={mutation_ms} planning_ms={}",
            start.elapsed().as_millis()
        );
    }
    // The right-click mark, then the calendar's refresh after it.
    let key = session_key.ok_or("No class session in the probe's range")?;
    for mark in ["cancelled", "class", "exam", "class"] {
        let start = Instant::now();
        let revision = service.revision()?;
        let read_ms = start.elapsed().as_millis();
        let start = Instant::now();
        // What the desktop command answers: the changes, revision and seq.
        let commit = service.mutate_commit(WorkspaceMutation {
            expected_revision: revision,
            intent: WorkspaceIntent::MarkSession {
                key: key.clone(),
                mark: mark.into(),
            },
        })?;
        let mutation_ms = start.elapsed().as_millis();
        let start = Instant::now();
        let payload = serde_json::to_vec(&commit)?;
        let payload_ms = start.elapsed().as_millis();
        let start = Instant::now();
        let view = core.schedule_query(vec![schedule_view()])?;
        let _ = serde_json::to_vec(&view.values)?;
        println!(
            "mark {mark}: revision_ms={read_ms} mutation_ms={mutation_ms} refresh_ms={} payload_bytes={} payload_encode_ms={payload_ms} changes={:?}",
            start.elapsed().as_millis(),
            payload.len(),
            commit.changes,
        );
    }
    // A sync bookkeeping commit (a `last_used` touch) changes no school
    // record; it commits to sync.db, never to cache.db.
    let cache = rusqlite::Connection::open(std::path::Path::new(&path).join("sync.db"))?;
    let touches = std::cell::Cell::new(0);
    let touch = || {
        touches.set(touches.get() + 1);
        cache.execute(
            "UPDATE dataset_state SET last_used=datetime('now', ?1 || ' seconds')",
            [touches.get()],
        )
    };
    for mark in ["cancelled", "class", "exam", "class"] {
        // The shell already holds the revision when the click lands.
        let revision = service.revision()?;
        touch()?;
        let start = Instant::now();
        service.mutate_commit(WorkspaceMutation {
            expected_revision: revision,
            intent: WorkspaceIntent::MarkSession {
                key: key.clone(),
                mark: mark.into(),
            },
        })?;
        let mutation_ms = start.elapsed().as_millis();
        touch()?;
        let start = Instant::now();
        let _ = core.schedule_query(vec![schedule_view()])?;
        println!(
            "after a sync touch: mark {mark} mutation_ms={mutation_ms}, the calendar's read query_ms={}",
            start.elapsed().as_millis()
        );
    }
    Ok(())
}
