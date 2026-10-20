//! The one read model of sync progress: runs with real counts, and one line
//! per source.
//!
//! Labels are i18n keys: a run is `sync.run.<dataset>` (dots of the dataset
//! name as dashes, e.g. `sync.run.lms-course`) with `term` (`current`,
//! `past` or null) and `scope`; the key being read is
//! `sync.current.<dataset>` with `title` and `key`.
use super::{
    planner::{Candidate, Facts, Inputs},
    runner::Batch,
    Held, Label, RunView, SourceView, SyncFailure, SyncSnapshot,
};
use crate::{
    school_auth::SchoolAuthProfile,
    sync_state::{Dataset, Key, KeyState},
};
use chrono::{DateTime, Utc};
use serde_json::json;
use std::collections::{BTreeMap, HashMap, HashSet};

pub(super) struct Parts<'a> {
    pub(super) generation: u64,
    pub(super) resetting: bool,
    pub(super) batches: &'a [Batch],
    pub(super) leased: &'a HashSet<Key>,
    pub(super) plan: &'a [Candidate],
    pub(super) facts: &'a Facts,
    pub(super) inputs: &'a Inputs,
    pub(super) sessions: &'a HashMap<SchoolAuthProfile, String>,
    pub(super) kyomu_notice: Option<&'a str>,
    pub(super) now: DateTime<Utc>,
}

fn label_name(dataset: Dataset) -> String {
    dataset.name().replace('.', "-")
}

/// The most common error kind, the count and the earliest retry.
fn failure<'a>(
    failed: impl Iterator<Item = (&'a str, Option<DateTime<Utc>>)>,
) -> Option<SyncFailure> {
    let mut kinds: BTreeMap<&str, u32> = BTreeMap::new();
    let mut retry_at: Option<DateTime<Utc>> = None;
    let mut count = 0u32;
    for (kind, retry) in failed {
        count = count.saturating_add(1);
        *kinds.entry(kind).or_default() += 1;
        if let Some(retry) = retry {
            retry_at = Some(retry_at.map_or(retry, |current| current.min(retry)));
        }
    }
    let kind = kinds
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(kind, _)| kind.to_string())?;
    Some(SyncFailure {
        kind,
        keys_failed: count,
        retry_at,
    })
}

fn count(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn run_view(batch: &Batch) -> RunView {
    let dataset = batch.group.dataset;
    let total = batch.pending.len() + batch.running.len() + batch.done.len() + batch.failed.len();
    RunView {
        run: batch.run.clone(),
        dataset: dataset.name().into(),
        label: Label {
            key: format!("sync.run.{}", label_name(dataset)),
            args: json!({ "term": batch.group.term, "scope": batch.group.scope }),
        },
        priority: batch.priority,
        done: count(batch.done.len()),
        total: Some(count(total)),
        current: batch.running.iter().next().map(|(key, title)| Label {
            key: format!("sync.current.{}", label_name(dataset)),
            args: json!({ "title": title, "key": key.key }),
        }),
        scope: batch.group.scope.clone(),
        held: batch.held.clone(),
        failure: failure(
            batch
                .failed
                .values()
                .map(|(kind, retry)| (kind.as_str(), *retry)),
        ),
    }
}

struct Source {
    name: &'static str,
    datasets: &'static [Dataset],
    /// The datasets whose newest read is the source's "last updated".
    updated_by: &'static [Dataset],
    profile: Option<SchoolAuthProfile>,
}

const SOURCES: [Source; 4] = [
    Source {
        name: "lms",
        datasets: &[
            Dataset::LmsEnrolment,
            Dataset::LmsDeadlines,
            Dataset::LmsCourse,
            Dataset::LmsAssign,
            Dataset::LmsHistory,
        ],
        updated_by: &[Dataset::LmsEnrolment, Dataset::LmsDeadlines],
        profile: Some(SchoolAuthProfile::Science),
    },
    Source {
        name: "kyomu",
        datasets: &[Dataset::KyomuPage],
        updated_by: &[Dataset::KyomuPage],
        profile: Some(SchoolAuthProfile::TokyoTech),
    },
    Source {
        name: "references",
        datasets: &[Dataset::Reference],
        updated_by: &[Dataset::Reference],
        profile: None,
    },
    Source {
        name: "catalog",
        datasets: &[Dataset::CatalogOfficial, Dataset::CatalogDetail],
        updated_by: &[Dataset::CatalogOfficial, Dataset::CatalogDetail],
        profile: None,
    },
];

fn configured(inputs: &Inputs, profile: SchoolAuthProfile) -> bool {
    match profile {
        SchoolAuthProfile::Science => inputs.science_configured,
        SchoolAuthProfile::TokyoTech => inputs.tokyo_configured,
    }
}

fn source_view(source: &Source, parts: &Parts<'_>) -> SourceView {
    let states = || {
        parts
            .inputs
            .states
            .iter()
            .filter(|(key, _)| source.datasets.contains(&key.dataset))
            .map(|(key, state)| (key, state))
    };
    let last_updated = states()
        .filter(|(key, _)| source.updated_by.contains(&key.dataset))
        .filter_map(|(_, state)| state.fetched_at)
        .max();
    let running = parts
        .leased
        .iter()
        .any(|key| source.datasets.contains(&key.dataset));
    let mut held = parts
        .plan
        .iter()
        .filter(|candidate| source.datasets.contains(&candidate.key.dataset))
        .find_map(|candidate| candidate.held.clone());
    if let Some(profile) = source.profile {
        if held.is_none()
            && !running
            && !parts.facts.signed_in.contains(&profile)
            && !configured(parts.inputs, profile)
        {
            held = Some(Held::SignInRequired { profile });
        }
    }
    let backing_off = |state: &&KeyState| {
        state.failures > 0 && state.next_eligible.is_some_and(|at| at > parts.now)
    };
    let failure = failure(
        states()
            .map(|(_, state)| state)
            .filter(backing_off)
            .map(|state| {
                (
                    state.last_error.as_deref().unwrap_or("failed"),
                    state.next_eligible,
                )
            }),
    );
    SourceView {
        source: source.name.into(),
        last_updated,
        running,
        held,
        failure,
        session: source
            .profile
            .and_then(|profile| parts.sessions.get(&profile).cloned()),
        maintenance: (source.name == "kyomu")
            .then(|| parts.kyomu_notice.map(str::to_owned))
            .flatten(),
    }
}

pub(super) fn build(parts: Parts<'_>) -> SyncSnapshot {
    SyncSnapshot {
        generation: parts.generation,
        resetting: parts.resetting,
        runs: parts.batches.iter().map(run_view).collect(),
        sources: SOURCES
            .iter()
            .map(|source| source_view(source, &parts))
            .collect(),
    }
}
