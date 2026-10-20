//! Coordinated pulls over Core's HTTP clients. The WebView only signs in;
//! LMS and 教務Web data are read by `net::moodle` and `net::kyomu`.
use super::*;
use crate::{
    kyomu_ingest::{apply_kyomu_payload, record_kyomu_unavailable},
    lms_ingest::{
        self, apply_lms_assign, apply_lms_course, apply_lms_deadlines, apply_lms_enrolment,
        apply_lms_history, plan_lms_keys, IngestError,
    },
    net::{
        kyomu::KyomuClient,
        moodle::{self, KeyError, LmsReader, MoodleClient},
        BoxFuture, Http, KeyOutcome, NetError, SessionCookies, SignIn, SignInGate,
    },
    school_parser::lms::{
        LmsAssignStates, LmsCourseContent, LmsDeadlines, LmsEnrolment, LmsHistoryMonth,
        StoredModule,
    },
    sync_state::{self, Dataset, Key, RunOutcome, RunRecord},
};
use chrono::{DateTime, Utc};

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

    /// Reads LMS one freshness key at a time. A named course reads its
    /// `lms.course` and `lms.assign` keys. Otherwise: `lms.enrolment`, then
    /// `lms.deadlines`, then the course keys that are due (current-term
    /// courses always on a manual pull), their `lms.assign` keys, and the
    /// `lms.history` months that are due. Each complete key commits on its
    /// own; a failed key writes nothing but its failure, and the pull goes on.
    async fn pull_lms(
        &self,
        course_id: Option<u64>,
        context: &PullContext,
    ) -> Result<IngestOutcome, SchoolBrowserError> {
        let auth = self.0.auth.clone();
        let account =
            tokio::task::spawn_blocking(move || auth.sync_account(SchoolAuthProfile::Science))
                .await
                .map_err(|_| SchoolBrowserError::Stopped)?
                .map_err(|error| SchoolBrowserError::Auth(error.to_string()))?;
        let mut run = LmsRun {
            id: uuid::Uuid::new_v4().to_string(),
            account,
            manual: context.priority() == PullPriority::Manual,
            done: 0,
            total: 0,
            committed: 0,
            changed: 0,
            failures: Vec::new(),
            last_error: None,
            sign_in: false,
            grid_term: None,
        };
        let first: Vec<LmsKey> = match course_id {
            Some(id) => vec![
                LmsKey::new(Dataset::LmsCourse, id.to_string()),
                LmsKey::new(Dataset::LmsAssign, id.to_string()),
            ],
            None => vec![
                LmsKey::new(Dataset::LmsEnrolment, "all"),
                LmsKey::new(Dataset::LmsDeadlines, "all"),
            ],
        };
        run.total = first.len();
        // Boxed with an explicit `Send` bound: the executor future is
        // otherwise rejected by rustc's higher-ranked auto-trait inference.
        let moodle = &self.transport()?.moodle;
        let opened: BoxFuture<'_, Result<LmsReader<'_>, KeyError>> = Box::pin(moodle.open());
        let mut reader = match opened.await {
            Ok(reader) => reader,
            Err(error) => {
                let started = Utc::now();
                for planned in &first {
                    self.lms_failed(&mut run, &planned.key, started, &error)
                        .await?;
                }
                if error.is_cancelled() {
                    return Err(SchoolBrowserError::Cancelled);
                }
                return self.finish_lms(run);
            }
        };
        for planned in first {
            if !self
                .lms_key(&mut reader, &mut run, planned, context)
                .await?
            {
                return self.finish_lms(run);
            }
        }
        if course_id.is_none() {
            let plan = self.plan_lms(&run).await?;
            run.total += plan.len();
            for planned in plan {
                if !self
                    .lms_key(&mut reader, &mut run, planned, context)
                    .await?
                {
                    break;
                }
            }
        }
        self.finish_lms(run)
    }

    /// Reads and commits one key. `Ok(false)` stops the pull: every later
    /// key would fail the same way. Cancellation between keys stops it too;
    /// committed keys stay.
    async fn lms_key(
        &self,
        reader: &mut LmsReader<'_>,
        run: &mut LmsRun,
        planned: LmsKey,
        context: &PullContext,
    ) -> Result<bool, SchoolBrowserError> {
        if context.is_cancelled() {
            return Err(SchoolBrowserError::Cancelled);
        }
        self.lms_progress(run, &planned);
        let key = planned.key;
        let started = Utc::now();
        let read = match key.dataset {
            Dataset::LmsEnrolment => {
                let read: BoxFuture<'_, Result<LmsEnrolment, KeyError>> =
                    Box::pin(reader.enrolment());
                read.await.map(LmsRead::Enrolment)
            }
            Dataset::LmsDeadlines => {
                let read: BoxFuture<'_, Result<LmsDeadlines, KeyError>> =
                    Box::pin(reader.deadlines());
                read.await.map(LmsRead::Deadlines)
            }
            Dataset::LmsCourse => {
                let id = lms_course_key(&key)?;
                let stored = self.lms_stored_modules(id).await?;
                let read: BoxFuture<'_, Result<LmsCourseContent, KeyError>> =
                    Box::pin(reader.course(id, &stored));
                read.await.map(LmsRead::Course)
            }
            Dataset::LmsAssign => {
                let id = lms_course_key(&key)?;
                let read: BoxFuture<'_, Result<LmsAssignStates, KeyError>> =
                    Box::pin(reader.assign(id));
                read.await.map(LmsRead::Assign)
            }
            Dataset::LmsHistory => {
                let (year, month) = parse_month(&key.key).ok_or_else(|| {
                    SchoolBrowserError::InvalidInput("Invalid LMS history month.".into())
                })?;
                let read: BoxFuture<'_, Result<LmsHistoryMonth, KeyError>> =
                    Box::pin(reader.history(year, month));
                read.await.map(LmsRead::History)
            }
            _ => {
                return Err(SchoolBrowserError::InvalidInput(
                    "Not an LMS dataset.".into(),
                ))
            }
        };
        run.done += 1;
        match read {
            Ok(read) => {
                if context.is_cancelled() {
                    self.lms_record_failure(run, &key, started, RunOutcome::Cancelled, "cancelled")
                        .await?;
                    return Err(SchoolBrowserError::Cancelled);
                }
                if let LmsRead::Enrolment(enrolment) = &read {
                    run.grid_term = enrolment.timetables.first().map(|grid| grid.term.clone());
                }
                self.lms_commit(run, &key, started, read, context).await?;
                Ok(true)
            }
            Err(error) => {
                self.lms_failed(run, &key, started, &error).await?;
                if error.is_cancelled() {
                    return Err(SchoolBrowserError::Cancelled);
                }
                Ok(!error.stops_pull)
            }
        }
    }

    /// One complete key: its rows, its freshness and its run record in one
    /// transaction, inside the write gate.
    async fn lms_commit(
        &self,
        run: &mut LmsRun,
        key: &Key,
        started: DateTime<Utc>,
        read: LmsRead,
        context: &PullContext,
    ) -> Result<(), SchoolBrowserError> {
        if !context.applying().await {
            self.lms_record_failure(run, key, started, RunOutcome::Cancelled, "cancelled")
                .await?;
            return Err(SchoolBrowserError::Cancelled);
        }
        let store = self.0.store.clone();
        let (run_id, account, owned_key) = (run.id.clone(), run.account.clone(), key.clone());
        let applied = tokio::task::spawn_blocking(move || {
            store.write(Some(WorkspaceChangeSource::SchoolPull), |transaction| {
                let now = Utc::now();
                let changed = match &read {
                    LmsRead::Enrolment(read) => {
                        apply_lms_enrolment(transaction, read, &account, now)
                    }
                    LmsRead::Deadlines(read) => {
                        apply_lms_deadlines(transaction, read, &account, now)
                    }
                    LmsRead::Course(read) => apply_lms_course(transaction, read, &account, now),
                    LmsRead::Assign(read) => apply_lms_assign(transaction, read, &account, now),
                    LmsRead::History(read) => apply_lms_history(transaction, read, &account, now),
                }?;
                sync_state::record_run(
                    transaction,
                    &RunRecord {
                        run: &run_id,
                        key: &owned_key,
                        account: &account,
                        started_at: started,
                        finished_at: now,
                        outcome: RunOutcome::Complete,
                        changed,
                        requests: None,
                        bytes: None,
                        source: moodle::source(owned_key.dataset),
                        error_kind: None,
                    },
                )?;
                Ok::<_, IngestError>(changed)
            })
        })
        .await;
        // The gate covers this key's commit only; the next key can still be
        // cancelled.
        context.applied().await;
        match applied.map_err(|_| SchoolBrowserError::Stopped)? {
            Ok(changed) => {
                run.committed += 1;
                run.changed += changed;
                Ok(())
            }
            Err(error) => {
                // The transaction rolled back: nothing of this key landed.
                log::warn!(target: "isct_core::school_browser", "LMS key {key} not stored: {error}");
                let kind = match error {
                    IngestError::InvalidPayload(_) => "parse",
                    IngestError::Storage(_) => "storage",
                };
                self.lms_record_failure(run, key, started, RunOutcome::Failed, kind)
                    .await
            }
        }
    }

    async fn lms_failed(
        &self,
        run: &mut LmsRun,
        key: &Key,
        started: DateTime<Utc>,
        error: &KeyError,
    ) -> Result<(), SchoolBrowserError> {
        log::warn!(target: "isct_core::school_browser", "LMS key {key} failed: {error}");
        let outcome = if error.is_cancelled() {
            RunOutcome::Cancelled
        } else if error.is_sign_in() {
            run.sign_in = true;
            RunOutcome::SignInRequired
        } else {
            RunOutcome::Failed
        };
        run.last_error = Some(error.clone());
        self.lms_record_failure(run, key, started, outcome, &error.kind())
            .await
    }

    /// A key that wrote no rows: its backoff and its run record.
    async fn lms_record_failure(
        &self,
        run: &mut LmsRun,
        key: &Key,
        started: DateTime<Utc>,
        outcome: RunOutcome,
        kind: &str,
    ) -> Result<(), SchoolBrowserError> {
        run.failures.push(format!("{key} ({kind})"));
        let store = self.0.store.clone();
        let (run_id, account, key, kind) = (
            run.id.clone(),
            run.account.clone(),
            key.clone(),
            kind.to_string(),
        );
        let recorded = tokio::task::spawn_blocking(move || {
            store.write(None, |transaction| {
                let now = Utc::now();
                sync_state::fail(transaction, &key, &account, &kind, now)?;
                sync_state::record_run(
                    transaction,
                    &RunRecord {
                        run: &run_id,
                        key: &key,
                        account: &account,
                        started_at: started,
                        finished_at: now,
                        outcome,
                        changed: 0,
                        requests: None,
                        bytes: None,
                        source: moodle::source(key.dataset),
                        error_kind: Some(&kind),
                    },
                )
            })
        })
        .await
        .map_err(|_| SchoolBrowserError::Stopped)?;
        if let Err(error) = recorded {
            log::warn!(target: "isct_core::school_browser", "LMS failure not recorded: {error}");
        }
        Ok(())
    }

    /// What the store holds of a course's modules, so its read fetches only
    /// changed details.
    async fn lms_stored_modules(
        &self,
        course: u64,
    ) -> Result<HashMap<u64, StoredModule>, SchoolBrowserError> {
        let store = self.0.store.clone();
        tokio::task::spawn_blocking(move || {
            store.write(None, |transaction| {
                lms_ingest::stored_modules(transaction, course)
            })
        })
        .await
        .map_err(|_| SchoolBrowserError::Stopped)?
        .map_err(|error| SchoolBrowserError::Storage(error.to_string()))
    }

    /// The course, assignment and history keys this pull reads after the
    /// enrolment and deadlines, from what is stored now.
    async fn plan_lms(&self, run: &LmsRun) -> Result<Vec<LmsKey>, SchoolBrowserError> {
        let store = self.0.store.clone();
        let (courses, states) = tokio::task::spawn_blocking(move || {
            store.write(None, |transaction| {
                let courses = lms_ingest::stored_courses(transaction)?;
                let mut states = HashMap::new();
                for dataset in [Dataset::LmsCourse, Dataset::LmsAssign, Dataset::LmsHistory] {
                    for state in sync_state::list(transaction, Some(dataset))? {
                        states.insert(Key::new(dataset, state.key.clone()), state);
                    }
                }
                Ok::<_, IngestError>((courses, states))
            })
        })
        .await
        .map_err(|_| SchoolBrowserError::Stopped)?
        .map_err(|error| SchoolBrowserError::Storage(error.to_string()))?;
        Ok(plan_lms_keys(
            &courses,
            &states,
            &run.account,
            run.manual,
            run.grid_term.as_deref(),
            Utc::now(),
        )
        .into_iter()
        .map(|(key, course)| LmsKey {
            key,
            label: course.map(|course| {
                let term = match (course.year, course.quarter) {
                    (Some(year), Some(quarter)) => format!("{year}-{quarter}Q"),
                    _ => String::new(),
                };
                (course.id, course.title.clone(), term)
            }),
        })
        .collect())
    }

    fn lms_progress(&self, run: &LmsRun, planned: &LmsKey) {
        // The phases the status tracker knows, counting keys of the whole pull.
        let phase = match planned.key.dataset {
            Dataset::LmsEnrolment | Dataset::LmsDeadlines => "catalog",
            Dataset::LmsCourse => "course",
            _ => "assignment",
        };
        let (course_id, course_title, term) = planned.label.clone().unwrap_or_default();
        let _ = self
            .0
            .events
            .emit(CoreEvent::SchoolPullProgress(SchoolPullProgress {
                site: SchoolSite::ScienceLms,
                phase: phase.into(),
                completed: run.done,
                total: run.total,
                course_id,
                course_title: course_title.chars().take(512).collect(),
                term,
                item_title: String::new(),
            }));
    }

    /// The pull's outcome: Ok when every key committed; otherwise the failed
    /// keys. A pull that committed nothing fails as a whole, as before.
    fn finish_lms(&self, run: LmsRun) -> Result<IngestOutcome, SchoolBrowserError> {
        let site = SchoolSite::ScienceLms;
        let outcome = if run.sign_in {
            sign_in_required(site, "lms")
        } else if run.failures.is_empty() {
            IngestOutcome {
                site,
                kind: "lms".into(),
                state: IngestState::Ok,
                changed: run.changed,
                message: format!("{} rows changed", run.changed),
            }
        } else if run.committed == 0 {
            return Err(
                match run.last_error.as_ref().map(KeyError::kind).as_deref() {
                    Some("timeout" | "stalled") => SchoolBrowserError::Timeout(
                        "The school site did not answer in time.".into(),
                    ),
                    Some("host-paused") => SchoolBrowserError::Maintenance(
                        "The school site is paused after repeated failures.".into(),
                    ),
                    _ => SchoolBrowserError::Platform(lms_failure_message(&run)),
                },
            );
        } else {
            IngestOutcome {
                site,
                kind: "lms".into(),
                state: IngestState::Error,
                changed: run.changed,
                message: lms_failure_message(&run),
            }
        };
        let _ = self.0.events.emit(CoreEvent::SchoolIngest(outcome.clone()));
        Ok(outcome)
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
                    let failure = (RunOutcome::Failed, error.kind(), true);
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

/// One LMS key a pull reads; `label` names its course for progress.
struct LmsKey {
    key: Key,
    label: Option<(u64, String, String)>,
}

impl LmsKey {
    fn new(dataset: Dataset, key: impl Into<String>) -> Self {
        Self {
            key: Key::new(dataset, key),
            label: None,
        }
    }
}

/// A complete key read, ready to commit.
enum LmsRead {
    Enrolment(LmsEnrolment),
    Deadlines(LmsDeadlines),
    Course(LmsCourseContent),
    Assign(LmsAssignStates),
    History(LmsHistoryMonth),
}

/// What one LMS pull has done so far.
struct LmsRun {
    /// Correlates this pull's `sync_runs` rows.
    id: String,
    account: String,
    manual: bool,
    done: usize,
    total: usize,
    committed: usize,
    changed: usize,
    /// `lms.course/123 (timeout)` per failed key.
    failures: Vec<String>,
    last_error: Option<KeyError>,
    sign_in: bool,
    /// The term of the dashboard's current grid, once read.
    grid_term: Option<String>,
}

fn lms_failure_message(run: &LmsRun) -> String {
    format!(
        "{} of {} LMS keys failed; cached data retained; failed: {}",
        run.failures.len(),
        run.done.max(run.failures.len()),
        run.failures.join(", ")
    )
}

fn lms_course_key(key: &Key) -> Result<u64, SchoolBrowserError> {
    key.key
        .parse()
        .map_err(|_| SchoolBrowserError::InvalidInput("Invalid LMS course key.".into()))
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
