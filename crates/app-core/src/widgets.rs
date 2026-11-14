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
        let fields = workspace_changes::loaded_fields(widgets::READS.iter().copied());
        let (workspace, school, _) = self.store.view(&fields).map_err(WorkspaceError::from)?;
        Ok(match destination {
            Course { key, date } => {
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
                let row = schedule::array(&workspace, "assignments")
                    .iter()
                    .find(|row| {
                        schedule::visible(row)
                            && schedule::text(row, "id") == Some(&id)
                            && row.get("hidden").and_then(serde_json::Value::as_bool) != Some(true)
                    });
                row.map_or(WidgetRoute::UnavailableAssignment, |row| {
                    let course_id = schedule::text(row, "courseId")
                        .filter(|id| {
                            schedule::array(&workspace, "courses").iter().any(|course| {
                                schedule::visible(course)
                                    && schedule::text(course, "id") == Some(*id)
                            })
                        })
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
    render_boundaries: bool,
    renew: Option<DateTime<Utc>>,
    next_course: Option<DateTime<Utc>>,
    next_assignment: Option<DateTime<Utc>>,
    course_boundaries: Vec<DateTime<Utc>>,
    assignment_boundaries: Vec<DateTime<Utc>>,
    reload_courses: bool,
    reload_assignments: bool,
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
        state.render_boundaries = render_boundaries;
        if render_boundaries {
            // Android may have been stopped at the preceding boundary.
            state.next_course = Some(DateTime::UNIX_EPOCH);
            state.next_assignment = state.next_course;
        }
        drop(state);
        self.changed();
    }

    fn refresh(&self) {
        {
            let mut state = self.state();
            if state.render_boundaries {
                state.reload_courses = true;
                state.reload_assignments = true;
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
            if let Err(error) = port.reload(&[WidgetKind::Courses, WidgetKind::Assignments]) {
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
        let (port, rebuild, course_due, assignment_due) = {
            let mut state = self.state();
            let Some(port) = state.port.clone().filter(|_| !state.paused) else {
                return Ok(());
            };
            let rebuild =
                std::mem::take(&mut state.dirty) || state.renew.is_some_and(|at| at <= now);
            let course_due = state.next_course.is_some_and(|at| at <= now);
            let assignment_due = state.next_assignment.is_some_and(|at| at <= now);
            // Android advances from the same contract on both changed and
            // unchanged content. Apple gets all display entries at publish.
            (port, rebuild, course_due, assignment_due)
        };
        if !rebuild {
            let mut state = self.state();
            if state.paused || generation != engine.generation() {
                return Ok(());
            }
            state.reload_courses |= course_due;
            state.reload_assignments |= assignment_due;
            state.advance_display(now);
            return state.reload(&*port);
        }
        let seq = self.store.commit_seq();
        let started = std::time::Instant::now();
        let mut snapshot = publication(&self.store, now)?;
        let elapsed = started.elapsed();

        let mut state = self.state();
        if state.paused || generation != engine.generation() {
            return Ok(());
        }
        if state.dirty || seq != self.store.commit_seq() {
            state.dirty = true;
            return Ok(());
        }
        // Keep comparison, replacement and reload under the same ownership
        // as clearing. A reset waits here even if it had to stop waiting on
        // older network commits.
        let previous = port
            .current()?
            .and_then(|raw| serde_json::from_str::<PublishedHashes>(&raw).ok())
            .filter(|file| file.schema_version == widgets::SCHEMA_VERSION)
            .map(|file| file.content_hash);
        let courses_changed = previous
            .as_ref()
            .is_none_or(|old| old.courses != snapshot.content_hash.courses);
        let assignments_changed = previous
            .as_ref()
            .is_none_or(|old| old.assignments != snapshot.content_hash.assignments);
        if courses_changed || assignments_changed {
            snapshot.generated_at = Utc::now();
            let json = serde_json::to_string(&snapshot)?;
            port.replace(&json)?;
            let (courses, assignments) = snapshot.metrics();
            log::info!(target: "app_core::widgets",
                "Published {courses} course / {assignments} assignment entries ({} bytes, projection {} ms)",
                json.len(), elapsed.as_millis());
        }
        state.renew = Some(snapshot.renew_at());
        if state.render_boundaries {
            state.course_boundaries = snapshot.display_boundaries(WidgetKind::Courses);
            state.assignment_boundaries = snapshot.display_boundaries(WidgetKind::Assignments);
            state.advance_display(now);
        }
        state.reload_courses |= courses_changed || course_due;
        state.reload_assignments |= assignments_changed || assignment_due;
        state.reload(&*port)
    }
}

impl State {
    fn advance_display(&mut self, now: DateTime<Utc>) {
        self.next_course = self.course_boundaries.iter().copied().find(|at| *at > now);
        self.next_assignment = self
            .assignment_boundaries
            .iter()
            .copied()
            .find(|at| *at > now);
    }

    fn reload(&mut self, port: &dyn WidgetPort) -> Result<(), WidgetError> {
        let reload: Vec<_> = [
            (self.reload_courses, WidgetKind::Courses),
            (self.reload_assignments, WidgetKind::Assignments),
        ]
        .into_iter()
        .filter_map(|(due, kind)| due.then_some(kind))
        .collect();
        if !reload.is_empty() {
            port.reload(&reload)?;
            self.reload_courses = false;
            self.reload_assignments = false;
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
        [state.renew, state.next_course, state.next_assignment]
            .into_iter()
            .flatten()
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
                state.next_course = None;
                state.next_assignment = None;
                crate::diagnostics::report("widgets", &error);
            }
        })
    }
}
