//! One application service owner, shared by native shells and agent adapters.
#[cfg(feature = "agent")]
use crate::agent::AgentService;
use crate::school_browser::{SchoolBrowser, SchoolBrowserEngine, SchoolBrowserError};
use crate::school_session::{
    SchoolConnectOutcome, SchoolSessionSnapshot, SchoolSessions, WebCookieSeed,
};
use crate::{
    data::{CommitObserver, DataStore, WorkspaceChangeSource},
    plan::{Hint, Network, OpportunityEnd, PlanEngine, PlanHints, SyncSnapshot, WakeDirective},
    ports::EventSink,
    runtime::{CoreRuntime, RuntimeError},
    scheduler::{ScheduledExporter, SchedulerError},
    school_auth::{SchoolAuthError, SchoolAuthProfile, SchoolAuthService, SchoolCredentials},
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
    /// LMS writes, or why the service could not open; the MCP tools share
    /// it. Only the desktop's agents write to the LMS.
    #[cfg(feature = "agent")]
    submissions: crate::agent::Submissions,
    pub workspace: WorkspaceService,
    pub reminders: crate::schedule::ReminderService,
    /// The campus map's reads over the stored Isct-Data file.
    #[cfg(feature = "campus-map")]
    pub campus_map: crate::campus_map::CampusMapService,
    /// When all background work runs: every network read of school and
    /// reference data, and the deadline duties (`crate::deadlines`);
    /// `Core::reset` closes its admission.
    pub(crate) sync: Arc<PlanEngine>,
    /// The desktop's MCP listener as a duty of the engine.
    #[cfg(feature = "agent")]
    agent_access: Arc<crate::deadlines::AgentAccess>,
    /// The school accounts' sign-in and sessions, owned by Core.
    sessions: SchoolSessions,
    school_browser: Mutex<Option<Arc<SchoolBrowserEngine>>>,
    events: Option<Arc<CoreEvents>>,
    pub(crate) reset_lease: crate::reset::ResetLease,
    finder: crate::course_query::FinderCache,
    /// Campus print sends wait here for the one before them
    /// (`Core::campus_print`).
    #[cfg(all(feature = "campus-printer", any(windows, target_os = "android")))]
    pub(crate) campus_print_turn: Arc<tokio::sync::Mutex<()>>,
}

struct CoreEvents {
    sink: Mutex<Option<Arc<dyn EventSink>>>,
    /// Every event passes the relay below, so the session snapshot reaches the
    /// sync planner once for all shells.
    sync: PlanHints,
}

struct NoopEvents;

impl EventSink for NoopEvents {
    fn emit(&self, _event: crate::events::CoreEvent) -> Result<(), crate::ports::EventError> {
        Ok(())
    }
}

impl EventSink for CoreEvents {
    fn emit(&self, event: crate::events::CoreEvent) -> Result<(), crate::ports::EventError> {
        if !matches!(
            event,
            crate::events::CoreEvent::SyncSnapshot(_) | crate::events::CoreEvent::WakeDirective(_)
        ) {
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
/// The store owns this observer, so it holds only weak handles to services
/// that hold the store; a strong one would keep the store, and the data
/// folder's lock, alive after Core is gone.
struct StoreChanges {
    events: Option<StoreEvents>,
    seq: crate::data::CommitSeq,
    runtime: std::sync::Weak<CoreRuntime>,
    /// The deadline services a commit can bring forward.
    reminders: crate::schedule::ReminderChanges,
    reminder_reads: Vec<WorkspaceChange>,
    plan: PlanHints,
    #[cfg(feature = "agent")]
    agent_access: std::sync::Weak<crate::deadlines::AgentAccess>,
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
        if let Some(runtime) = self.runtime.upgrade() {
            if workspace_changes::touches_user_data(tables) {
                runtime.request_cloud_capture();
            }
            if tables.contains("scheduled_tasks") {
                runtime.scheduled_tasks_changed();
            }
        }
        #[cfg(feature = "agent")]
        if tables.contains("settings") {
            if let Some(agent_access) = self.agent_access.upgrade() {
                agent_access.changed();
                self.plan.duties_changed();
            }
        }
        let changes = workspace_changes::changes_for(tables);
        if changes
            .iter()
            .any(|change| self.reminder_reads.contains(change))
        {
            self.reminders.changed();
            self.plan.duties_changed();
        }
        let Some(events) = &self.events else {
            return;
        };
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
        let sync = PlanEngine::new(store.clone(), auth.clone(), Network::Unknown);
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
        let reminders = crate::schedule::ReminderService::new(store.clone());
        #[cfg(feature = "agent")]
        let agent_access = Arc::new(crate::deadlines::AgentAccess::new(store.clone()));
        let sessions = SchoolSessions::new(crate::net::Http::new().ok(), auth.clone());
        sessions.set_events(
            events
                .as_ref()
                .map(|events| events.clone() as Arc<dyn EventSink>)
                .unwrap_or_else(|| Arc::new(NoopEvents)),
        );
        runtime.downloads.attach_sessions(sessions.clone());
        let moodle = Arc::new(crate::net::moodle::MoodleClient::new(
            auth.clone(),
            sessions.credential_signal(),
        ));
        sync.attach_school(crate::plan::SchoolReaders {
            sessions: sessions.clone(),
            moodle: moodle.clone(),
        });
        let sync = Arc::new(sync);
        let plan_hints = sync.hints();
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
            runtime: Arc::downgrade(&runtime),
            reminders: reminders.changes(),
            reminder_reads: crate::schedule::WorkspaceQuery::NotificationPlan {
                now: String::new(),
                horizon: String::new(),
                locale: None,
            }
            .reads(),
            plan: plan_hints.clone(),
            #[cfg(feature = "agent")]
            agent_access: Arc::downgrade(&agent_access),
        }));
        let assignment_workspaces = crate::workspaces::AssignmentWorkspaces::new(
            store.clone(),
            runtime.notes.clone(),
            runtime.downloads.clone(),
        );
        // A ledger that cannot open leaves writes off and reads untouched;
        // the tools answer with the reason.
        #[cfg(feature = "agent")]
        let submissions = Arc::new(
            crate::submissions::SubmissionService::open(
                &store,
                assignment_workspaces.clone(),
                Arc::new(crate::net::moodle::LmsSubmissions::new(
                    moodle,
                    sessions.clone(),
                )),
                plan_hints.clone(),
                runtime.tasks().clone(),
            )
            .map_err(|error| {
                crate::diagnostics::report("submissions", &error);
                error.to_string()
            }),
        );
        #[cfg(not(feature = "agent"))]
        let _ = moodle;
        // Every deadline of Core, run by the one engine.
        sync.register(Arc::new(crate::deadlines::SessionPauses(sessions.clone())));
        sync.register(Arc::new(crate::deadlines::Reminders(reminders.clone())));
        sync.register(Arc::new(crate::deadlines::Scheduled {
            runtime: runtime.clone(),
            store: store.clone(),
        }));
        sync.register(Arc::new(crate::deadlines::Cloud(runtime.clone())));
        sync.register(Arc::new(crate::deadlines::Maintenance::new(
            store.clone(),
            #[cfg(feature = "agent")]
            submissions.as_ref().as_ref().ok().cloned(),
        )));
        #[cfg(feature = "agent")]
        {
            sync.register(agent_access.clone());
            if let Ok(submissions) = submissions.as_ref() {
                sync.register(Arc::new(crate::deadlines::SubmissionChecks(
                    submissions.clone(),
                )));
            }
        }
        Self {
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
            reminders,
            #[cfg(feature = "campus-map")]
            campus_map: crate::campus_map::CampusMapService::new(store.clone()),
            sync,
            #[cfg(feature = "agent")]
            agent_access,
            sessions,
            school_browser: Mutex::new(None),
            store,
            auth,
            runtime,
            events,
            reset_lease: Default::default(),
            finder: Default::default(),
            #[cfg(all(feature = "campus-printer", any(windows, target_os = "android")))]
            campus_print_turn: Default::default(),
        }
    }

    /// A shell fact or intent for the sync planner. Never blocks.
    pub fn sync_hint(&self, hint: Hint) {
        if let Hint::Network { network } = &hint {
            self.sessions.network(*network);
        }
        self.sync.hint(hint);
    }

    /// The shell's ports for the deadline duties, attached once: the
    /// scheduled-export port, local notifications (the desktop's; phones
    /// schedule the plan with their OS) and the cloud providers.
    pub fn attach_ports(&self, ports: PlatformPorts) {
        if let Some(exporter) = ports.exporter {
            self.runtime.attach_exporter(exporter);
        }
        if let Some(notifications) = ports.notifications {
            self.reminders.attach(notifications);
        }
        if let (Some(cloud), Some(sync)) = (ports.cloud, self.runtime.cloud()) {
            sync.attach(cloud);
        }
        self.sync.duties_changed();
    }

    /// The desktop's MCP listener, which Core starts and stops to match the
    /// "Allow local agents" setting.
    #[cfg(feature = "agent")]
    pub fn attach_agent_listener(&self, listener: Arc<dyn crate::deadlines::AgentListener>) {
        self.agent_access.attach(listener);
        self.sync.duties_changed();
    }

    /// When Core next has work, as last published (`wake-directive`).
    pub fn wake_directive(&self) -> WakeDirective {
        self.sync.wake_directive()
    }

    /// Every directive from now on, for a timer in this process.
    pub fn wake_directives(&self) -> tokio::sync::watch::Receiver<WakeDirective> {
        self.sync.wake_directives()
    }

    /// The shell's timer for directive `revision` fired. Never blocks.
    pub fn deadline_reached(&self, revision: u64) {
        self.sync.deadline_reached(revision);
    }

    /// An OS background opportunity (iOS BGTask, Android WorkManager) of at
    /// most `budget`: Core reads what is due (signing in over HTTP when a
    /// session needs it) if `network` allows, runs the local duties, and
    /// waits until all of that is quiet or the budget ends. The budget
    /// covers everything the opportunity starts. Then it ends as
    /// [`Self::end_background_opportunity`] does. Returns whether Core went
    /// quiet in time.
    pub async fn background_opportunity(
        &self,
        id: String,
        budget: std::time::Duration,
        network: bool,
    ) -> bool {
        self.sync.begin_opportunity(id.clone(), network);
        let quiet = self.sync.wait_quiet(budget).await;
        self.end_background_opportunity(&id, OpportunityEnd::Finished);
        quiet
    }

    /// Ends opportunity `id`, also when the OS takes the time back early.
    /// Unless the app came to the front meanwhile, no new read starts, the
    /// reads it started stop (what they leave stays wanted), and a sign-in
    /// under way ends; local commits finish.
    pub fn end_background_opportunity(&self, id: &str, reason: OpportunityEnd) {
        if self.sync.end_opportunity(id, reason) {
            self.sessions.suspend();
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

    #[cfg(feature = "agent")]
    pub fn submissions(&self) -> Option<&crate::submissions::SubmissionService> {
        self.submissions.as_ref().as_ref().ok()
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
        Ok(self.sessions.connect(profile, credentials).await?)
    }

    /// The person's Retry: one sign-in with the credentials saved now, read
    /// again first.
    pub async fn retry_school(&self, profile: SchoolAuthProfile) -> SchoolSessionSnapshot {
        self.sessions.retry(profile).await
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
        Ok(self.sessions.sign_out(profile).await?)
    }

    /// Runs one of the person's scheduled tasks now.
    pub async fn run_scheduled(
        &self,
        id: String,
    ) -> Result<Result<String, SchedulerError>, RuntimeError> {
        self.runtime.run_scheduled(&self.store, id).await
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

    pub async fn import_legacy_reminders(
        &self,
        fired: serde_json::Value,
        now: chrono::DateTime<chrono::FixedOffset>,
    ) -> Result<bool, crate::schedule::ReminderError> {
        self.reminders.import_legacy_fired(fired, now).await
    }

    pub async fn shutdown(&self) {
        #[cfg(feature = "agent")]
        self.agent_access.detach();
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

/// The shell's ports for Core's deadline duties (`Core::attach_ports`).
#[derive(Default)]
pub struct PlatformPorts {
    pub exporter: Option<Arc<dyn ScheduledExporter>>,
    pub notifications: Option<Arc<dyn crate::schedule::NotificationSink>>,
    pub cloud: Option<crate::cloud_sync::CloudServices>,
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
