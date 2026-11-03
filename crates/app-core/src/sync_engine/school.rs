//! Executors of leased LMS and 教務Web keys. Each key is one read operation
//! of its site session (`school_session::run`): the session is recovered at
//! most once inside the call, and a read that cannot reach the school ends
//! the key as a wait for what it said lets it go on, never as a failure. A complete key commits its rows, its
//! freshness and its `sync_runs` row in one transaction inside the lease's
//! write gate, while the account's permit holds; any other ending is
//! returned to the runner, which records it.
use super::{Depth, Gated, KeyEnd, Lease};
use crate::{
    data::WorkspaceChangeSource,
    events::CoreEvent,
    kyomu_ingest::{apply_kyomu_payload, record_kyomu_unavailable},
    lifecycle::SchoolSite,
    lms_ingest::{
        self, apply_lms_assign, apply_lms_course, apply_lms_deadlines, apply_lms_enrolment,
        apply_lms_history, IngestError,
    },
    net::{
        kyomu,
        moodle::{self, KeyError, LmsReader, MoodleClient},
        BoxFuture, Host, NetError,
    },
    ports::EventSink,
    school_auth::SchoolAuthProfile,
    school_service::{IngestOutcome, IngestState},
    school_session::{Attempt, Operation, RunError, SchoolSessions, Site},
    sync_state::{self, Dataset, Key, RunOutcome, RunRecord},
};
use adpt_isct::{
    kyomu::KyomuPayload,
    lms::{
        LmsAssignStates, LmsCourseContent, LmsDeadlines, LmsEnrolment, LmsHistoryMonth,
        StoredModule,
    },
};
use chrono::{DateTime, Utc};
use std::{collections::HashMap, sync::Arc};
use url::Url;

/// What the school executors read with.
#[derive(Clone)]
pub(crate) struct SchoolReaders {
    pub(crate) sessions: SchoolSessions,
    pub(crate) moodle: Arc<MoodleClient>,
}

fn paused_at(until_unix: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(until_unix, 0).unwrap_or_else(Utc::now)
}

/// The ending of a key whose session step failed.
pub(crate) fn session_end<E>(
    error: RunError<E>,
    profile: SchoolAuthProfile,
    source: String,
    op: impl FnOnce(E) -> KeyEnd,
) -> KeyEnd {
    match error {
        RunError::Op(error) => op(error),
        RunError::Blocked(wait) => KeyEnd::Blocked {
            profile,
            wait,
            source,
        },
        RunError::Net(error) => net_end(error, source),
        RunError::Cancelled => KeyEnd::Cancelled { source },
        RunError::Deadline => KeyEnd::failed("timeout", source),
    }
}

/// A network ending: a paused host holds every key of the host.
pub(crate) fn net_end(error: NetError, source: String) -> KeyEnd {
    match error {
        NetError::HostPaused { host, until_unix } => KeyEnd::HostPaused {
            host,
            until: paused_at(until_unix),
            source,
        },
        NetError::Cancelled => KeyEnd::Cancelled { source },
        error => KeyEnd::failed(error.kind(), source),
    }
}

/// One LMS key, as an operation of the LMS session. The reader the worker
/// keeps open across keys is opened on the first attempt that needs it.
struct LmsKey<'a> {
    moodle: &'a Arc<MoodleClient>,
    reader: &'a mut Option<LmsReader>,
    lease: &'a Lease,
    stored: &'a HashMap<u64, StoredModule>,
}

impl Operation for LmsKey<'_> {
    type Output = LmsRead;
    type Error = KeyError;

    fn attempt(&mut self, attempt: Attempt) -> BoxFuture<'_, Result<LmsRead, KeyError>> {
        Box::pin(async move {
            if self.reader.is_none() {
                *self.reader = Some(self.moodle.open(&attempt).await?);
            }
            let Some(open) = self.reader.as_mut() else {
                return Err(KeyError::new(
                    "open",
                    moodle::MoodleError::Malformed("reader"),
                ));
            };
            let key = &self.lease.key;
            match key.dataset {
                Dataset::LmsEnrolment => match self.lease.depth {
                    Depth::Light => open.current_enrolment(&attempt).await,
                    Depth::Full => open.full_enrolment(&attempt).await,
                }
                .map(LmsRead::Enrolment),
                Dataset::LmsDeadlines => open.deadlines(&attempt).await.map(LmsRead::Deadlines),
                Dataset::LmsCourse => {
                    let id = lms_course_key(key).ok_or_else(invalid_key)?;
                    open.course(&attempt, id, self.stored)
                        .await
                        .map(LmsRead::Course)
                }
                Dataset::LmsAssign => {
                    let id = lms_course_key(key).ok_or_else(invalid_key)?;
                    open.assign(&attempt, id).await.map(LmsRead::Assign)
                }
                Dataset::LmsHistory => {
                    let (year, month) = parse_month(&key.key).ok_or_else(invalid_key)?;
                    open.history(&attempt, year, month)
                        .await
                        .map(LmsRead::History)
                }
                _ => Err(invalid_key()),
            }
        })
    }
}

fn invalid_key() -> KeyError {
    KeyError::new("key", moodle::MoodleError::Malformed("invalid key"))
}

/// Reads and commits one leased LMS key. `reader` is the transport the
/// worker keeps open across keys; it is opened here when empty.
pub(crate) async fn read_lms_key(
    readers: &SchoolReaders,
    reader: &mut Option<LmsReader>,
    lease: &Lease,
) -> KeyEnd {
    let key = &lease.key;
    let source = moodle::source(key.dataset);
    let Some(permit) = readers.sessions.permit(SchoolAuthProfile::Science) else {
        return KeyEnd::Cancelled {
            source: source.into(),
        };
    };
    let stored = if key.dataset == Dataset::LmsCourse {
        let Some(id) = lms_course_key(key) else {
            return KeyEnd::failed("invalid-key", source);
        };
        match lms_stored_modules(lease, id).await {
            Ok(stored) => stored,
            Err(error) => {
                log::warn!(target: "app_core::sync", "LMS modules of {key} not read: {error}");
                return KeyEnd::failed("storage", source);
            }
        }
    } else {
        HashMap::new()
    };
    let mut operation = LmsKey {
        moodle: &readers.moodle,
        reader,
        lease,
        stored: &stored,
    };
    let read = readers
        .sessions
        .run(Site::Lms, &permit, lease.deadline, &mut operation)
        .await;
    match read {
        Ok(_) if lease.is_cancelled() || permit.is_cancelled() => KeyEnd::Cancelled {
            source: source.into(),
        },
        // Committed while the permit holds: a sign-out or reset waits for it.
        Ok(read) => lms_commit(lease, read).await,
        Err(error) => session_end(error, SchoolAuthProfile::Science, source.into(), |error| {
            lms_end(key, &error)
        }),
    }
}

/// What the store holds of a course's modules, so its read fetches only
/// changed details.
async fn lms_stored_modules(
    lease: &Lease,
    course: u64,
) -> Result<HashMap<u64, StoredModule>, String> {
    let store = lease.writer().store().clone();
    tokio::task::spawn_blocking(move || {
        store.write(None, |transaction| {
            lms_ingest::stored_modules(transaction, course)
        })
    })
    .await
    .map_err(|_| "stopped".to_string())?
    .map_err(|error| error.to_string())
}

/// One 教務Web page, as an operation of the 教務Web session.
struct KyomuPage<'a> {
    kind: &'a str,
}

impl Operation for KyomuPage<'_> {
    type Output = KyomuPayload;
    type Error = NetError;

    fn attempt(&mut self, attempt: Attempt) -> BoxFuture<'_, Result<KyomuPayload, NetError>> {
        Box::pin(async move { kyomu::page(&attempt, self.kind).await })
    }
}

/// Reads and commits one leased 教務Web page.
pub(crate) async fn read_kyomu_page(
    readers: &SchoolReaders,
    events: Option<&Arc<dyn EventSink>>,
    lease: &Lease,
) -> KeyEnd {
    let site = SchoolSite::Kyomu;
    let source = adpt_isct::kyomu::page_url(&lease.key.key)
        .and_then(|url| Url::parse(&url).ok())
        .map(|url| sync_state::source_of_url(&url))
        .unwrap_or_default();
    let Some(kind) = adpt_isct::kyomu::PAGES
        .iter()
        .map(|(_, kind)| *kind)
        .find(|kind| *kind == lease.key.key)
    else {
        return KeyEnd::failed("unsupported", source);
    };
    let Some(permit) = readers.sessions.permit(SchoolAuthProfile::TokyoTech) else {
        return KeyEnd::Cancelled { source };
    };
    let emit = |state: IngestState, changed: usize, message: String| {
        if let Some(events) = events {
            let _ = events.emit(CoreEvent::SchoolIngest(IngestOutcome {
                site,
                kind: kind.into(),
                state,
                changed,
                message,
            }));
        }
    };
    let read = readers
        .sessions
        .run(
            Site::Kyomu,
            &permit,
            lease.deadline,
            &mut KyomuPage { kind },
        )
        .await;
    let payload = match read {
        Ok(_) if lease.is_cancelled() || permit.is_cancelled() => {
            return KeyEnd::Cancelled { source };
        }
        Ok(payload) => payload,
        Err(error) => {
            return session_end(
                error,
                SchoolAuthProfile::TokyoTech,
                source.clone(),
                |error| {
                    if !matches!(error, NetError::HostPaused { .. } | NetError::Cancelled) {
                        emit(IngestState::Error, 0, error.to_string());
                    }
                    net_end(error, source.clone())
                },
            );
        }
    };
    let unavailable = payload.status == "unavailable";
    let message = payload.message.clone();
    let writer = lease.writer();
    let (key, run, started, record_source) = (
        lease.key.clone(),
        lease.run.clone(),
        lease.started,
        source.clone(),
    );
    let applied = tokio::task::spawn_blocking(move || {
        writer.write(Some(WorkspaceChangeSource::SchoolPull), |transaction| {
            let changed = if unavailable {
                record_kyomu_unavailable(transaction, &payload).map(|()| 0)?
            } else {
                apply_kyomu_payload(transaction, &payload)?
            };
            sync_state::record_run(
                transaction,
                &RunRecord {
                    run: &run,
                    key: &key,
                    started_at: started,
                    finished_at: Utc::now(),
                    outcome: if unavailable {
                        RunOutcome::Unavailable
                    } else {
                        RunOutcome::Complete
                    },
                    changed,
                    requests: None,
                    bytes: None,
                    source: &record_source,
                    error_kind: None,
                },
            )?;
            Ok::<_, IngestError>(changed)
        })
    })
    .await;
    drop(permit);
    match applied {
        Ok(Ok(changed)) => {
            if unavailable {
                emit(IngestState::Unavailable, 0, message);
            } else {
                emit(IngestState::Ok, changed, format!("{changed} rows changed"));
            }
            KeyEnd::Done { changed }
        }
        Ok(Err(Gated::Stale)) => KeyEnd::Stale,
        // A page projection refused rolled back whole; it backs off like
        // any other failed read.
        Ok(Err(Gated::Failed(error))) => {
            emit(IngestState::Error, 0, error.to_string());
            KeyEnd::failed("parse", source)
        }
        Err(_) => KeyEnd::failed("stopped", source),
    }
}

/// One complete key: its rows, its freshness and its run record in one
/// transaction, inside the write gate.
async fn lms_commit(lease: &Lease, read: LmsRead) -> KeyEnd {
    let source = moodle::source(lease.key.dataset);
    let writer = lease.writer();
    let (key, run, started) = (lease.key.clone(), lease.run.clone(), lease.started);
    let applied = tokio::task::spawn_blocking(move || {
        writer.write(Some(WorkspaceChangeSource::SchoolPull), |transaction| {
            let now = Utc::now();
            let changed = match &read {
                LmsRead::Enrolment(read) => apply_lms_enrolment(transaction, read, now),
                LmsRead::Deadlines(read) => apply_lms_deadlines(transaction, read, now),
                LmsRead::Course(read) => apply_lms_course(transaction, read, now),
                LmsRead::Assign(read) => apply_lms_assign(transaction, read, now),
                LmsRead::History(read) => apply_lms_history(transaction, read, now),
            }?;
            sync_state::record_run(
                transaction,
                &RunRecord {
                    run: &run,
                    key: &key,
                    started_at: started,
                    finished_at: now,
                    outcome: RunOutcome::Complete,
                    changed,
                    requests: None,
                    bytes: None,
                    source,
                    error_kind: None,
                },
            )?;
            Ok::<_, IngestError>(changed)
        })
    })
    .await;
    match applied {
        Ok(Ok(changed)) => KeyEnd::Done { changed },
        Ok(Err(Gated::Stale)) => KeyEnd::Stale,
        Ok(Err(Gated::Failed(error))) => {
            // The transaction rolled back: nothing of this key landed.
            log::warn!(target: "app_core::sync", "LMS key {} not stored: {error}", lease.key);
            let kind = match error {
                IngestError::InvalidPayload(_) => "parse",
                IngestError::Storage(_) => "storage",
                IngestError::NotEnrolled(_) => {
                    return KeyEnd::NotEnrolled {
                        source: source.into(),
                    }
                }
            };
            KeyEnd::failed(kind, source)
        }
        Err(_) => KeyEnd::failed("stopped", source),
    }
}

/// The ending of an LMS key that could not be read in full.
fn lms_end(key: &Key, error: &KeyError) -> KeyEnd {
    let source = moodle::source(key.dataset).to_string();
    if error.is_cancelled() {
        return KeyEnd::Cancelled { source };
    }
    log::warn!(target: "app_core::sync", "LMS key {key} failed: {error}");
    if let Some(until) = error.paused_until() {
        return KeyEnd::HostPaused {
            host: Host::Moodle,
            until: paused_at(until),
            source,
        };
    }
    KeyEnd::Failed {
        kind: error.kind(),
        source,
    }
}

/// A complete key read, ready to commit.
pub(crate) enum LmsRead {
    Enrolment(LmsEnrolment),
    Deadlines(LmsDeadlines),
    Course(LmsCourseContent),
    Assign(LmsAssignStates),
    History(LmsHistoryMonth),
}

fn lms_course_key(key: &Key) -> Option<u64> {
    key.key.parse().ok().filter(|id| *id > 0)
}

/// `2026-04` → `(2026, 4)`.
fn parse_month(key: &str) -> Option<(i32, u32)> {
    let (year, month) = key.split_once('-')?;
    let month = month
        .parse()
        .ok()
        .filter(|month| (1..=12).contains(month))?;
    Some((year.parse().ok()?, month))
}
