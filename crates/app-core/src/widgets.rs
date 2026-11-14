//! Main-app widget publishing, serialized with reset and owned by PlanEngine.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::{
    data::DataStore,
    facade::ReadModelError,
    net::BoxFuture,
    plan::{BackgroundWork, Duty, PlanHints},
    runtime_tasks::RuntimeTaskError,
    schedule::{
        self,
        widgets::{self, WidgetDestination, WidgetHashes, WidgetKind, WidgetPublication},
        ScheduleError,
    },
    workspace::WorkspaceError,
    workspace_changes, Core,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

#[derive(Debug, thiserror::Error)]
pub enum WidgetError {
    #[error(transparent)]
    Schedule(#[from] ScheduleError),
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Runtime(#[from] RuntimeTaskError),
    #[error("Widget storage: {0}")]
    Storage(String),
}

/// Implemented only by the main app. The platform performs the atomic
/// replacement and requests rendering for the kinds Core names.
pub trait WidgetPort: Send + Sync + 'static {
    fn current(&self) -> Result<Option<String>, WidgetError>;
    fn replace(&self, json: &str) -> Result<(), WidgetError>;
    fn clear(&self) -> Result<(), WidgetError>;
    fn reload(&self, kinds: &[WidgetKind]) -> Result<(), WidgetError>;
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum WidgetRoute {
    Course {
        course_id: String,
    },
    Exam {
        key: String,
        term: String,
        date: String,
    },
    Assignment {
        id: String,
        course_id: Option<String>,
    },
    Timetable,
    Assignments,
    Sync,
    UnavailableCourse,
    UnavailableAssignment,
}

impl Core {
    pub fn attach_widgets(&self, port: Arc<dyn WidgetPort>, render_boundaries: bool) {
        self.widgets.attach(port, render_boundaries);
    }

    pub fn refresh_widgets(&self) {
        self.widgets.refresh();
    }

    pub fn widget_publication(&self, now: DateTime<Utc>) -> Result<WidgetPublication, WidgetError> {
        publication(&self.store, now)
    }

    pub fn widget_route(
        &self,
        destination: WidgetDestination,
    ) -> Result<WidgetRoute, ReadModelError> {
        use WidgetDestination::*;
        let view = || {
            let fields = workspace_changes::loaded_fields(widgets::READS.iter().copied());
            self.store.view(&fields).map_err(WorkspaceError::from)
        };
        Ok(match destination {
            Course { key, date } => {
                let (workspace, school, _) = view()?;
                let days = schedule::calendar_between(&workspace, &school, &date, &date)?;
                let item = days
                    .into_iter()
                    .flat_map(|day| day.occurrences)
                    .find(|row| row.key == key && row.cancelled != Some(true));
                match item {
                    Some(row) if row.kind == schedule::OccurrenceKind::Exam => {
                        row.term_id.map_or(WidgetRoute::UnavailableCourse, |term| {
                            WidgetRoute::Exam {
                                key: row.key,
                                term,
                                date,
                            }
                        })
                    }
                    Some(row) if row.kind == schedule::OccurrenceKind::Course => row
                        .course_id
                        .map_or(WidgetRoute::UnavailableCourse, |course_id| {
                            WidgetRoute::Course { course_id }
                        }),
                    _ => WidgetRoute::UnavailableCourse,
                }
            }
            Assignment { id } => {
                let (workspace, _, _) = view()?;
                let row = schedule::visible_by_id(&workspace, "assignments", &id)
                    .filter(|row| row["hidden"].as_bool() != Some(true));
                row.map_or(WidgetRoute::UnavailableAssignment, |row| {
                    let course_id = schedule::text(row, "courseId")
                        .filter(|id| schedule::visible_by_id(&workspace, "courses", id).is_some())
                        .map(str::to_owned);
                    WidgetRoute::Assignment { id, course_id }
                })
            }
            Timetable => WidgetRoute::Timetable,
            Assignments => WidgetRoute::Assignments,
            Sync => WidgetRoute::Sync,
        })
    }
}

fn publication(store: &DataStore, now: DateTime<Utc>) -> Result<WidgetPublication, WidgetError> {
    let fields = workspace_changes::loaded_fields(widgets::READS.iter().copied());
    let (workspace, school, _) = store.view(&fields).map_err(WorkspaceError::from)?;
    widgets::project(&workspace, &school, now)
}

#[derive(Default)]
struct State {
    port: Option<Arc<dyn WidgetPort>>,
    dirty: bool,
    paused: bool,
    renew: Option<DateTime<Utc>>,
    display: Option<Display>,
    reload: [bool; 2],
}

#[derive(Default)]
struct Display {
    boundaries: [Vec<DateTime<Utc>>; 2],
    selected_at: DateTime<Utc>,
}

impl Display {
    fn pending(&self) -> [Option<DateTime<Utc>>; 2] {
        self.boundaries
            .each_ref()
            .map(|times| times.iter().copied().find(|&time| time > self.selected_at))
    }
}

pub(crate) struct Publisher {
    store: DataStore,
    plan: PlanHints,
    state: Mutex<State>,
}

impl Publisher {
    pub(crate) fn new(store: DataStore, plan: PlanHints) -> Self {
        Self {
            store,
            plan,
            state: Mutex::default(),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn attach(&self, port: Arc<dyn WidgetPort>, render_boundaries: bool) {
        let mut state = self.state();
        state.port = Some(port);
        state.display = render_boundaries.then(Display::default);
        drop(state);
        self.refresh();
    }

    fn refresh(&self) {
        {
            let mut state = self.state();
            if state.display.is_some() {
                // Android may have been stopped at a display boundary.
                state.reload.fill(true);
            }
        }
        self.changed();
    }

    pub(crate) fn changed(&self) {
        let mut state = self.state();
        if state.port.is_none() || state.dirty {
            return;
        }
        state.dirty = true;
        drop(state);
        self.plan.duties_changed();
    }

    /// Call after closing PlanEngine admission, before any data is cleared.
    pub(crate) fn pause_and_clear(&self) -> Result<(), WidgetError> {
        let mut state = self.state();
        state.paused = true;
        if let Some(port) = &state.port {
            port.clear()?;
            if let Err(error) = port.reload(&WidgetKind::ALL) {
                crate::diagnostics::report("widgets", &error);
            }
        }
        Ok(())
    }

    pub(crate) fn resume(&self) {
        let mut state = self.state();
        state.paused = false;
        state.dirty = true;
        drop(state);
        self.plan.duties_changed();
    }

    fn publish(&self) -> Result<(), WidgetError> {
        let Some(engine) = self.plan.engine() else {
            return Ok(());
        };
        let generation = engine.generation();
        let now = Utc::now();
        let (port, rebuild) = {
            let mut state = self.state();
            let Some(port) = state.port.clone().filter(|_| !state.paused) else {
                return Ok(());
            };
            (
                port,
                std::mem::take(&mut state.dirty) || state.renew.is_some_and(|at| at <= now),
            )
        };
        let seq = self.store.commit_seq();
        let snapshot = if rebuild {
            let started = std::time::Instant::now();
            Some((publication(&self.store, now)?, started.elapsed()))
        } else {
            None
        };

        let mut state = self.state();
        if state.paused || generation != engine.generation() {
            return Ok(());
        }
        // Replacement and reload use the same lock as clearing.
        state.advance_display(now);
        if let Some((mut snapshot, elapsed)) = snapshot {
            if state.dirty || seq != self.store.commit_seq() {
                state.dirty = true;
                return Ok(());
            }
            let previous = port.current()?.as_deref().and_then(PublishedHashes::read);
            let changed = WidgetKind::ALL.map(|kind| {
                previous
                    .as_ref()
                    .is_none_or(|old| old.get(kind) != snapshot.content_hash.get(kind))
            });
            if changed.contains(&true) {
                snapshot.generated_at = Utc::now();
                let json = serde_json::to_string(&snapshot)?;
                port.replace(&json)?;
                let (courses, assignments) = snapshot.metrics();
                log::info!(target: "app_core::widgets",
                    "Published {courses} course / {assignments} assignment entries ({} bytes, projection {} ms)",
                    json.len(), elapsed.as_millis());
                for (pending, changed) in state.reload.iter_mut().zip(changed) {
                    *pending |= changed;
                }
            }
            state.renew = Some(snapshot.renew_at());
            if let Some(display) = &mut state.display {
                display.boundaries = WidgetKind::ALL.map(|kind| snapshot.display_boundaries(kind));
            }
        }
        state.reload(&*port)
    }
}

impl State {
    fn advance_display(&mut self, now: DateTime<Utc>) {
        if let Some(display) = &mut self.display {
            if now < display.selected_at {
                // A clock change can make a consumed boundary upcoming again.
                self.reload.fill(true);
            } else {
                for (pending, time) in self.reload.iter_mut().zip(display.pending()) {
                    *pending |= time.is_some_and(|at| at <= now);
                }
            }
            display.selected_at = now;
        }
    }

    fn reload(&mut self, port: &dyn WidgetPort) -> Result<(), WidgetError> {
        if self.reload.contains(&true) {
            let reload: Vec<_> = WidgetKind::ALL
                .into_iter()
                .zip(self.reload)
                .filter_map(|(kind, pending)| pending.then_some(kind))
                .collect();
            port.reload(&reload)?;
            self.reload.fill(false);
            log::info!(target: "app_core::widgets", "Requested refresh for {reload:?}");
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublishedHashes {
    schema_version: u32,
    content_hash: WidgetHashes,
}

impl PublishedHashes {
    fn read(raw: &str) -> Option<WidgetHashes> {
        let file: Self = serde_json::from_str(raw).ok()?;
        (file.schema_version == widgets::SCHEMA_VERSION).then_some(file.content_hash)
    }
}

impl Duty for Publisher {
    fn name(&self) -> &'static str {
        "widgets"
    }
    fn work(&self) -> BackgroundWork {
        BackgroundWork::Local
    }

    fn next_due(&self) -> Option<DateTime<Utc>> {
        let state = self.state();
        if state.paused || state.port.is_none() {
            return None;
        }
        if state.dirty {
            return Some(DateTime::UNIX_EPOCH);
        }
        state
            .renew
            .into_iter()
            .chain(state.display.iter().flat_map(Display::pending).flatten())
            .min()
    }

    fn run(self: Arc<Self>) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            let service = self.clone();
            let result = tokio::task::spawn_blocking(move || service.publish()).await;
            if let Err(error) = result
                .map_err(RuntimeTaskError::from)
                .map_err(WidgetError::from)
                .and_then(|result| result)
            {
                // Another normal lifecycle or workspace event can retry.
                let mut state = self.state();
                state.renew = None;
                state.advance_display(Utc::now());
                crate::diagnostics::report("widgets", &error);
            }
        })
    }
}

#[cfg(test)]
#[path = "tests/widgets.rs"]
mod tests;
