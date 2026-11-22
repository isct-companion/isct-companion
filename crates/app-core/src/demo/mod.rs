//! The website screenshot scene: a fictional second-year Computer Science
//! student on Thursday 2026-07-23 at 10:20 JST, in 2026 2Q.
//!
//! Only a `demo` build (`yarn desktop:demo` or the iOS `Demo`
//! configuration, the separate debug app `app.isctcompanion.demo`) compiles
//! this. Its clock starts at
//! [`scene_start`] (`crate::clock`), its school accounts read as signed in
//! from [`DemoVault`] while no school request is ever sent, and [`seed`]
//! writes `scene.json` once, through the same ingest functions and intents a
//! real sync and real edits use. Course codes, the calendar and exams are
//! public; everything personal (names, notes, past-exam files) is made up.

use crate::course_materials::{course_folder, material_path};
use crate::data::{DataError, DataStore, WorkspaceChangeSource};
use crate::exam_share::ExamSubjectPage;
use crate::lms_ingest::{apply_lms_assign, apply_lms_course, apply_lms_deadlines, apply_lms_enrolment};
use crate::notes::CourseFolder;
use crate::ports::{SecretError, SecretVault};
use crate::sync_state::{self, Key};
use crate::workspace::{WorkspaceIntent, WorkspaceMutation};
use crate::Core;
use adpt_isct::lms::{LmsAssignStates, LmsCourseContent, LmsDeadlines, LmsEnrolment};
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

/// The moment the scene shows: 2026-07-23 10:20 JST.
pub fn scene_start() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 23, 1, 20, 0)
        .single()
        .unwrap_or_default()
}

/// The search-algorithms slides the scene's lecture 11 opens.
pub(crate) const SAMPLE_PDF: &[u8] = include_bytes!("search-algorithms.pdf");
/// The made-up exam every past-exam document opens, printed from
/// `sample-exam.html` (Chrome: `--headless --print-to-pdf`).
pub(crate) const SAMPLE_EXAM_PDF: &[u8] = include_bytes!("sample-exam.pdf");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Scene {
    offerings: Vec<Value>,
    enrolment: LmsEnrolment,
    contents: Vec<LmsCourseContent>,
    deadlines: LmsDeadlines,
    assign: Vec<LmsAssignStates>,
    intents: Vec<WorkspaceIntent>,
    department_id: String,
    /// Course colours by LMS course id.
    colors: BTreeMap<u64, String>,
    /// Course outlines read again after everything above was seen: what
    /// Today lists as new.
    fresh: Vec<LmsCourseContent>,
    materials: Vec<Material>,
    notes: Vec<Note>,
    pub(crate) past_exams: Vec<ExamSubjectPage>,
}

/// A downloaded course file: the scene's sample PDF at `url`'s local path.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Material {
    lms_course: u64,
    url: url::Url,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Note {
    lms_course: u64,
    /// The LMS section id the note belongs to.
    section: u64,
    text: String,
}

pub(crate) fn scene() -> Result<&'static Scene, String> {
    static SCENE: OnceLock<Result<Scene, String>> = OnceLock::new();
    SCENE
        .get_or_init(|| {
            serde_json::from_str(include_str!("scene.json"))
                .map_err(|error| format!("demo scene.json: {error}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// The school accounts' vault: both accounts saved with fictional
/// credentials, held in memory only. Sign-ins never run (`SchoolSessions`
/// stays offline), so the accounts read as connected without a request.
pub struct DemoVault(Mutex<HashMap<String, String>>);

impl Default for DemoVault {
    fn default() -> Self {
        let accounts = json!({"version": 1, "accounts": {
            "science": {"username": "demo.student", "password": "demo",
                        "otpSecret": "JBSWY3DPEHPK3PXP", "matrix": ""},
            "tokyo-tech": {"username": "25B00000", "password": "demo", "otpSecret": "",
                           "matrix": "A".repeat(70)},
        }});
        Self(Mutex::new(HashMap::from([(
            "school-accounts".to_owned(),
            accounts.to_string(),
        )])))
    }
}

impl SecretVault for DemoVault {
    fn store(&self, account: &str, secret: &str) -> Result<(), SecretError> {
        self.items()?.insert(account.to_owned(), secret.to_owned());
        Ok(())
    }

    fn load(&self, account: &str) -> Result<Option<String>, SecretError> {
        Ok(self.items()?.get(account).cloned())
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        self.items()?.remove(account);
        Ok(())
    }
}

impl DemoVault {
    fn items(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, String>>, SecretError> {
        self.0
            .lock()
            .map_err(|_| SecretError::Access("demo vault poisoned".into()))
    }
}

/// Writes the scene into a store that does not hold it yet; the course
/// files and notes go under `course_root`. A second launch finds the LMS
/// enrolment stored and leaves everything as the last session left it.
pub fn seed(core: &Core, course_root: &Path) -> Result<(), String> {
    let scene = scene()?;
    let store = &core.store;
    let seeded = store
        .write(None, |tx| sync_state::get(tx, &Key::lms_enrolment()))
        .map_err(text)?
        .is_some();
    if seeded {
        return Ok(());
    }
    store.catalog_upsert_offerings(&scene.offerings).map_err(text)?;
    school_pull(store, |tx| {
        let now = crate::clock::now();
        apply_lms_enrolment(tx, &scene.enrolment, now)?;
        for content in &scene.contents {
            apply_lms_course(tx, content, now)?;
        }
        apply_lms_deadlines(tx, &scene.deadlines, now)?;
        for states in &scene.assign {
            apply_lms_assign(tx, states, now)?;
        }
        Ok(())
    })?;
    for intent in &scene.intents {
        mutate(core, intent.clone())?;
    }
    for (lms_course, color) in &scene.colors {
        mutate(
            core,
            WorkspaceIntent::SaveCourse {
                draft: json!({"color": color, "onTimetable": true}),
                term_id: "2026-2Q".into(),
                existing_id: Some(course_id(store, *lms_course)?),
            },
        )?;
    }
    let mut settings = store.load_fields(&["settings"]).map_err(text)?["settings"].clone();
    settings["departmentId"] = scene.department_id.clone().into();
    mutate(core, WorkspaceIntent::ReplaceSettings { settings })?;
    mutate(core, WorkspaceIntent::MarkAllRecentSeen)?;
    school_pull(store, |tx| {
        for content in &scene.fresh {
            apply_lms_course(tx, content, crate::clock::now())?;
        }
        Ok(())
    })?;
    for material in &scene.materials {
        let course = course_id(store, material.lms_course)?;
        let folder = course_folder(store, &course).ok_or("demo course has no folder")?;
        let path = material_path(course_root, &folder, &material.url).map_err(text)?;
        std::fs::write(&path, SAMPLE_PDF).map_err(text)?;
    }
    let notes = core.runtime.notes.at(course_root.to_path_buf());
    for note in &scene.notes {
        let folder = CourseFolder {
            course_id: course_id(store, note.lms_course)?,
            section: format!("section-{}", note.section),
        };
        notes
            .save(store, folder, String::new(), note.text.clone())
            .map_err(text)?;
    }
    Ok(())
}

fn school_pull(
    store: &DataStore,
    write: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<(), crate::lms_ingest::IngestError>,
) -> Result<(), String> {
    store
        .write(Some(WorkspaceChangeSource::SchoolPull), write)
        .map_err(text)
}

fn mutate(core: &Core, intent: WorkspaceIntent) -> Result<(), String> {
    let expected_revision = core.workspace.revision().map_err(text)?;
    core.workspace
        .mutate_commit(WorkspaceMutation {
            expected_revision,
            intent,
        })
        .map(|_| ())
        .map_err(text)
}

/// The internal id of the course the LMS lists as `lms_course`.
fn course_id(store: &DataStore, lms_course: u64) -> Result<String, String> {
    store
        .write(None, |tx| {
            crate::data::course_keys::lookup(tx, &format!("lms:{lms_course}"))
        })
        .map_err(|error: DataError| text(error))?
        .ok_or_else(|| format!("demo LMS course {lms_course} has no course"))
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}
