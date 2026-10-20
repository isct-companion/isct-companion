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
            .last_import_runs()
            .map_err(|e| SchoolBrowserError::Storage(e.to_string()))?["lms:all"]
            .is_string();
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
        let applied = tokio::task::spawn_blocking(move || {
            store.write(Some(WorkspaceChangeSource::SchoolPull), |transaction| {
                apply_lms_payload(transaction, &payload)
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

    async fn pull_kyomu(
        &self,
        context: &PullContext,
    ) -> Result<Vec<IngestOutcome>, SchoolBrowserError> {
        let site = SchoolSite::Kyomu;
        let kyomu = self.transport()?.kyomu.clone();
        let mut outcomes = Vec::new();
        for (_, kind) in crate::school_parser::kyomu::PAGES {
            if context.is_cancelled() {
                return Err(SchoolBrowserError::Cancelled);
            }
            let outcome = match kyomu.page(kind).await {
                Ok(KeyOutcome::Complete(payload))
                | Ok(KeyOutcome::Partial { data: payload, .. }) => {
                    if !context.applying().await {
                        return Err(SchoolBrowserError::Cancelled);
                    }
                    let store = self.0.store.clone();
                    let unavailable = payload.status == "unavailable";
                    let kind = payload.kind.clone();
                    let message = payload.message.clone();
                    let applied = tokio::task::spawn_blocking(move || {
                        store.write(Some(WorkspaceChangeSource::SchoolPull), |transaction| {
                            if unavailable {
                                record_kyomu_unavailable(transaction, &payload).map(|()| 0)
                            } else {
                                apply_kyomu_payload(transaction, &payload)
                            }
                        })
                    })
                    .await;
                    // The write gate covers this page's commit only; the next
                    // page can still be cancelled.
                    context.applied().await;
                    match applied.map_err(|_| SchoolBrowserError::Stopped)? {
                        Ok(changed) => {
                            self.clear_kyomu_maintenance();
                            IngestOutcome {
                                site,
                                kind,
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
                        Err(error) => IngestOutcome {
                            site,
                            kind,
                            state: IngestState::Error,
                            changed: 0,
                            message: error.to_string(),
                        },
                    }
                }
                Ok(KeyOutcome::Unavailable(what)) => IngestOutcome {
                    site,
                    kind: kind.into(),
                    state: IngestState::Unavailable,
                    changed: 0,
                    message: what.into(),
                },
                Ok(KeyOutcome::SignInRequired) | Err(NetError::SignInRequired) => {
                    sign_in_required(site, kind)
                }
                Err(NetError::HostPaused { .. }) => {
                    let message = kyomu
                        .maintenance_notice()
                        .unwrap_or_else(|| "School service under maintenance.".into());
                    self.0.report_kyomu_maintenance(&message);
                    IngestOutcome {
                        site,
                        kind: "service".into(),
                        state: IngestState::Maintenance,
                        changed: 0,
                        message,
                    }
                }
                Err(NetError::Cancelled) => return Err(SchoolBrowserError::Cancelled),
                Err(error) if outcomes.is_empty() => return Err(net_error(error)),
                Err(error) => IngestOutcome {
                    site,
                    kind: kind.into(),
                    state: IngestState::Error,
                    changed: 0,
                    message: error.to_string(),
                },
            };
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
