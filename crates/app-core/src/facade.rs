//! One application service owner, shared by native shells and agent adapters.
#[cfg(feature = "agent")]
use crate::agent::AgentService;
use crate::school_browser::{SchoolBrowser, SchoolBrowserEngine, SchoolBrowserError};
use crate::school_session::{
    SchoolConnectOutcome, SchoolSessionSnapshot, SchoolSessions, WebCookieSeed,
};
use crate::{
    data::{CommitObserver, DataStore, WorkspaceChangeSource},
    ports::EventSink,
    runtime::{CloudWakeError, CoreRuntime, RuntimeError},
    scheduler::{ScheduledExporter, SchedulerServices},
    school_auth::{SchoolAuthError, SchoolAuthProfile, SchoolAuthService, SchoolCredentials},
    sync_engine::{Hint, Network, SyncEngine, SyncHints, SyncSnapshot},
    workspace::{WorkspaceError, WorkspaceService},
    workspace_changes::{self, Coalescer, WorkspaceChange, COALESCE_WINDOW},
};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

pub struct Core {
    pub store: DataStore,
    pub runtime: Arc<CoreRuntime>,
    pub auth: Arc<SchoolAuthService>,
    #[cfg(feature = "agent")]
    pub agent: AgentService,
    /// Assignment workspaces under the selected course storage root.
    pub assignment_workspaces: crate::workspaces::AssignmentWorkspaces,
    /// LMS writes; the MCP tools share the slot. Only the desktop's agents
    /// write to the LMS.
    #[cfg(feature = "agent")]
    submissions: Arc<std::sync::OnceLock<crate::submissions::SubmissionService>>,
    pub workspace: WorkspaceService,
    pub reminders: crate::schedule::ReminderService,
    /// Every network read of school and reference data; `Core::reset`
    /// closes its admission.
    pub(crate) sync: Arc<SyncEngine>,
    /// The school accounts' sign-in and sessions, owned by Core.
    sessions: SchoolSessions,
    school_browser: Mutex<Option<Arc<SchoolBrowserEngine>>>,
    events: Option<Arc<CoreEvents>>,
    pub(crate) reset_lease: crate::reset::ResetLease,
    finder: crate::course_query::FinderCache,
}

struct CoreEvents {
    sink: Mutex<Option<Arc<dyn EventSink>>>,
    /// Every event passes the relay below, so the session snapshot reaches the
    /// sync planner once for all shells.
    sync: SyncHints,
}

struct NoopEvents;

impl EventSink for NoopEvents {
    fn emit(&self, _event: crate::events::CoreEvent) -> Result<(), crate::ports::EventError> {
        Ok(())
    }
}

impl EventSink for CoreEvents {
    fn emit(&self, event: crate::events::CoreEvent) -> Result<(), crate::ports::EventError> {
        if !matches!(event, crate::events::CoreEvent::SyncSnapshot(_)) {
            self.sync.observe(&event);
        }
        let sink = self
            .sink
            .lock()
            .map_err(|_| crate::ports::EventError::Delivery("Event bridge is unavailable.".into()))?
            .clone();
        let Some(sink) = sink else {
            return Err(crate::ports::EventError::Delivery(
                "Event bridge has stopped.".into(),
            ));
        };
        sink.emit(event)
    }
}

/// Every committed workspace write reaches shells and cloud sync from here,
/// whichever service, executor or agent tool made it.
struct StoreChanges {
    events: Option<StoreEvents>,
    seq: crate::data::CommitSeq,
    runtime: Arc<CoreRuntime>,
}

struct StoreEvents {
    sink: Arc<CoreEvents>,
    workspace: Arc<Coalescer>,
    /// Catalog commits by year, with the catalog version after each.
    catalog: Arc<Coalescer<u16, u64>>,
}

impl CommitObserver for StoreChanges {
    fn committed(
        &self,
        source: WorkspaceChangeSource,
        revision: Option<String>,
        tables: &BTreeSet<String>,
    ) {
        if workspace_changes::touches_user_data(tables) {
            self.runtime.request_cloud_capture();
        }
        let Some(events) = &self.events else {
            return;
        };
        let changes = workspace_changes::changes_for(tables);
        if source == WorkspaceChangeSource::UserIntent {
            // The shell that made the intent waits on this event; it is
            // never held back. Events are hints with snapshot recovery; a
            // failed delivery does not undo the committed write.
            let _ = events
                .sink
                .emit(crate::events::CoreEvent::WorkspaceChanged {
                    source,
                    revision,
                    changes: changes.into_iter().collect(),
                    seq: self.seq.get(),
                });
        } else if !changes.is_empty() {
            events.workspace.push(source, changes);
        }
    }

    /// A download lands department by department, one commit each; the
    /// shell hears it at most once per window and year.
    fn catalog_changed(&self, year: u16, version: u64) {
        if let Some(events) = &self.events {
            events.catalog.push(year, BTreeSet::from([version]));
        }
    }
}

impl CoreEvents {
    fn clear(&self) {
        if let Ok(mut sink) = self.sink.lock() {
            *sink = None;
        }
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        // The sync driver holds the engine; it ends once its tasks close.
        self.sync.stop();
    }
}

impl Core {
    /// The network starts as `Unknown`; shells send `Hint::Network`.
    pub fn new(store: DataStore, auth: Arc<SchoolAuthService>) -> Self {
        Self::build(store, auth, None)
    }

    pub fn with_events(
        store: DataStore,
        auth: Arc<SchoolAuthService>,
        events: Arc<dyn EventSink>,
    ) -> Self {
        Self::build(store, auth, Some(events))
    }

    fn build(
        store: DataStore,
        auth: Arc<SchoolAuthService>,
        events: Option<Arc<dyn EventSink>>,
    ) -> Self {
        let sync = SyncEngine::new(store.clone(), auth.clone(), Network::Unknown);
        let events = events.map(|events| {
            Arc::new(CoreEvents {
                sink: Mutex::new(Some(events)),
                sync: sync.hints(),
            })
        });
        if let Some(events) = &events {
            sync.set_events(events.clone());
        }
        let runtime = Arc::new(CoreRuntime::new(sync.hints()));
        let sessions = SchoolSessions::new(crate::net::Http::new().ok(), auth.clone());
        sessions.set_events(
            events
                .as_ref()
                .map(|events| events.clone() as Arc<dyn EventSink>)
                .unwrap_or_else(|| Arc::new(NoopEvents)),
        );
        runtime.downloads.attach_sessions(sessions.clone());
        let moodle = Arc::new(crate::net::moodle::MoodleClient::new(auth.clone()));
        sync.attach_school(crate::sync_engine::SchoolReaders {
            sessions: sessions.clone(),
            moodle: moodle.clone(),
        });
        let sync = Arc::new(sync);
        let seq = store.commit_seq_handle();
        store.observe(Arc::new(StoreChanges {
            seq: seq.clone(),
            events: events.clone().map(|events| {
                let sink = events.clone();
                // Delivered after every commit it merges has landed.
                let workspace = Coalescer::new(COALESCE_WINDOW, move |source, changes| {
                    let _ = sink.emit(crate::events::CoreEvent::WorkspaceChanged {
                        source,
                        revision: None,
                        changes,
                        seq: seq.get(),
                    });
                });
                let sink = events.clone();
                let catalog = Coalescer::new(COALESCE_WINDOW, move |year, versions: Vec<u64>| {
                    let version = versions.last().copied().unwrap_or_default();
                    let _ = sink.emit(crate::events::CoreEvent::CatalogChanged { year, version });
                });
                StoreEvents {
                    sink: events,
                    workspace,
                    catalog,
                }
            }),
            runtime: runtime.clone(),
        }));
        let assignment_workspaces = crate::workspaces::AssignmentWorkspaces::new(
            store.clone(),
            runtime.notes.clone(),
            runtime.downloads.clone(),
        );
        #[cfg(feature = "agent")]
        let submissions = Arc::new(std::sync::OnceLock::new());
        let core = Self {
            #[cfg(feature = "agent")]
            agent: AgentService::new(
                store.clone(),
                assignment_workspaces.clone(),
                submissions.clone(),
            ),
            assignment_workspaces,
            #[cfg(feature = "agent")]
            submissions,
            workspace: WorkspaceService::new(store.clone()),
            reminders: crate::schedule::ReminderService::new(store.clone()),
            sync,
            sessions: sessions.clone(),
            school_browser: Mutex::new(None),
            store,
            auth,
            runtime,
            events,
            reset_lease: Default::default(),
            finder: Default::default(),
        };
        // A ledger that cannot open leaves writes off and reads untouched.
        #[cfg(feature = "agent")]
        if let Err(error) = core.attach_submissions(Arc::new(
            crate::net::moodle::LmsSubmissions::new(moodle, sessions),
        )) {
            crate::diagnostics::report("submissions", &error);
        }
        #[cfg(not(feature = "agent"))]
        let _ = (moodle, sessions);
        core
    }

    /// A shell fact or intent for the sync planner. Never blocks.
    pub fn sync_hint(&self, hint: Hint) {
        if let Hint::Network { network } = &hint {
            self.sessions.network(*network);
        }
        self.sync.hint(hint);
    }

    /// Holds an OS background wake open: resumes, then waits until sync
    /// has nothing left to read, at most `limit`. Core signs in over HTTP on
    /// its own, so school reads run in the background too; what the time
    /// does not cover waits for the next wake. Returns whether sync went
    /// quiet in time.
    pub async fn sync_in_background(&self, limit: std::time::Duration) -> bool {
        self.runtime.resume().await;
        self.sync.wait_quiet(limit).await
    }

    /// Ends a background wake: suspends again unless the app came to the
    /// front meanwhile.
    pub async fn end_background_wake(&self) {
        if !self.sync.in_front() {
            self.suspend().await;
        }
    }

    /// The OS is about to suspend the process: no new keys are leased, and
    /// a sign-in under way stops (a password already sent waits out its
    /// pause, never resent at once).
    pub async fn suspend(&self) {
        self.sessions.suspend();
        self.runtime.suspend().await;
    }

    /// What sync is doing now. The same value is sent as `sync-snapshot`
    /// whenever it changes; shells read this on start and after a missed
    /// event.
    pub async fn sync_snapshot(&self) -> SyncSnapshot {
        self.sync.snapshot()
    }

    /// Starts the sync driver on the shell's runtime. A hint from inside a
    /// runtime starts it too.
    pub fn start(
        &self,
        handle: &tokio::runtime::Handle,
    ) -> Result<(), crate::runtime_tasks::RuntimeTaskError> {
        self.sync.start(handle)
    }

    /// Attaches the shell's school web view: the pages the person opens.
    pub fn attach_school_browser(
        &self,
        browser: Arc<dyn SchoolBrowser>,
    ) -> Result<Arc<SchoolBrowserEngine>, SchoolBrowserError> {
        let mut owner = self
            .school_browser
            .lock()
            .map_err(|_| SchoolBrowserError::Stopped)?;
        if owner.is_some() {
            return Err(SchoolBrowserError::InvalidInput(
                "School browser is already attached.".into(),
            ));
        }
        let events = self
            .events
            .as_ref()
            .map(|events| events.clone() as Arc<dyn EventSink>)
            .unwrap_or_else(|| Arc::new(NoopEvents));
        let engine = SchoolBrowserEngine::new(self.sessions.clone(), events, browser)?;
        *owner = Some(engine.clone());
        Ok(engine)
    }

    /// The shell's write-only port into the web view's cookie store, where
    /// Core writes an account's verified identity before a web operation
    /// opens a school page and clears the account's websites when it goes.
    pub fn attach_web_cookies(&self, seed: Arc<dyn WebCookieSeed>) {
        self.sessions.attach_web(seed);
    }

    /// The school accounts' sessions.
    pub fn sessions(&self) -> &SchoolSessions {
        &self.sessions
    }

    /// Opens the submission service over `transport`, once. What an earlier
    /// run left unsettled is recovered by its first call or the next wake.
    #[cfg(feature = "agent")]
    fn attach_submissions(
        &self,
        transport: Arc<dyn crate::submissions::SubmissionTransport>,
    ) -> Result<(), crate::submissions::SubmissionError> {
        let service = crate::submissions::SubmissionService::open(
            &self.store,
            self.assignment_workspaces.clone(),
            transport,
        )?;
        self.submissions.set(service).map_err(|_| {
            crate::submissions::SubmissionError::Input(
                "LMS submissions are already attached.".into(),
            )
        })
    }

    #[cfg(feature = "agent")]
    pub fn submissions(&self) -> Option<&crate::submissions::SubmissionService> {
        self.submissions.get()
    }

    pub fn school_browser(&self) -> Result<Arc<SchoolBrowserEngine>, SchoolBrowserError> {
        self.school_browser
            .lock()
            .map_err(|_| SchoolBrowserError::Stopped)?
            .clone()
            .ok_or_else(|| SchoolBrowserError::Platform("School browser is unavailable.".into()))
    }

    /// The one way a shell connects a school account: sign in over HTTP
    /// with the entered credentials (blank fields keep what is saved),
    /// through any refusal on record and in place of a sign-in running. They
    /// are saved, and reach the synchronized Keychain item and cloud sync,
    /// only once the school accepts them; anything else is the Connect
    /// form's to show and changes nothing.
    pub async fn connect_school(
        &self,
        profile: SchoolAuthProfile,
        credentials: SchoolCredentials,
    ) -> Result<SchoolConnectOutcome, CredentialMutationError> {
        // Blank fields fill from what the vault holds now, under the
        // account's permit (`SchoolSessions::connect`).
        let outcome = self.sessions.connect(profile, credentials).await?;
        self.sync.unblock(profile);
        Ok(outcome)
    }

    /// The person's Retry: one sign-in with the credentials saved now, read
    /// again first.
    pub async fn retry_school(&self, profile: SchoolAuthProfile) -> SchoolSessionSnapshot {
        let snapshot = self.sessions.retry(profile).await;
        self.sync.unblock(profile);
        snapshot
    }

    /// Both accounts' sessions as shells render them.
    pub fn school_session(&self) -> SchoolSessionSnapshot {
        self.sessions.snapshot()
    }

    /// Forgets the account's credentials and signs it out of this device:
    /// once every operation of the account has ended, the credentials,
    /// Core's session, the LMS token and the school websites in the web
    /// view go (`SchoolSessions::sign_out`).
    pub async fn delete_credentials(
        &self,
        profile: SchoolAuthProfile,
    ) -> Result<bool, CredentialMutationError> {
        let deleted = self.sessions.sign_out(profile).await;
        self.sync.unblock(profile);
        Ok(deleted?)
    }

    /// A credential file was imported: both accounts read their saved
    /// credentials again; a refusal of credentials it replaced no longer
    /// applies.
    pub fn credentials_imported(&self) {
        for profile in [SchoolAuthProfile::Science, SchoolAuthProfile::TokyoTech] {
            self.sessions.credentials_changed(profile);
            self.sync.unblock(profile);
        }
    }

    /// Ports are scoped to operations, so completed work releases native handles.
    pub fn services(&self, exporter: Arc<dyn ScheduledExporter>) -> SchedulerServices {
        SchedulerServices {
            store: self.store.clone(),
            exporter,
        }
    }

    /// Evaluate a page's related queries against one coherent database snapshot.
    pub fn schedule_query(
        &self,
        queries: Vec<crate::schedule::WorkspaceQuery>,
    ) -> Result<ScheduleQuerySnapshot, ReadModelError> {
        if queries.len() > 64 {
            return Err(ReadModelError::InvalidInput(
                "Too many schedule queries.".into(),
            ));
        }
        // Read before the data, so it is never newer than what was read.
        let seq = self.store.commit_seq();
        let reads = crate::schedule::reads_of(&queries);
        // Only the tables the queries read (`WorkspaceQuery::reads`).
        let fields = workspace_changes::loaded_fields(reads.iter().copied());
        let (workspace, school, _) = self.store.view(&fields).map_err(WorkspaceError::from)?;
        let values = queries
            .into_iter()
            .map(|query| crate::schedule::query(&workspace, &school, query))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ScheduleQuerySnapshot { values, reads, seq })
    }

    pub fn finder_query(
        &self,
        request: crate::course_query::FinderRequest,
    ) -> Result<crate::course_query::FinderPage, ReadModelError> {
        let seq = self.store.commit_seq();
        // Only what a page reads besides the catalog; cached, so each
        // keystroke reads the same view.
        let fields =
            workspace_changes::loaded_fields(crate::course_query::FINDER_READS.iter().copied());
        let (workspace, _, content_revision) =
            self.store.view(&fields).map_err(WorkspaceError::from)?;
        let options = self
            .store
            .kyomu_search_options(request.year)
            .map_err(crate::catalog::CatalogError::from)?;
        let mut page = self.finder.page(
            &self.store,
            &workspace,
            &content_revision,
            &options,
            request,
        )?;
        page.seq = seq;
        Ok(page)
    }

    pub async fn reminder_tick(
        &self,
        sink: Arc<dyn crate::schedule::NotificationSink>,
        now: chrono::DateTime<chrono::FixedOffset>,
    ) -> Result<bool, crate::schedule::ReminderError> {
        self.reminders.tick(sink, now).await
    }

    /// Run every periodic domain service once. Each result is retained so a
    /// failed refresh cannot suppress cloud journaling or reminder delivery.
    pub async fn wake(&self, services: WakeServices) -> WakeReport {
        // A session's pause that ran out lifts; the planner reads again.
        self.sessions.wake();
        let runtime = self.runtime.wake(services.scheduler).await;
        let cloud = services
            .cloud
            .map(|services| self.runtime.wake_cloud(services));
        let reminders = match services.reminders {
            Some(reminders) => Some(self.reminder_tick(reminders.sink, reminders.now).await),
            None => None,
        };
        #[cfg(feature = "agent")]
        let submissions = match self.submissions.get() {
            Some(submissions) => Some(submissions.wake().await),
            None => None,
        };
        WakeReport {
            runtime,
            cloud,
            reminders,
            #[cfg(feature = "agent")]
            submissions,
        }
    }

    pub async fn import_legacy_reminders(
        &self,
        fired: serde_json::Value,
        now: chrono::DateTime<chrono::FixedOffset>,
    ) -> Result<bool, crate::schedule::ReminderError> {
        self.reminders.import_legacy_fired(fired, now).await
    }

    pub async fn shutdown(&self) {
        self.reminders.shutdown().await;
        // Reads stop before the sessions they read with.
        self.sync.shutdown().await;
        if let Ok(browser) = self.school_browser() {
            browser.shutdown().await;
        }
        self.sessions.shutdown().await;
        self.runtime.shutdown().await;
        if let Some(events) = &self.events {
            events.clear();
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialMutationError {
    #[error(transparent)]
    Auth(#[from] SchoolAuthError),
    #[error(transparent)]
    Browser(#[from] SchoolBrowserError),
}

pub struct ReminderWake {
    pub sink: Arc<dyn crate::schedule::NotificationSink>,
    pub now: chrono::DateTime<chrono::FixedOffset>,
}

pub struct WakeServices {
    pub scheduler: SchedulerServices,
    pub cloud: Option<crate::cloud_sync::CloudServices>,
    pub reminders: Option<ReminderWake>,
}

pub struct WakeReport {
    pub runtime: Result<(), RuntimeError>,
    pub cloud: Option<Result<bool, CloudWakeError>>,
    pub reminders: Option<Result<bool, crate::schedule::ReminderError>>,
    /// How many LMS writes whose outcome was unknown this wake settled; the
    /// first wake also recovers what the last run left unsettled.
    #[cfg(feature = "agent")]
    pub submissions: Option<Result<usize, crate::submissions::SubmissionError>>,
}

#[derive(Debug, thiserror::Error)]
pub enum ReadModelError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Schedule(#[from] crate::schedule::ScheduleError),
    #[error(transparent)]
    Finder(#[from] crate::course_query::FinderError),
    #[error(transparent)]
    Catalog(#[from] crate::catalog::CatalogError),
    #[error("{0}")]
    InvalidInput(String),
}

#[derive(serde::Serialize)]
pub struct ScheduleQuerySnapshot {
    pub values: Vec<crate::schedule::WorkspaceQueryResult>,
    /// The changes the values were computed from (`WorkspaceQuery::reads`);
    /// a shell re-runs the queries when an event names one.
    pub reads: Vec<WorkspaceChange>,
    /// `DataStore::commit_seq` read before the values' data.
    pub seq: u64,
}

#[cfg(test)]
#[path = "tests/facade.rs"]
mod tests;
