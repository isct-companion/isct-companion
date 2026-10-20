//! What the school connection is doing, as one read model for every shell.
//!
//! The rules come from the desktop footer (`usePullState.ts`), so desktop and
//! the native shells read the same answer: which pull to show, how far it has
//! got, why it waits, what went wrong and when each source last finished.
//! Core gives kinds and numbers; each shell owns the words.
use crate::{
    events::CoreEvent,
    lifecycle::SchoolSite,
    pull_coordinator::{PullPriority, PullSnapshot, PullState, PullTarget},
    school_auth::SchoolAuthProfile,
    school_browser::{SchoolLoginEvent, SchoolPullProgress},
    school_service::{IngestOutcome, IngestState},
    sync_state::{Dataset, KeyState},
};
use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

/// The whole status: one headline for the footer or the sync ring, and one
/// row per source (LMS, then 教務Web) for the connection sheet.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SchoolStatus {
    pub headline: SchoolHeadline,
    pub sources: Vec<SchoolSourceStatus>,
}

/// The one line a shell shows. A pull the person started is `Busy`; Core's
/// own automatic pulls stay `Background`; `Problem` only shows once nothing
/// is running.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SchoolHeadline {
    Idle,
    Busy {
        #[cfg_attr(feature = "ts", ts(type = "SchoolSource"))]
        site: SchoolSite,
        /// 0–100, never moving backwards within one pull.
        progress: u8,
        step: PullStep,
        /// Set while the shown pull is admitted but not running yet.
        waiting: Option<WaitReason>,
        /// The other source's manual pull, queued behind this one.
        #[cfg_attr(feature = "ts", ts(type = "SchoolSource | null"))]
        queued: Option<SchoolSite>,
    },
    Background {
        #[cfg_attr(feature = "ts", ts(type = "SchoolSource"))]
        site: SchoolSite,
        waiting: Option<WaitReason>,
    },
    Problem {
        #[cfg_attr(feature = "ts", ts(type = "SchoolSource"))]
        site: SchoolSite,
        problem: SchoolProblem,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SchoolSourceStatus {
    #[cfg_attr(feature = "ts", ts(type = "SchoolSource"))]
    pub site: SchoolSite,
    pub activity: SourceActivity,
    /// The latest sign-in report for the profile that reads this source.
    pub sign_in: Option<SignInState>,
    /// When Core last finished reading this source (RFC 3339): the newest
    /// complete read of its course list and deadlines, or of any 教務 page.
    pub last_updated: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SourceActivity {
    Idle,
    Waiting {
        reason: WaitReason,
        manual: bool,
    },
    Running {
        manual: bool,
        progress: u8,
        step: PullStep,
    },
    Problem {
        problem: SchoolProblem,
    },
}

/// Why an admitted pull is not running (`pull_coordinator` blockers).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum WaitReason {
    /// Behind another school pull.
    Queued,
    Offline,
    /// Automatic pulls wait for Wi-Fi unless the person allowed cellular.
    Cellular,
    Background,
    /// The person is using the school page the pull would drive.
    SchoolVisible,
    Suspended,
    Other,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PullStep {
    pub kind: PullStepKind,
    /// Items done and expected in this step; both 0 before the first report.
    pub completed: u32,
    pub total: u32,
    pub term: String,
    pub course_title: String,
    pub item_title: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum PullStepKind {
    /// Reading the enrolled-course list; also the step before any report.
    Catalog,
    Course,
    Material,
    Assignment,
    Timetable,
    /// Writing what was read.
    Saving,
    /// Reading 教務Web pages, which report no finer steps.
    Kyomu,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SchoolProblem {
    Maintenance {
        detail: String,
    },
    Failed {
        detail: Option<String>,
    },
    /// The pull finished but some pages could not be read.
    Partial {
        detail: Option<String>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SignInState {
    Connected,
    Step { step: SignInStep },
    NeedsUser { message: String },
    Failed { message: String },
}

/// Steps the sign-in script reports (`school-login.js`).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum SignInStep {
    Agree,
    Identifier,
    Password,
    Method,
    Otp,
    Matrix,
    Other,
}

#[cfg(feature = "ts")]
pub fn declarations() -> Vec<String> {
    vec![
        "type SchoolSource = \"science-lms\" | \"kyomu\";".into(),
        SchoolStatus::decl(),
        SchoolHeadline::decl(),
        SchoolSourceStatus::decl(),
        SourceActivity::decl(),
        WaitReason::decl(),
        PullStep::decl(),
        PullStepKind::decl(),
        SchoolProblem::decl(),
        SignInState::decl(),
        SignInStep::decl(),
    ]
}

const SOURCES: [SchoolSite; 2] = [SchoolSite::ScienceLms, SchoolSite::Kyomu];
const START: f64 = 5.0;

/// Progress and step of the pull currently shown for one source. Both start
/// over when a different pull takes its place.
#[derive(Default)]
struct Lane {
    shown: Option<String>,
    progress: f64,
    step: Option<PullStep>,
}

impl Lane {
    fn show(&mut self, id: Option<&str>) {
        if self.shown.as_deref() != id {
            self.shown = id.map(str::to_owned);
            self.progress = START;
            self.step = None;
        }
    }
}

/// Folds the events Core already emits into [`SchoolStatus`]. The facade's
/// event relay feeds it, so no shell keeps its own copy of these rules.
#[derive(Default)]
pub(crate) struct SchoolStatusTracker {
    pulls: Vec<PullSnapshot>,
    lms: Lane,
    kyomu: Lane,
    kyomu_maintenance: Option<String>,
    science_sign_in: Option<SignInState>,
    tokyo_tech_sign_in: Option<SignInState>,
    /// Stored freshness (`DataStore::freshness`).
    freshness: Vec<KeyState>,
}

impl SchoolStatusTracker {
    /// Takes in one event. Returns whether stored freshness may have changed,
    /// so the owner re-reads it before the next [`Self::status`].
    pub(crate) fn observe(&mut self, event: &CoreEvent) -> bool {
        match event {
            CoreEvent::PullStateChanged(pulls) => self.pulls_changed(pulls),
            CoreEvent::SchoolPullProgress(progress) => {
                self.progress(progress);
                false
            }
            CoreEvent::SchoolIngest(outcome) => {
                self.ingest(outcome);
                true
            }
            CoreEvent::SchoolLogin(event) => {
                self.sign_in(event);
                false
            }
            _ => false,
        }
    }

    pub(crate) fn set_freshness(&mut self, freshness: Vec<KeyState>) {
        self.freshness = freshness;
    }

    fn pulls_changed(&mut self, pulls: &[PullSnapshot]) -> bool {
        // A school pull that just finished committed its keys.
        let finished = pulls.iter().any(|next| {
            school_site(next).is_some()
                && matches!(next.state, PullState::Succeeded | PullState::Failed)
                && !self
                    .pulls
                    .iter()
                    .any(|old| old.id == next.id && old.state == next.state)
        });
        self.pulls = pulls.to_vec();
        let lms = shown(&self.pulls, SchoolSite::ScienceLms).map(|pull| pull.id.clone());
        let kyomu = shown(&self.pulls, SchoolSite::Kyomu).map(|pull| pull.id.clone());
        self.lms.show(lms.as_deref());
        self.kyomu.show(kyomu.as_deref());
        finished
    }

    fn progress(&mut self, progress: &SchoolPullProgress) {
        if progress.site != SchoolSite::ScienceLms {
            return;
        }
        let (kind, start, end) = match progress.phase.as_str() {
            "catalog" => (PullStepKind::Catalog, 5.0, 8.0),
            "course" => (PullStepKind::Course, 8.0, 25.0),
            "material" => (PullStepKind::Material, 25.0, 38.0),
            "assignment" => (PullStepKind::Assignment, 38.0, 44.0),
            "timetable" => (PullStepKind::Timetable, 44.0, 95.0),
            _ => return,
        };
        let ratio = if progress.total == 0 {
            1.0
        } else {
            (progress.completed as f64 / progress.total as f64).min(1.0)
        };
        self.lms.progress = self.lms.progress.max(start + (end - start) * ratio);
        self.lms.step = Some(PullStep {
            kind,
            completed: count(progress.completed),
            total: count(progress.total),
            term: progress.term.clone(),
            course_title: progress.course_title.clone(),
            item_title: progress.item_title.clone(),
        });
    }

    fn ingest(&mut self, outcome: &IngestOutcome) {
        if outcome.site != SchoolSite::Kyomu {
            return;
        }
        match outcome.state {
            IngestState::Maintenance => self.kyomu_maintenance = Some(outcome.message.clone()),
            IngestState::Ok | IngestState::Unavailable => {
                self.kyomu_maintenance = None;
                // Each page read moves 教務Web on; the service probe does not.
                if outcome.kind != "service" {
                    self.kyomu.progress = (self.kyomu.progress.max(15.0) + 20.0).min(95.0);
                }
            }
            IngestState::ReconnectRequired | IngestState::Error => {}
        }
    }

    fn sign_in(&mut self, event: &SchoolLoginEvent) {
        let state = match event.state.as_str() {
            "connected" => SignInState::Connected,
            "progress" => SignInState::Step {
                step: match event.message.as_str() {
                    "agree" => SignInStep::Agree,
                    "identifier" => SignInStep::Identifier,
                    "password" => SignInStep::Password,
                    "method" => SignInStep::Method,
                    "otp" => SignInStep::Otp,
                    "matrix" => SignInStep::Matrix,
                    _ => SignInStep::Other,
                },
            },
            "needs-user" => SignInState::NeedsUser {
                message: event.message.clone(),
            },
            _ => SignInState::Failed {
                message: event.message.clone(),
            },
        };
        match event.profile {
            SchoolAuthProfile::Science => self.science_sign_in = Some(state),
            SchoolAuthProfile::TokyoTech => self.tokyo_tech_sign_in = Some(state),
        }
    }

    pub(crate) fn status(&self) -> SchoolStatus {
        SchoolStatus {
            headline: self.headline(),
            sources: SOURCES.iter().map(|site| self.source(*site)).collect(),
        }
    }

    fn lane(&self, site: SchoolSite) -> &Lane {
        if site == SchoolSite::Kyomu {
            &self.kyomu
        } else {
            &self.lms
        }
    }

    /// What the shown pull is doing. Before LMS reports a step it is reading
    /// the course list; 教務Web never reports finer steps.
    fn step(&self, site: SchoolSite, pull: &PullSnapshot) -> PullStep {
        if pull.state == PullState::Applying {
            return empty_step(PullStepKind::Saving);
        }
        if site == SchoolSite::Kyomu {
            return empty_step(PullStepKind::Kyomu);
        }
        self.lms
            .step
            .clone()
            .unwrap_or_else(|| empty_step(PullStepKind::Catalog))
    }

    fn progress_of(&self, site: SchoolSite) -> u8 {
        self.lane(site).progress.round().clamp(0.0, 100.0) as u8
    }

    fn headline(&self) -> SchoolHeadline {
        let lms = shown(&self.pulls, SchoolSite::ScienceLms);
        let kyomu = shown(&self.pulls, SchoolSite::Kyomu);
        let manual = |pull: Option<&PullSnapshot>| {
            pull.is_some_and(|pull| pull.priority == PullPriority::Manual)
        };
        let busy = |site: SchoolSite, pull: &PullSnapshot| SchoolHeadline::Busy {
            site,
            progress: self.progress_of(site),
            step: self.step(site, pull),
            waiting: waiting(pull),
            queued: None,
        };
        // A running manual pull wins over the order below, naming a manual
        // pull queued behind it rather than hiding it.
        let running = [(SchoolSite::Kyomu, kyomu), (SchoolSite::ScienceLms, lms)]
            .into_iter()
            .find_map(|(site, pull)| {
                pull.filter(|pull| is_running(pull))
                    .map(|pull| (site, pull))
            });
        if let Some((site, pull)) = running.filter(|_| manual(kyomu) || manual(lms)) {
            let queued = [(SchoolSite::Kyomu, kyomu), (SchoolSite::ScienceLms, lms)]
                .into_iter()
                .find(|(_, pull)| {
                    pull.is_some_and(|pull| {
                        pull.priority == PullPriority::Manual
                            && matches!(pull.state, PullState::Queued | PullState::Blocked)
                    })
                })
                .map(|(site, _)| site);
            return SchoolHeadline::Busy {
                site,
                progress: self.progress_of(site),
                step: self.step(site, pull),
                waiting: None,
                queued,
            };
        }
        if lms.is_none() && kyomu.is_none() {
            if let Some(detail) = &self.kyomu_maintenance {
                return SchoolHeadline::Problem {
                    site: SchoolSite::Kyomu,
                    problem: SchoolProblem::Maintenance {
                        detail: detail.clone(),
                    },
                };
            }
            if let Some(failed) = latest_failed(&self.pulls, None) {
                return SchoolHeadline::Problem {
                    site: school_site(failed).unwrap_or(SchoolSite::ScienceLms),
                    problem: failure(failed),
                };
            }
            return SchoolHeadline::Idle;
        }
        if let Some(pull) = kyomu.filter(|_| manual(kyomu)) {
            return busy(SchoolSite::Kyomu, pull);
        }
        if let Some(pull) = lms.filter(|_| manual(lms)) {
            return busy(SchoolSite::ScienceLms, pull);
        }
        let (site, pull) = match kyomu {
            Some(pull) => (SchoolSite::Kyomu, pull),
            None => match lms {
                Some(pull) => (SchoolSite::ScienceLms, pull),
                None => return SchoolHeadline::Idle,
            },
        };
        SchoolHeadline::Background {
            site,
            waiting: waiting(pull),
        }
    }

    fn source(&self, site: SchoolSite) -> SchoolSourceStatus {
        let activity = match shown(&self.pulls, site) {
            Some(pull) if is_running(pull) => SourceActivity::Running {
                manual: pull.priority == PullPriority::Manual,
                progress: self.progress_of(site),
                step: self.step(site, pull),
            },
            Some(pull) => SourceActivity::Waiting {
                reason: waiting(pull).unwrap_or(WaitReason::Queued),
                manual: pull.priority == PullPriority::Manual,
            },
            None => match (
                &self.kyomu_maintenance,
                latest_failed(&self.pulls, Some(site)),
            ) {
                (Some(detail), _) if site == SchoolSite::Kyomu => SourceActivity::Problem {
                    problem: SchoolProblem::Maintenance {
                        detail: detail.clone(),
                    },
                },
                (_, Some(failed)) => SourceActivity::Problem {
                    problem: failure(failed),
                },
                _ => SourceActivity::Idle,
            },
        };
        let (sign_in, last_updated) = if site == SchoolSite::Kyomu {
            (
                self.tokyo_tech_sign_in.clone(),
                self.last_read(&[Dataset::KyomuPage]),
            )
        } else {
            (
                self.science_sign_in.clone(),
                self.last_read(&[Dataset::LmsEnrolment, Dataset::LmsDeadlines]),
            )
        };
        SchoolSourceStatus {
            site,
            activity,
            sign_in,
            last_updated,
        }
    }

    fn last_read(&self, datasets: &[Dataset]) -> Option<String> {
        self.freshness
            .iter()
            .filter(|state| {
                datasets
                    .iter()
                    .any(|dataset| dataset.name() == state.dataset)
            })
            .filter_map(|state| state.fetched_at)
            .max()
            .map(|at| at.to_rfc3339())
    }
}

fn school_site(pull: &PullSnapshot) -> Option<SchoolSite> {
    match &pull.target {
        PullTarget::School { site, .. } => Some(*site),
        _ => None,
    }
}

/// The pull a source shows: one that runs, else one that is not blocked,
/// else a blocked one (`schoolPullForDisplay`).
fn shown(pulls: &[PullSnapshot], site: SchoolSite) -> Option<&PullSnapshot> {
    let active = || {
        pulls.iter().filter(move |pull| {
            school_site(pull) == Some(site)
                && !matches!(
                    pull.state,
                    PullState::Succeeded | PullState::Failed | PullState::Cancelled
                )
        })
    };
    active()
        .find(|pull| is_running(pull))
        .or_else(|| active().find(|pull| pull.state != PullState::Blocked))
        .or_else(|| active().next())
}

fn is_running(pull: &PullSnapshot) -> bool {
    matches!(
        pull.state,
        PullState::Running | PullState::Applying | PullState::Cancelling
    )
}

fn waiting(pull: &PullSnapshot) -> Option<WaitReason> {
    match pull.state {
        PullState::Queued => Some(WaitReason::Queued),
        PullState::Blocked => Some(match pull.blocked_by.as_deref() {
            Some("offline") => WaitReason::Offline,
            Some("cellular") => WaitReason::Cellular,
            Some("background") => WaitReason::Background,
            Some("schoolVisible") => WaitReason::SchoolVisible,
            Some("suspended") => WaitReason::Suspended,
            _ => WaitReason::Other,
        }),
        _ => None,
    }
}

/// The newest failure among each target's latest pull. Core keeps pulls in
/// admission order, so a later success for the same target clears it.
fn latest_failed(pulls: &[PullSnapshot], site: Option<SchoolSite>) -> Option<&PullSnapshot> {
    let mut latest: Vec<&PullSnapshot> = Vec::new();
    for pull in pulls {
        let PullTarget::School {
            site: at,
            course_id,
        } = &pull.target
        else {
            continue;
        };
        if site.is_some_and(|site| site != *at) {
            continue;
        }
        latest.retain(|old| {
            !matches!(&old.target, PullTarget::School { site, course_id: id } if site == at && id == course_id)
        });
        latest.push(pull);
    }
    latest
        .into_iter()
        .rev()
        .find(|pull| pull.state == PullState::Failed)
}

fn failure(pull: &PullSnapshot) -> SchoolProblem {
    let detail = pull.last_error.clone();
    if pull.phase.as_deref() == Some("partial") {
        SchoolProblem::Partial { detail }
    } else {
        SchoolProblem::Failed { detail }
    }
}

fn empty_step(kind: PullStepKind) -> PullStep {
    PullStep {
        kind,
        completed: 0,
        total: 0,
        term: String::new(),
        course_title: String::new(),
        item_title: String::new(),
    }
}

fn count(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
#[path = "tests/school_status.rs"]
mod tests;
