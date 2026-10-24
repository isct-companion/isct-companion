//! Local profiling against a disposable copy of the data folder; never prints workspace contents.
use isct_core::{
    data::DataStore,
    workspace::{WorkspaceIntent, WorkspaceMutation, WorkspaceService},
};
use std::time::Instant;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--disposable-db") {
        return Err(
            "Use --disposable-db <copy of the data folder>; this probe changes quarter selection"
                .into(),
        );
    }
    let path = args
        .next()
        .ok_or("Supply a disposable copy of the data folder")?;
    let store = DataStore::initialize_dir(path.into())?.0;
    let service = WorkspaceService::new(store);
    // The phone timetable's read: the week shown with four weeks on either
    // side, from 3Q's toolbar context, here around 2Q's middle week.
    for _ in 0..2 {
        let start = Instant::now();
        let snapshot = service.snapshot()?;
        let read_ms = start.elapsed().as_millis();
        let start = Instant::now();
        let view = isct_core::schedule::query(
            &snapshot.workspace,
            isct_core::schedule::WorkspaceQuery::ScheduleView {
                term: "2026-3Q".into(),
                from: "2026-05-18".into(),
                to: "2026-07-17".into(),
                include_personal: false,
            },
        )?;
        let query_ms = start.elapsed().as_millis();
        let start = Instant::now();
        let encoded = serde_json::to_vec(&view)?;
        let encode_ms = start.elapsed().as_millis();
        // A digest, never the contents, so two builds can be compared.
        let mut digest = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&encoded, &mut digest);
        println!(
            "schedule view: snapshot_ms={read_ms} query_ms={query_ms} encode_ms={encode_ms} bytes={} digest={:x}",
            encoded.len(),
            std::hash::Hasher::finish(&digest)
        );
    }
    for term in ["2026-1Q", "2026-2Q", "2026-3Q", "2026-4Q"] {
        let start = Instant::now();
        let snapshot = service.snapshot()?;
        let read_ms = start.elapsed().as_millis();
        let start = Instant::now();
        let next = service.mutate(WorkspaceMutation {
            expected_revision: snapshot.revision,
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
    Ok(())
}
