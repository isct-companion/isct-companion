//! Coordinated pulls over Core's HTTP clients. The WebView only signs in;
//! LMS and 教務Web data are read by `net::moodle` and `net::kyomu`.
use super::*;
use crate::{
    kyomu_ingest::{apply_kyomu_payload, record_kyomu_unavailable},
    lms_ingest::apply_lms_payload,
    net::{
        kyomu::KyomuClient,
        moodle::{LmsScope, MoodleClient},
        BoxFuture, Http, KeyOutcome, NetError, SessionCookies, SignIn, SignInGate,
    },
    school_parser::lms::LmsPayload,
    sync_state::{self, Commit, Dataset, Key, Outcome},
};

/// The HTTP clients a pull reads with. Built once, when the shell attaches its
/// cookie port.
pub(crate) struct Transport {
    pub(crate) moodle: MoodleClient,
    pub(crate) kyomu: Arc<KyomuClient>,
}

/// Sign-in requests from the HTTP clients run the existing WebView SSO.
struct EngineSignIn(Weak<Inner>);

impl SignIn for EngineSignIn {
    fn sign_in<'a>(&'a self, profile: SchoolAuthProfile) -> BoxFuture<'a, Result<(), NetError>> {
        Box::pin(async move {
            let inner = self
                .0
                .upgrade()
                .ok_or_else(|| NetError::Invalid("The school browser has stopped.".into()))?;
            match SchoolBrowserEngine(inner).ensure_session(profile).await {
                Ok(true) => Ok(()),
                Ok(false) => Err(NetError::SignInRequired),
                Err(SchoolBrowserError::Cancelled) => Err(NetError::Cancelled),
                Err(error) => Err(NetError::Invalid(error.to_string())),
            }
        })
    }
}

impl SchoolBrowserEngine {
    /// Builds the HTTP clients over the shell's cookie port. Called once.
    pub fn attach_transport(
        &self,
        http: Http,
        cookies: Arc<dyn SessionCookies>,
    ) -> Result<Arc<KyomuClient>, SchoolBrowserError> {
        let sign_in = SignInGate::new(Arc::new(EngineSignIn(Arc::downgrade(&self.0))));
        let kyomu = Arc::new(KyomuClient::new(
            http.clone(),
            cookies.clone(),
            sign_in.clone(),
        ));
        let transport = Transport {
            moodle: MoodleClient::new(http, cookies, sign_in, self.0.auth.clone()),
            kyomu: kyomu.clone(),
        };
        self.0.transport.set(transport).map_err(|_| {
            SchoolBrowserError::InvalidInput("School transport is already attached.".into())
        })?;
        Ok(kyomu)
    }

    /// The 教務Web client course search uses.
    pub fn kyomu_client(&self) -> Result<Arc<KyomuClient>, SchoolBrowserError> {
        Ok(self.transport()?.kyomu.clone())
    }

    fn transport(&self) -> Result<&Transport, SchoolBrowserError> {
        self.0
            .transport
            .get()
            .ok_or_else(|| SchoolBrowserError::Platform("School transport is unavailable.".into()))
    }

    pub(super) async fn pull_once(
        &self,
        site: SchoolSite,
        course_id: Option<u64>,
        context: &PullContext,
    ) -> Result<Vec<IngestOutcome>, SchoolBrowserError> {
        match site {
            SchoolSite::ScienceLms => self
                .pull_lms(course_id, context)
                .await
                .map(|outcome| vec![outcome]),
            SchoolSite::Kyomu => self.pull_kyomu(context).await,
            _ => Err(SchoolBrowserError::InvalidInput(
                "Only LMS and 教務Web can be pulled.".into(),
            )),
        }
    }

    async fn pull_lms(
        &self,
        course_id: Option<u64>,
        context: &PullContext,
    ) -> Result<IngestOutcome, SchoolBrowserError> {
        let site = SchoolSite::ScienceLms;
        let has_all = self
            .0
            .store
            .freshness(Some(Dataset::LmsEnrolment))
            .map_err(|e| SchoolBrowserError::Storage(e.to_string()))?
            .iter()
            .any(|state| state.fetched_at.is_some());
        let scope = if course_id.is_some() {
            LmsScope::Summary
        } else if context.current_quarter() {
            LmsScope::CurrentQuarter
        } else if context.priority() == PullPriority::Manual || !has_all {
            LmsScope::Full
        } else {
            LmsScope::Summary
        };
        let events = self.0.events.clone();
        let progress = move |progress: SchoolPullProgress| {
            let _ = events.emit(CoreEvent::SchoolPullProgress(progress));
        };
        // Boxed with an explicit `Send` bound: the executor future is
        // otherwise rejected by rustc's higher-ranked auto-trait inference.
        let moodle = &self.transport()?.moodle;
        let read: BoxFuture<'_, Result<KeyOutcome<LmsPayload>, NetError>> =
            Box::pin(moodle.pull(scope, course_id, &progress));
        let read = read.await;
        if context.is_cancelled() {
            return Err(SchoolBrowserError::Cancelled);
        }
        let outcome = match read {
            Ok(KeyOutcome::Complete(payload)) => {
                self.apply_lms(payload, Vec::new(), context).await?
            }
            Ok(KeyOutcome::Partial { data, missing }) => {
                self.apply_lms(data, missing, context).await?
            }
            Ok(KeyOutcome::Unavailable(what)) => IngestOutcome {
                site,
                kind: "lms".into(),
                state: IngestState::Unavailable,
                changed: 0,
                message: what.into(),
            },
            Ok(KeyOutcome::SignInRequired) | Err(NetError::SignInRequired) => {
                sign_in_required(site, "lms")
            }
            Err(error) => return Err(net_error(error)),
        };
        let _ = self.0.events.emit(CoreEvent::SchoolIngest(outcome.clone()));
        Ok(outcome)
    }

    async fn apply_lms(
        &self,
        payload: LmsPayload,
        missing: Vec<&'static str>,
        context: &PullContext,
    ) -> Result<IngestOutcome, SchoolBrowserError> {
        if !context.applying().await {
            return Err(SchoolBrowserError::Cancelled);
        }
        let store = self.0.store.clone();
        let failed = payload.timetable_errors.len()
            + payload
                .structures
                .iter()
                .filter(|row| !row.complete)
                .count()
            + missing.len();
        let reason =
            payload.timetable_errors.first().cloned().or_else(|| {
                (!missing.is_empty()).then(|| format!("not read: {}", missing.join(", ")))
            });
        let account = self
            .0
            .auth
            .sync_account(SchoolAuthProfile::Science)
            .map_err(|error| SchoolBrowserError::Storage(error.to_string()))?;
        let complete = failed == 0 && payload.full_scan && payload.courses_complete;
        let applied = tokio::task::spawn_blocking(move || {
            store.write(Some(WorkspaceChangeSource::SchoolPull), |transaction| {
                let changed = apply_lms_payload(transaction, &payload)?;
                if complete {
                    let now = chrono::Utc::now();
                    for dataset in [Dataset::LmsEnrolment, Dataset::LmsDeadlines] {
                        sync_state::commit(
                            transaction,
                            &Key::new(dataset, "all"),
                            Commit {
                                account: &account,
                                validator: None,
                                outcome: Outcome::Complete,
                                message: None,
                            },
                            now,
                        )?;
                    }
                }
                Ok::<_, crate::lms_ingest::IngestError>(changed)
            })
        })
        .await;
        context.applied().await;
        let changed = applied
            .map_err(|_| SchoolBrowserError::Stopped)?
            .map_err(|error| SchoolBrowserError::Storage(error.to_string()))?;
        Ok(IngestOutcome {
            site: SchoolSite::ScienceLms,
            kind: "lms".into(),
            state: if failed > 0 {
                IngestState::Error
            } else {
                IngestState::Ok
            },
            changed,
            message: if failed > 0 {
                format!(
                    "LMS refresh incomplete: {failed} reads failed; cached data retained{}",
                    reason
                        .map(|reason| format!("; {reason}"))
                        .unwrap_or_default()
                )
            } else {
                format!("{changed} rows changed")
            },
        })
    }

    /// Reads the 教務Web pages one freshness key at a time. An automatic pull
    /// reads only due pages, missing ones first; a manual pull reads all six.
    /// Each page's rows, its `kyomu.page` key and its `sync_runs` row commit
    /// in one transaction; a failed page writes no rows and backs off alone.
    async fn pull_kyomu(
        &self,
        context: &PullContext,
    ) -> Result<Vec<IngestOutcome>, SchoolBrowserError> {
        use crate::data::DataError;
        use crate::lms_ingest::IngestError;
        use crate::sync_state::{self, Dataset, Due, Key, RunOutcome, RunRecord, Term};
        use chrono::{DateTime, Utc};

        /// The facts every `sync_runs` row of one page read carries.
        struct PageRun {
            run: String,
            key: Key,
            account: String,
            source: String,
            started: DateTime<Utc>,
        }

        impl PageRun {
            fn record(
                &self,
                connection: &rusqlite::Connection,
                outcome: RunOutcome,
                changed: usize,
                error_kind: Option<&str>,
            ) -> Result<(), DataError> {
                sync_state::record_run(
                    connection,
                    &RunRecord {
                        run: &self.run,
                        key: &self.key,
                        account: &self.account,
                        started_at: self.started,
                        finished_at: Utc::now(),
                        outcome,
                        changed,
                        requests: None,
                        bytes: None,
                        source: &self.source,
                        error_kind,
                    },
                )
            }

            /// A read that wrote no rows. `backoff` is false when the cause
            /// is not this page (a paused host, a lost sign-in), so the key
            /// keeps its own failure count.
            fn fail(
                &self,
                connection: &rusqlite::Connection,
                outcome: RunOutcome,
                error_kind: &str,
                backoff: bool,
            ) -> Result<(), DataError> {
                if backoff {
                    sync_state::fail(connection, &self.key, &self.account, error_kind, Utc::now())?;
                }
                self.record(connection, outcome, 0, Some(error_kind))
            }
        }

        fn error_kind(error: &NetError) -> String {
            match error {
                NetError::Offline => "offline".into(),
                NetError::Dns => "dns".into(),
                NetError::Tls(_) => "tls".into(),
                NetError::Connect(_) => "connect".into(),
                NetError::Stalled => "stalled".into(),
                NetError::Deadline => "timeout".into(),
                NetError::TooLarge => "too-large".into(),
                NetError::Http(status) => format!("http-{status}"),
                NetError::HostPaused { .. } => "host-paused".into(),
                NetError::SignInRequired => "sign-in".into(),
                NetError::Cancelled => "cancelled".into(),
                NetError::Invalid(_) => "parse".into(),
            }
        }

        let site = SchoolSite::Kyomu;
        let kyomu = self.transport()?.kyomu.clone();
        let auth = self.0.auth.clone();
        let store = self.0.store.clone();
        let (account, pages) = tokio::task::spawn_blocking(move || {
            let account = auth
                .sync_account(SchoolAuthProfile::TokyoTech)
                .map_err(|error| SchoolBrowserError::Auth(error.to_string()))?;
            let states = store
                .freshness(Some(Dataset::KyomuPage))
                .map_err(|error| SchoolBrowserError::Storage(error.to_string()))?;
            let ttl = sync_state::ttl(Dataset::KyomuPage, Term::Current);
            let now = Utc::now();
            let mut pages: Vec<(Due, &'static str)> = crate::school_parser::kyomu::PAGES
                .iter()
                .map(|(_, kind)| {
                    let state = states.iter().find(|state| state.key == *kind);
                    (sync_state::due(state, ttl, &account, now), *kind)
                })
                .collect();
            // Stable, so pages of the same urgency keep the menu order.
            pages.sort_by_key(|(due, _)| *due);
            Ok::<_, SchoolBrowserError>((account, pages))
        })
        .await
        .map_err(|_| SchoolBrowserError::Stopped)??;
        let run = new_id();
        let mut outcomes = Vec::new();
        for (due, kind) in pages {
            if context.is_cancelled() {
                return Err(SchoolBrowserError::Cancelled);
            }
            // Read again per page: a manual join can promote this pull.
            if !due.is_due() && context.priority() != PullPriority::Manual {
                continue;
            }
            let page = PageRun {
                run: run.clone(),
                key: Key::new(Dataset::KyomuPage, kind),
                account: account.clone(),
                source: crate::school_parser::kyomu::page_url(kind)
                    .and_then(|url| url::Url::parse(&url).ok())
                    .map(|url| sync_state::source_of_url(&url))
                    .unwrap_or_default(),
                started: Utc::now(),
            };
            let error_outcome = |message: String| IngestOutcome {
                site,
                kind: kind.into(),
                state: IngestState::Error,
                changed: 0,
                message,
            };
            // `failure` is the run outcome, error kind and backoff of a read
            // that wrote no rows; `abort` ends the whole pull after it.
            let mut abort = None;
            let (outcome, failure) = match kyomu.page(kind).await {
                Ok(KeyOutcome::Complete(payload)) => {
                    if !context.applying().await {
                        return Err(SchoolBrowserError::Cancelled);
                    }
                    let store = self.0.store.clone();
                    let unavailable = payload.status == "unavailable";
                    let message = payload.message.clone();
                    let applied = tokio::task::spawn_blocking(move || {
                        let written =
                            store.write(Some(WorkspaceChangeSource::SchoolPull), |transaction| {
                                let changed = if unavailable {
                                    record_kyomu_unavailable(transaction, &payload, &page.account)
                                        .map(|()| 0)?
                                } else {
                                    apply_kyomu_payload(transaction, &payload, &page.account)?
                                };
                                let outcome = if unavailable {
                                    RunOutcome::Unavailable
                                } else {
                                    RunOutcome::Complete
                                };
                                page.record(transaction, outcome, changed, None)?;
                                Ok::<_, IngestError>(changed)
                            });
                        // A page projection refused rolled back whole; it
                        // backs off like any other failed read.
                        if written.is_err() {
                            if let Err(error) = store.write(None, |transaction| {
                                page.fail(transaction, RunOutcome::Failed, "parse", true)
                            }) {
                                crate::diagnostics::report("school-pull", &error);
                            }
                        }
                        written
                    })
                    .await;
                    // The write gate covers this page's commit only; the next
                    // page can still be cancelled.
                    context.applied().await;
                    let outcome = match applied.map_err(|_| SchoolBrowserError::Stopped)? {
                        Ok(changed) => {
                            self.clear_kyomu_maintenance();
                            IngestOutcome {
                                site,
                                kind: kind.into(),
                                state: if unavailable {
                                    IngestState::Unavailable
                                } else {
                                    IngestState::Ok
                                },
                                changed,
                                message: if unavailable {
                                    message
                                } else {
                                    format!("{changed} rows changed")
                                },
                            }
                        }
                        Err(error) => error_outcome(error.to_string()),
                    };
                    let _ = self.0.events.emit(CoreEvent::SchoolIngest(outcome.clone()));
                    outcomes.push(outcome);
                    continue;
                }
                // The 教務 client never splits a page; treat a split as failed.
                Ok(KeyOutcome::Partial { missing, .. }) => (
                    error_outcome(format!("not read: {}", missing.join(", "))),
                    (RunOutcome::Failed, "partial".to_string(), true),
                ),
                Ok(KeyOutcome::Unavailable(what)) => (
                    IngestOutcome {
                        site,
                        kind: kind.into(),
                        state: IngestState::Unavailable,
                        changed: 0,
                        message: what.into(),
                    },
                    (RunOutcome::Failed, "unsupported".to_string(), true),
                ),
                Ok(KeyOutcome::SignInRequired) | Err(NetError::SignInRequired) => (
                    sign_in_required(site, kind),
                    (RunOutcome::SignInRequired, "sign-in".to_string(), false),
                ),
                // Maintenance pauses the host, not this page: no backoff.
                Err(NetError::HostPaused { .. }) => {
                    let message = kyomu
                        .maintenance_notice()
                        .unwrap_or_else(|| "School service under maintenance.".into());
                    self.0.report_kyomu_maintenance(&message);
                    (
                        IngestOutcome {
                            site,
                            kind: "service".into(),
                            state: IngestState::Maintenance,
                            changed: 0,
                            message,
                        },
                        (RunOutcome::Failed, "host-paused".to_string(), false),
                    )
                }
                // Cancelled reads write nothing: the gate is closed to them.
                Err(NetError::Cancelled) => return Err(SchoolBrowserError::Cancelled),
                Err(error) => {
                    let failure = (RunOutcome::Failed, error_kind(&error), true);
                    let outcome = error_outcome(error.to_string());
                    if outcomes.is_empty() {
                        abort = Some(net_error(error));
                    }
                    (outcome, failure)
                }
            };
            if !context.applying().await {
                return Err(SchoolBrowserError::Cancelled);
            }
            let store = self.0.store.clone();
            let (run_outcome, kind_code, backoff) = failure;
            let written = tokio::task::spawn_blocking(move || {
                store.write(None, |transaction| {
                    page.fail(transaction, run_outcome, &kind_code, backoff)
                })
            })
            .await;
            context.applied().await;
            if let Err(error) = written.map_err(|_| SchoolBrowserError::Stopped)? {
                crate::diagnostics::report("school-pull", &error);
            }
            if let Some(error) = abort {
                return Err(error);
            }
            let stop = matches!(
                outcome.state,
                IngestState::ReconnectRequired | IngestState::Maintenance
            );
            let _ = self.0.events.emit(CoreEvent::SchoolIngest(outcome.clone()));
            outcomes.push(outcome);
            if stop {
                break;
            }
        }
        Ok(outcomes)
    }

    fn clear_kyomu_maintenance(&self) {
        let recovered = self
            .0
            .state
            .with(|state| state.kyomu_maintenance.take().is_some());
        if recovered
            && self.0.state.with(|state| {
                state.sessions.get(&SchoolAuthProfile::TokyoTech)
                    == Some(&SchoolSessionState::Maintenance)
            })
        {
            self.set_session(SchoolAuthProfile::TokyoTech, SchoolSessionState::Active);
        }
    }
}

fn net_error(error: NetError) -> SchoolBrowserError {
    match error {
        NetError::Cancelled => SchoolBrowserError::Cancelled,
        NetError::Stalled | NetError::Deadline => {
            SchoolBrowserError::Timeout("The school site did not answer in time.".into())
        }
        NetError::SignInRequired => SchoolBrowserError::Auth("Sign-in required".into()),
        NetError::HostPaused { .. } => SchoolBrowserError::Maintenance(
            "The school site is paused after repeated failures.".into(),
        ),
        other => SchoolBrowserError::Platform(other.to_string()),
    }
}

pub(super) fn sign_in_required(site: SchoolSite, kind: &str) -> IngestOutcome {
    IngestOutcome {
        site,
        kind: kind.into(),
        state: IngestState::ReconnectRequired,
        changed: 0,
        message: "Sign-in required".into(),
    }
}
