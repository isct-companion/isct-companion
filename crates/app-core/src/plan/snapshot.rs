//! The one read model of sync progress: runs with real counts, and one line
//! per source.
//!
//! Labels are i18n keys: a run is `sync.run.<dataset>` (dots of the dataset
//! name as dashes, e.g. `sync.run.lms-course`) with `term` (`current`,
//! `past` or null) and `scope`; the key being read is
//! `sync.current.<dataset>` with `title` and `key`.
use super::{
    planner::{Candidate, Inputs},
    runner::{Batch, FailedKey},
    AccountWait, Held, Label, Priority, RetryPlan, RunView, Scope, SourceView, SyncAttention,
    SyncAttentionProblem, SyncFailure, SyncSnapshot, SyncWork,
};
use crate::{
    lifecycle::SchoolSite,
    school_auth::SchoolAuthProfile,
    school_session::{SchoolSessionState, SignInPresentation},
    sync_state::{Dataset, Key, KeyState},
};
use chrono::{DateTime, Utc};
use serde_json::json;
use std::collections::{BTreeMap, HashMap, HashSet};

/// One account as its session snapshot last said, for the status line only:
/// the planner never reads it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Shown {
    pub(super) state: SchoolSessionState,
    pub(super) presentation: SignInPresentation,
    pub(super) retry_at: Option<DateTime<Utc>>,
    pub(super) maintenance: Option<String>,
}

pub(super) struct Parts<'a> {
    pub(super) generation: u64,
    pub(super) resetting: bool,
    pub(super) batches: &'a [Batch],
    pub(super) leased: &'a HashSet<Key>,
    pub(super) plan: &'a [Candidate],
    pub(super) sessions: &'a HashMap<SchoolAuthProfile, Shown>,
    pub(super) inputs: &'a Inputs,
    pub(super) now: DateTime<Utc>,
}

pub(super) fn label_name(dataset: Dataset) -> String {
    dataset.name().replace('.', "-")
}

fn failure_reason(kind: &str) -> Label {
    let known = match kind {
        "timeout" => "timedOut",
        "offline" => "noConnection",
        "sign-in" => "signInRequired",
        "site-changed" => "siteChanged",
        "storage" => "notSaved",
        "parse" => "unreadable",
        "dns" | "tls" | "connect" => "unreachable",
        "stalled" => "interrupted",
        "too-large" => "tooLarge",
        "host-paused" => "paused",
        "cancelled" => "cancelled",
        _ => "",
    };
    let (key, args) = if !known.is_empty() {
        (known, json!({}))
    } else if let Some(status) = kind
        .strip_prefix("http-")
        .filter(|status| status.len() == 3 && status.bytes().all(|byte| byte.is_ascii_digit()))
    {
        let key = match status.as_bytes().first() {
            Some(b'5') => "serverFailed",
            Some(b'3') => "serverRedirected",
            _ => "serverRefused",
        };
        (key, json!({ "status": status }))
    } else {
        // The code stays in diagnostics; the person sees a plain sentence.
        ("unknownCode", json!({}))
    };
    Label {
        key: format!("syncStatus.failure.{key}"),
        args,
    }
}

/// The 教務Web pages Core reads, by `Key::key`; each has a name in the
/// catalogs, so a page in progress never shows its identifier.
pub(super) const KYOMU_PAGES: [&str; 6] = [
    "profile",
    "registrations",
    "declared",
    "grades",
    "requirements",
    "gpa",
];

/// What a run is reading now. The catalogs print `title`; a 教務Web page has
/// none and is named by its page key, and any other key without a title is
/// left out rather than shown as an identifier.
fn current_label(dataset: Dataset, key: &str, title: Option<&str>) -> Option<Label> {
    match title {
        Some(title) => Some(Label {
            key: format!("sync.current.{}", label_name(dataset)),
            args: json!({ "title": title, "key": key }),
        }),
        None if dataset == Dataset::KyomuPage && KYOMU_PAGES.contains(&key) => Some(Label {
            key: format!("syncStatus.kyomuPage.{key}"),
            args: json!({}),
        }),
        None => None,
    }
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
        reason: failure_reason(&kind),
        kind,
        keys_failed: count,
        retry_at,
        detail: None,
    })
}

/// `grades: <reason>`, one line a failed page. Only 教務Web: its keys name
/// pages a person can recognize and quote, the others are ids.
fn failed_pages(dataset: Dataset, failed: &BTreeMap<Key, FailedKey>) -> Option<String> {
    if dataset != Dataset::KyomuPage || failed.is_empty() {
        return None;
    }
    let lines: Vec<String> = failed
        .iter()
        .map(|(key, failed)| {
            let reason = failed.reason.as_deref().unwrap_or(&failed.kind);
            format!("{}: {reason}", key.key)
        })
        .collect();
    Some(lines.join("\n"))
}

fn count(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn year(value: &str) -> Option<u16> {
    value
        .parse::<u16>()
        .ok()
        .filter(|year| (1000..=9999).contains(year) && year.to_string() == value)
}

fn course_id(value: &str) -> Option<u64> {
    value
        .parse::<u64>()
        .ok()
        .filter(|id| *id > 0 && id.to_string() == value)
}

fn retry_scope(key: &Key, run_scope: Option<&str>) -> Option<Scope> {
    match key.dataset {
        Dataset::LmsEnrolment | Dataset::LmsDeadlines
            if run_scope.is_none() && key.key == "all" =>
        {
            Some(Scope::Site {
                site: SchoolSite::ScienceLms,
            })
        }
        Dataset::LmsCourse | Dataset::LmsAssign => {
            let id = course_id(&key.key)?;
            if run_scope.is_some_and(|scope| course_id(scope) != Some(id)) {
                return None;
            }
            Some(Scope::Course { lms_course_id: id })
        }
        Dataset::LmsHistory if run_scope.is_none() => {
            let (raw_year, raw_month) = key.key.split_once('-')?;
            let year = year(raw_year)?;
            let month = raw_month
                .parse::<u8>()
                .ok()
                .filter(|month| (1..=12).contains(month) && format!("{month:02}") == raw_month)?;
            Some(Scope::LmsHistory { year, month })
        }
        Dataset::KyomuPage
            if run_scope.is_none()
                && adpt_isct::kyomu::PAGES
                    .iter()
                    .any(|page| page.kind == key.key) =>
        {
            Some(Scope::Site {
                site: SchoolSite::Kyomu,
            })
        }
        Dataset::Reference
            if run_scope.is_none() && crate::reference_refresh::parse_key(&key.key).is_some() =>
        {
            Some(Scope::References)
        }
        Dataset::CatalogOfficial => {
            let (raw_year, prefix) = key.key.split_once('/')?;
            let target_year = year(raw_year)?;
            let prefix = prefix.strip_suffix("/en").unwrap_or(prefix);
            if !super::planner::department_prefixes()
                .iter()
                .any(|known| known == prefix)
                || run_scope.is_some_and(|scope| year(scope) != Some(target_year))
            {
                return None;
            }
            Some(Scope::Catalog { year: target_year })
        }
        _ => None,
    }
}

fn retry_plan(scopes: impl IntoIterator<Item = Scope>) -> Option<RetryPlan> {
    let mut unique = Vec::new();
    for scope in scopes {
        if !unique.contains(&scope) {
            unique.push(scope);
        }
    }
    (!unique.is_empty()).then_some(RetryPlan { scopes: unique })
}

pub(super) fn run_view(batch: &Batch) -> RunView {
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
        current: batch
            .running
            .iter()
            .next()
            .and_then(|(key, title)| current_label(dataset, &key.key, title.as_deref())),
        scope: batch.group.scope.clone(),
        retry: batch
            .failed
            .keys()
            .map(|key| {
                (key.dataset == dataset)
                    .then(|| retry_scope(key, batch.group.scope.as_deref()))
                    .flatten()
            })
            .collect::<Option<Vec<_>>>()
            .and_then(retry_plan),
        held: batch.held.clone(),
        failure: failure(
            batch
                .failed
                .values()
                .map(|failed| (failed.kind.as_str(), failed.retry)),
        )
        .map(|failure| SyncFailure {
            detail: failed_pages(dataset, &batch.failed),
            ..failure
        }),
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
        datasets: &[Dataset::CatalogOfficial],
        updated_by: &[Dataset::CatalogOfficial],
        profile: None,
    },
];

fn source_view(source: &Source, parts: &Parts<'_>) -> SourceView {
    let states = || {
        parts
            .inputs
            .states
            .iter()
            .filter(|(key, _)| source.datasets.contains(&key.dataset))
    };
    let last_updated = states()
        .filter(|(key, _)| source.updated_by.contains(&key.dataset))
        .filter_map(|(_, state)| state.fetched_at)
        .max();
    let running = parts
        .leased
        .iter()
        .any(|key| source.datasets.contains(&key.dataset));
    // As a run's summary: a source with a key running or ready to run is
    // not waiting, whatever its other keys wait for. A key waiting for
    // another of the same source says nothing about the source; its run
    // shows the wait.
    let planned = || {
        parts
            .plan
            .iter()
            .filter(|candidate| source.datasets.contains(&candidate.key.dataset))
    };
    let ready = planned().any(|candidate| candidate.held.is_none());
    let held = if running || ready {
        None
    } else {
        planned().find_map(|candidate| {
            candidate
                .held
                .clone()
                .filter(|held| *held != Held::Prerequisite)
        })
    };
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
    let mut failed_keys: Vec<_> = states()
        .filter(|(_, state)| backing_off(state))
        .map(|(key, _)| key)
        .collect();
    failed_keys.sort_unstable();
    SourceView {
        source: source.name.into(),
        last_updated,
        running,
        held,
        failure,
        retry: retry_plan(
            failed_keys
                .into_iter()
                .filter_map(|key| retry_scope(key, None)),
        ),
        session: source
            .profile
            .and_then(|profile| parts.sessions.get(&profile))
            .map(|shown| shown.state),
        maintenance: source
            .profile
            .and_then(|profile| parts.sessions.get(&profile))
            .and_then(|shown| shown.maintenance.clone()),
    }
}

fn account_wait(held: Option<&Held>) -> Option<SchoolAuthProfile> {
    match held {
        Some(Held::Account {
            profile,
            wait: AccountWait::Person,
        }) => Some(*profile),
        _ => None,
    }
}

fn attention(parts: &Parts<'_>, runs: &[RunView], sources: &[SourceView]) -> SyncAttention {
    if parts.resetting {
        return SyncAttention {
            primary: Some(SyncWork::Resetting),
            problem: None,
            busy: true,
        };
    }
    let mut ranked: Vec<_> = runs.iter().collect();
    ranked.sort_by_key(|run| std::cmp::Reverse(run.priority));
    let active = ranked
        .iter()
        .copied()
        .find(|run| run.held.is_none() && run.failure.is_none());
    let foreground_held = ranked
        .iter()
        .copied()
        .filter(|run| run.priority != Priority::Background);
    let foreground_account = foreground_held
        .clone()
        .find_map(|run| account_wait(run.held.as_ref()));
    let account = foreground_account
        .or_else(|| {
            ranked
                .iter()
                .find_map(|run| account_wait(run.held.as_ref()))
        })
        .or_else(|| {
            sources
                .iter()
                .find_map(|source| account_wait(source.held.as_ref()))
        });
    let problem = account
        .map(|profile| {
            let session = parts.sessions.get(&profile);
            SyncAttentionProblem::Account {
                profile,
                presentation: session.map(|session| session.presentation.clone()),
                retry_at: session.and_then(|session| session.retry_at),
            }
        })
        .or_else(|| {
            ranked.iter().find_map(|run| {
                Some(SyncAttentionProblem::RunFailed {
                    dataset: run.dataset.clone(),
                    failure: run.failure.clone()?,
                    retry: run.retry.clone(),
                })
            })
        })
        .or_else(|| {
            sources.iter().find_map(|source| {
                Some(SyncAttentionProblem::SourceFailed {
                    source: source.source.clone(),
                    failure: source.failure.clone()?,
                    retry: source.retry.clone(),
                })
            })
        });
    let busy = active.is_some_and(|run| run.priority != Priority::Background);
    let active_work = |run: &RunView| SyncWork::Active {
        run: run.run.clone(),
    };
    let held_work = |run: &RunView| SyncWork::Held {
        run: run.run.clone(),
    };
    let primary = active
        .filter(|_| busy)
        .map(active_work)
        .or_else(|| foreground_account.map(|_| SyncWork::Problem))
        .or_else(|| {
            foreground_held
                .clone()
                .find(|run| run.held.is_some())
                .map(held_work)
        })
        .or_else(|| active.map(active_work))
        .or_else(|| problem.as_ref().map(|_| SyncWork::Problem))
        .or_else(|| {
            ranked
                .iter()
                .copied()
                .find(|run| run.held.is_some())
                .map(held_work)
        });
    SyncAttention {
        primary,
        problem,
        busy,
    }
}

/// Announcement reads have per-key diagnostics and retry state, but their
/// progress does not belong in the school sync snapshot.
pub(super) fn shows_run(dataset: Dataset) -> bool {
    dataset != Dataset::Announcements
}

pub(super) fn build(parts: Parts<'_>) -> SyncSnapshot {
    let runs: Vec<_> = parts
        .batches
        .iter()
        .filter(|batch| shows_run(batch.group.dataset))
        .map(run_view)
        .collect();
    let sources: Vec<_> = SOURCES
        .iter()
        .map(|source| source_view(source, &parts))
        .collect();
    let attention = attention(&parts, &runs, &sources);
    SyncSnapshot {
        generation: parts.generation,
        resetting: parts.resetting,
        runs,
        sources,
        attention,
    }
}
