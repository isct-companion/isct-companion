//! Local profiling against a disposable copy of the data folder; never prints workspace contents.
use isct_core::{
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
    let store = DataStore::initialize_dir(path.clone().into())?.0;
    let service = WorkspaceService::new(store);
    let mut session_key = None;
    for _ in 0..3 {
        let start = Instant::now();
        let workspace = service.snapshot()?.workspace;
        let read_ms = start.elapsed().as_millis();
        let start = Instant::now();
        let view = isct_core::schedule::query(&workspace, schedule_view())?;
        let query_ms = start.elapsed().as_millis();
        let start = Instant::now();
        let encoded = serde_json::to_vec(&view)?;
        let encode_ms = start.elapsed().as_millis();
        let whole = service.snapshot()?;
        let start = Instant::now();
        let payload = serde_json::to_vec(&whole)?;
        let payload_ms = start.elapsed().as_millis();
        let payload = payload.len();
        // Without the school records the workspace also carries for queries.
        let mut workspace = serde_json::Value::clone(&whole.workspace);
        for field in ["registrations", "grades", "lmsCourses"] {
            workspace.as_object_mut().map(|object| object.remove(field));
        }
        let workspace_digest = digest(&serde_json::to_vec(&(&workspace, &whole.school_records))?);
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
            "schedule view: snapshot_ms={read_ms} query_ms={query_ms} encode_ms={encode_ms} bytes={} snapshot_bytes={payload} snapshot_encode_ms={payload_ms} digest={:x} workspace_digest={workspace_digest:x}",
            encoded.len(),
            digest(&encoded),
        );
    }
    for term in ["2026-1Q", "2026-2Q", "2026-3Q", "2026-4Q"] {
        let start = Instant::now();
        // Only the revision: a shell keeps no snapshot alive while it writes.
        let revision = service.snapshot()?.revision;
        let read_ms = start.elapsed().as_millis();
        let start = Instant::now();
        let next = service.mutate(WorkspaceMutation {
            expected_revision: revision,
            intent: WorkspaceIntent::SelectTerm {
                term_id: term.into(),
            },
        })?;
        let mutation_ms = start.elapsed().as_millis();
        let start = Instant::now();
        let _ = isct_core::schedule::planning_week(&next.workspace, term)?;
        println!(
            "{term}: snapshot_ms={read_ms} mutation_ms={mutation_ms} planning_ms={}",
            start.elapsed().as_millis()
        );
    }
    // The right-click mark, then the calendar's refresh after it.
    let key = session_key.ok_or("No class session in the probe's range")?;
    for mark in ["cancelled", "class", "exam", "class"] {
        let start = Instant::now();
        let revision = service.snapshot()?.revision;
        let read_ms = start.elapsed().as_millis();
        let start = Instant::now();
        // What the desktop command answers: the changes and their patch.
        let commit = service.mutate_patch(WorkspaceMutation {
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
        let workspace = service.snapshot()?.workspace;
        let view = isct_core::schedule::query(&workspace, schedule_view())?;
        let _ = serde_json::to_vec(&view)?;
        println!(
            "mark {mark}: snapshot_ms={read_ms} mutation_ms={mutation_ms} refresh_ms={} payload_bytes={} payload_encode_ms={payload_ms} changes={:?}",
            start.elapsed().as_millis(),
            payload.len(),
            commit.changes,
        );
    }
    // A sync bookkeeping commit (a `last_used` touch) changes no school
    // record, but it is a cache.db commit: the next read or write rebuilds.
    let cache = rusqlite::Connection::open(std::path::Path::new(&path).join("cache.db"))?;
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
        let revision = service.snapshot()?.revision;
        touch()?;
        let start = Instant::now();
        service.mutate(WorkspaceMutation {
            expected_revision: revision,
            intent: WorkspaceIntent::MarkSession {
                key: key.clone(),
                mark: mark.into(),
            },
        })?;
        let mutation_ms = start.elapsed().as_millis();
        touch()?;
        let start = Instant::now();
        let _ = service.snapshot()?;
        println!(
            "after a sync touch: mark {mark} mutation_ms={mutation_ms}, a read snapshot_ms={}",
            start.elapsed().as_millis()
        );
    }
    Ok(())
}
