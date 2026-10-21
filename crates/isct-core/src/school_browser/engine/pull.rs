//! Executors of leased LMS and 教務Web keys over Core's HTTP clients. The
//! WebView only signs in; LMS and 教務Web data are read by `net::moodle` and
//! `net::kyomu`. Each complete key commits its rows, its freshness and its
//! `sync_runs` row in one transaction inside the lease's write gate; any
//! other ending is returned to the runner, which records it.
use super::*;
use crate::{
    kyomu_ingest::{apply_kyomu_payload, record_kyomu_unavailable},
    lms_ingest::{
        self, apply_lms_assign, apply_lms_course, apply_lms_deadlines, apply_lms_enrolment,
        apply_lms_history, IngestError,
    },
    net::{
        kyomu::KyomuClient,
        moodle::{self, KeyError, LmsReader, MoodleClient},
        BoxFuture, Host, Http, KeyOutcome, NetError, SessionCookies, SignIn, SignInGate,
    },
    school_parser::lms::{
        LmsAssignStates, LmsCourseContent, LmsDeadlines, LmsEnrolment, LmsHistoryMonth,
        StoredModule,
    },
    sync_engine::{Gated, KeyEnd, Lease},
    sync_state::{self, Dataset, Key, RunOutcome, RunRecord},
};
use chrono::{DateTime, Utc};

/// The HTTP clients the executors read with. Built once, when the shell
/// attaches its cookie port.
pub(crate) struct Transport {
    otp_setup: crate::school_auth::otp_setup::OtpSetupClient,
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
            // A phone app out of front may not run the WebView: iOS and
            // Android stop or throttle it, so a background wake reads only
            // what the token and the stored session cookies reach. The read
            // ends as a sign-in wait and runs again in the foreground. A
            // desktop window out of focus still signs in.
            #[cfg(any(target_os = "ios", target_os = "android"))]
            if !inner.hints.in_front() {
                return Err(NetError::SignInRequired);
            }
            let engine = SchoolBrowserEngine(inner);
            let fail = |error: SchoolBrowserError| match error {
                SchoolBrowserError::Cancelled => NetError::Cancelled,
                error => NetError::Invalid(error.to_string()),
            };
            if !engine.ensure_session(profile).await.map_err(fail)? {
                return Err(NetError::SignInRequired);
            }
            // A live GetAccess sign-in is not yet a 教務Web session: only
            // its entry page starts one (see `enter_kyomu`).
            if profile == SchoolAuthProfile::TokyoTech
                && !engine.enter_kyomu().await.map_err(fail)?
            {
                return Err(NetError::SignInRequired);
            }
            Ok(())
        })
    }
}

fn paused_at(until_unix: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(until_unix, 0).unwrap_or_else(Utc::now)
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
            otp_setup: crate::school_auth::otp_setup::OtpSetupClient::new(
                http.clone(),
                cookies.clone(),
            ),
            moodle: MoodleClient::new(http, cookies, sign_in, self.0.auth.clone()),
            kyomu: kyomu.clone(),
        };
        self.0.transport.set(transport).map_err(|_| {
            SchoolBrowserError::InvalidInput("School transport is already attached.".into())
        })?;
        Ok(kyomu)
    }

    /// The 教務Web client the official catalog executor reads with.
    pub fn kyomu_client(&self) -> Result<Arc<KyomuClient>, SchoolBrowserError> {
        Ok(self.transport()?.kyomu.clone())
    }

    fn transport(&self) -> Result<&Transport, SchoolBrowserError> {
        self.0
            .transport
            .get()
            .ok_or_else(|| SchoolBrowserError::Platform("School transport is unavailable.".into()))
    }

    /// Reads only enrollment state, never the OTP secret or registration form.
    pub async fn school_otp_setup(
        &self,
    ) -> Result<crate::school_auth::SchoolOtpSetup, crate::school_auth::OtpSetupError> {
        self.transport()
            .map_err(|_| crate::school_auth::OtpSetupError::Unavailable)?
            .otp_setup
            .read()
            .await
    }

    /// Reads and commits one leased LMS key. `reader` is the transport the
    /// worker keeps open across keys; it is opened here when empty.
    pub(crate) async fn read_lms_key<'r>(
        &'r self,
        reader: &mut Option<LmsReader<'r>>,
        lease: &Lease,
    ) -> KeyEnd {
        let key = &lease.key;
        let source = moodle::source(key.dataset);
        let moodle = match self.transport() {
            Ok(transport) => &transport.moodle,
            Err(_) => {
                return KeyEnd::SignInRequired {
                    profile: SchoolAuthProfile::Science,
                    source: source.into(),
                }
            }
        };
        if reader.is_none() {
            // Boxed with an explicit `Send` bound: the worker future is
            // otherwise rejected by rustc's higher-ranked auto-trait inference.
            let opened: BoxFuture<'r, Result<LmsReader<'r>, KeyError>> = Box::pin(moodle.open());
            match opened.await {
                Ok(opened) => *reader = Some(opened),
                Err(error) => return lms_end(key, &error),
            }
        }
        let Some(open) = reader.as_mut() else {
            return KeyEnd::failed("unavailable", source);
        };
        let read = match key.dataset {
            Dataset::LmsEnrolment => {
                let read: BoxFuture<'_, Result<LmsEnrolment, KeyError>> =
                    Box::pin(open.enrolment());
                read.await.map(LmsRead::Enrolment)
            }
            Dataset::LmsDeadlines => {
                let read: BoxFuture<'_, Result<LmsDeadlines, KeyError>> =
                    Box::pin(open.deadlines());
                read.await.map(LmsRead::Deadlines)
            }
            Dataset::LmsCourse => {
                let Some(id) = lms_course_key(key) else {
                    return KeyEnd::failed("invalid-key", source);
                };
                let stored = match self.lms_stored_modules(lease, id).await {
                    Ok(stored) => stored,
                    Err(error) => {
                        log::warn!(target: "isct_core::school_browser", "LMS modules of {key} not read: {error}");
                        return KeyEnd::failed("storage", source);
                    }
                };
                let read: BoxFuture<'_, Result<LmsCourseContent, KeyError>> =
                    Box::pin(open.course(id, &stored));
                read.await.map(LmsRead::Course)
            }
            Dataset::LmsAssign => {
                let Some(id) = lms_course_key(key) else {
                    return KeyEnd::failed("invalid-key", source);
                };
                let read: BoxFuture<'_, Result<LmsAssignStates, KeyError>> =
                    Box::pin(open.assign(id));
                read.await.map(LmsRead::Assign)
            }
            Dataset::LmsHistory => {
                let Some((year, month)) = parse_month(&key.key) else {
                    return KeyEnd::failed("invalid-key", source);
                };
                let read: BoxFuture<'_, Result<LmsHistoryMonth, KeyError>> =
                    Box::pin(open.history(year, month));
                read.await.map(LmsRead::History)
            }
            _ => return KeyEnd::failed("unsupported", source),
        };
        match read {
            Ok(_) if lease.is_cancelled() => KeyEnd::Cancelled {
                source: source.into(),
            },
            Ok(read) => lms_commit(lease, read).await,
            Err(error) => lms_end(key, &error),
        }
    }

    /// What the store holds of a course's modules, so its read fetches only
    /// changed details.
    async fn lms_stored_modules(
        &self,
        lease: &Lease,
        course: u64,
    ) -> Result<HashMap<u64, StoredModule>, SchoolBrowserError> {
        let store = lease.writer().store().clone();
        tokio::task::spawn_blocking(move || {
            store.write(None, |transaction| {
                lms_ingest::stored_modules(transaction, course)
            })
        })
        .await
        .map_err(|_| SchoolBrowserError::Stopped)?
        .map_err(|error| SchoolBrowserError::Storage(error.to_string()))
    }

    /// Reads and commits one leased 教務Web page.
    pub(crate) async fn read_kyomu_page(&self, lease: &Lease) -> KeyEnd {
        let site = SchoolSite::Kyomu;
        let source = crate::school_parser::kyomu::page_url(&lease.key.key)
            .and_then(|url| Url::parse(&url).ok())
            .map(|url| sync_state::source_of_url(&url))
            .unwrap_or_default();
        let Some(kind) = crate::school_parser::kyomu::PAGES
            .iter()
            .map(|(_, kind)| *kind)
            .find(|kind| *kind == lease.key.key)
        else {
            return KeyEnd::failed("unsupported", source);
        };
        let kyomu = match self.transport() {
            Ok(transport) => transport.kyomu.clone(),
            Err(_) => {
                return KeyEnd::SignInRequired {
                    profile: SchoolAuthProfile::TokyoTech,
                    source,
                }
            }
        };
        let outcome = |state: IngestState, changed: usize, message: String| IngestOutcome {
            site,
            kind: kind.into(),
            state,
            changed,
            message,
        };
        let end = match kyomu.page(kind).await {
            Ok(KeyOutcome::Complete(_)) if lease.is_cancelled() => {
                return KeyEnd::Cancelled { source };
            }
            Ok(KeyOutcome::Complete(payload)) => {
                let unavailable = payload.status == "unavailable";
                let message = payload.message.clone();
                let writer = lease.writer();
                let (key, account, run, started, record_source) = (
                    lease.key.clone(),
                    lease.account.clone(),
                    lease.run.clone(),
                    lease.started,
                    source.clone(),
                );
                let applied = tokio::task::spawn_blocking(move || {
                    writer.write(Some(WorkspaceChangeSource::SchoolPull), |transaction| {
                        let changed = if unavailable {
                            record_kyomu_unavailable(transaction, &payload, &account).map(|()| 0)?
                        } else {
                            apply_kyomu_payload(transaction, &payload, &account)?
                        };
                        sync_state::record_run(
                            transaction,
                            &RunRecord {
                                run: &run,
                                key: &key,
                                account: &account,
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
                match applied {
                    Ok(Ok(changed)) => {
                        self.clear_kyomu_maintenance();
                        let state = if unavailable {
                            IngestState::Unavailable
                        } else {
                            IngestState::Ok
                        };
                        let message = if unavailable {
                            message
                        } else {
                            format!("{changed} rows changed")
                        };
                        let _ = self
                            .0
                            .events
                            .emit(CoreEvent::SchoolIngest(outcome(state, changed, message)));
                        return KeyEnd::Done { changed };
                    }
                    Ok(Err(Gated::Stale)) => return KeyEnd::Stale,
                    // A page projection refused rolled back whole; it backs
                    // off like any other failed read.
                    Ok(Err(Gated::Failed(error))) => (
                        outcome(IngestState::Error, 0, error.to_string()),
                        KeyEnd::failed("parse", source),
                    ),
                    Err(_) => return KeyEnd::failed("stopped", source),
                }
            }
            // The 教務 client never splits a page; treat a split as failed.
            Ok(KeyOutcome::Partial { missing, .. }) => (
                outcome(
                    IngestState::Error,
                    0,
                    format!("not read: {}", missing.join(", ")),
                ),
                KeyEnd::failed("partial", source),
            ),
            Ok(KeyOutcome::Unavailable(what)) => (
                outcome(IngestState::Unavailable, 0, what.into()),
                KeyEnd::failed("unsupported", source),
            ),
            Ok(KeyOutcome::SignInRequired) | Err(NetError::SignInRequired) => (
                sign_in_required(site, kind),
                KeyEnd::SignInRequired {
                    profile: SchoolAuthProfile::TokyoTech,
                    source,
                },
            ),
            // Maintenance pauses the host, not this page.
            Err(NetError::HostPaused { host, until_unix }) => {
                let notice = kyomu
                    .maintenance_notice()
                    .unwrap_or_else(|| "School service under maintenance.".into());
                // Reports the maintenance outcome itself.
                self.0.report_kyomu_maintenance(&notice);
                return KeyEnd::HostPaused {
                    host,
                    until: paused_at(until_unix),
                    notice: Some(notice),
                    source,
                };
            }
            Err(NetError::Cancelled) => return KeyEnd::Cancelled { source },
            Err(error) => (
                outcome(IngestState::Error, 0, error.to_string()),
                KeyEnd::failed(error.kind(), source),
            ),
        };
        let (reported, end) = end;
        let _ = self.0.events.emit(CoreEvent::SchoolIngest(reported));
        end
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

/// One complete key: its rows, its freshness and its run record in one
/// transaction, inside the write gate.
async fn lms_commit(lease: &Lease, read: LmsRead) -> KeyEnd {
    let source = moodle::source(lease.key.dataset);
    let writer = lease.writer();
    let (key, account, run, started) = (
        lease.key.clone(),
        lease.account.clone(),
        lease.run.clone(),
        lease.started,
    );
    let applied = tokio::task::spawn_blocking(move || {
        writer.write(Some(WorkspaceChangeSource::SchoolPull), |transaction| {
            let now = Utc::now();
            let changed = match &read {
                LmsRead::Enrolment(read) => apply_lms_enrolment(transaction, read, &account, now),
                LmsRead::Deadlines(read) => apply_lms_deadlines(transaction, read, &account, now),
                LmsRead::Course(read) => apply_lms_course(transaction, read, &account, now),
                LmsRead::Assign(read) => apply_lms_assign(transaction, read, &account, now),
                LmsRead::History(read) => apply_lms_history(transaction, read, &account, now),
            }?;
            sync_state::record_run(
                transaction,
                &RunRecord {
                    run: &run,
                    key: &key,
                    account: &account,
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
            log::warn!(target: "isct_core::school_browser", "LMS key {} not stored: {error}", lease.key);
            let kind = match error {
                IngestError::InvalidPayload(_) => "parse",
                IngestError::Storage(_) => "storage",
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
    log::warn!(target: "isct_core::school_browser", "LMS key {key} failed: {error}");
    if error.is_sign_in() {
        return KeyEnd::SignInRequired {
            profile: SchoolAuthProfile::Science,
            source,
        };
    }
    if let Some(until) = error.paused_until() {
        return KeyEnd::HostPaused {
            host: Host::Moodle,
            until: paused_at(until),
            notice: None,
            source,
        };
    }
    KeyEnd::Failed {
        kind: error.kind(),
        source,
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

/// A complete key read, ready to commit.
enum LmsRead {
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
