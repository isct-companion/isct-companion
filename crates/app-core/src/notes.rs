//! Local course notes. They stay in the course folder: no backup or cloud
//! copy carries them (slotledger constraint/cloud-sync-scope).

use crate::{
    course_materials::{course_folder, CourseKey},
    data::DataStore,
    storage::{self, component, describe, storage_dir, LocalFile, StorageError},
};
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

const MAX_NOTE_BYTES: u64 = 1_000_000;
const MAX_LISTED_NOTES: usize = 1_000;

/// Where a note lives: a section of a course, named by the course's id. Core
/// finds the course's folder (`course_folder`).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CourseFolder {
    pub course_id: String,
    pub section: String,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum NoteError {
    #[error("{0}")]
    Input(String),
    #[error("{0}")]
    Storage(String),
    #[error("Storage lock unavailable")]
    Lock,
    #[error("Notes are limited to 1 MB")]
    TooLarge,
}

/// One process-wide note store shared across storage-root selections.
#[derive(Clone, Default)]
pub struct NoteStore(Arc<NoteStoreInner>);

#[derive(Default)]
struct NoteStoreInner {
    /// Serializes filesystem operations and root switches.
    gate: Mutex<()>,
    /// Short-lived metadata lock; never held during filesystem I/O.
    selected: Mutex<Option<PathBuf>>,
}

impl NoteStore {
    pub fn at(&self, root: PathBuf) -> Notes {
        if let Ok(mut selected) = self.0.selected.lock() {
            if selected.is_none() {
                *selected = Some(root.clone());
            }
        }
        Notes {
            root,
            store: self.clone(),
        }
    }

    /// The storage root shells selected last, which every course-file
    /// service uses; `None` before any shell reported one.
    pub fn selected_root(&self) -> Option<PathBuf> {
        self.0
            .selected
            .lock()
            .ok()
            .and_then(|selected| selected.clone())
    }

    /// Persist a new storage selection while note operations are stopped, then
    /// publish it to future handles. The callback must not call back into notes.
    pub fn select_root<E>(
        &self,
        root: PathBuf,
        persist: impl FnOnce() -> Result<(), E>,
    ) -> Result<(), NoteError>
    where
        E: std::fmt::Display + 'static,
    {
        let _gate = self.0.gate.lock().map_err(|_| NoteError::Lock)?;
        persist().map_err(storage_error)?;
        let mut selected = self.0.selected.lock().map_err(|_| NoteError::Lock)?;
        *selected = Some(root);
        Ok(())
    }
}

#[derive(Clone)]
pub struct Notes {
    root: PathBuf,
    store: NoteStore,
}

impl Notes {
    fn lock_selected(&self) -> Result<MutexGuard<'_, ()>, NoteError> {
        let gate = self.store.0.gate.lock().map_err(|_| NoteError::Lock)?;
        let selected = self.store.0.selected.lock().map_err(|_| NoteError::Lock)?;
        if selected.as_ref() != Some(&self.root) {
            return Err(NoteError::Storage(
                "Course storage changed; retry with the current folder".into(),
            ));
        }
        drop(selected);
        Ok(gate)
    }

    pub fn save(
        &self,
        data: &DataStore,
        context: CourseFolder,
        id: String,
        text: String,
    ) -> Result<LocalFile, NoteError> {
        if text.len() as u64 > MAX_NOTE_BYTES {
            return Err(NoteError::TooLarge);
        }
        let id = if id.is_empty() {
            uuid::Uuid::now_v7().to_string()
        } else {
            id
        };
        let course = course(data, &context.course_id)?;
        let _guard = self.lock_selected()?;
        let path = self.note_path(&course, &context.section, &id)?;
        write_note(&path, text.as_bytes())?;
        describe(path, id, context.section, false, false).map_err(NoteError::from)
    }

    pub fn read(
        &self,
        data: &DataStore,
        context: CourseFolder,
        id: String,
    ) -> Result<LocalFile, NoteError> {
        let course = course(data, &context.course_id)?;
        let _guard = self.lock_selected()?;
        describe(
            self.note_path(&course, &context.section, &id)?,
            id,
            context.section,
            false,
            true,
        )
        .map_err(NoteError::from)
    }

    pub fn move_note(
        &self,
        data: &DataStore,
        context: CourseFolder,
        id: String,
        restore: bool,
    ) -> Result<bool, NoteError> {
        let course = course(data, &context.course_id)?;
        let _guard = self.lock_selected()?;
        let note = self.note_path(&course, &context.section, &id)?;
        let trash = self
            .folder(&course, &context.section, "deleted-notes")?
            .join(format!("{id}.md"));
        reject_symlink(&trash, "Cannot move symbolic link")?;
        let (source, target) = if restore {
            (trash, note)
        } else {
            (note, trash)
        };
        if target.exists() {
            return Err(NoteError::Input(
                "A note already exists at the destination".into(),
            ));
        }
        fs::rename(&source, &target).map_err(storage_error)?;
        fs::File::options()
            .write(true)
            .open(&target)
            .and_then(|file| {
                file.set_times(fs::FileTimes::new().set_modified(std::time::SystemTime::now()))
            })
            .map_err(storage_error)?;
        Ok(true)
    }

    /// The notes of the course `course_id` in `sections`.
    pub fn list(
        &self,
        data: &DataStore,
        course_id: &str,
        sections: Vec<String>,
    ) -> Result<Vec<LocalFile>, NoteError> {
        let context = course(data, course_id)?;
        let _guard = self.lock_selected()?;
        let mut notes = Vec::new();
        let tree = self.tree();
        let canonical_root = canonical_root_if_present(&tree)?;
        for section in sections.into_iter().take(300) {
            component(&context.term)?;
            component(&context.course)?;
            component(&section)?;
            let dir = tree
                .join(&context.term)
                .join(&context.course)
                .join(&section)
                .join("notes");
            if !dir.is_dir() {
                continue;
            }
            ensure_directory(&dir, canonical_root.as_deref())?;
            for entry in fs::read_dir(dir).map_err(storage_error)?.take(500) {
                let path = entry.map_err(storage_error)?.path();
                if path.is_symlink()
                    || path.extension().and_then(|extension| extension.to_str()) != Some("md")
                {
                    continue;
                }
                let id = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                if uuid::Uuid::parse_str(&id).is_err() {
                    continue;
                }
                notes.push(
                    describe(path, id, section.clone(), false, false).map_err(NoteError::from)?,
                );
                if notes.len() >= MAX_LISTED_NOTES {
                    return Ok(notes);
                }
            }
        }
        Ok(notes)
    }

    /// Notes live in the material tree, beside the course's downloads.
    fn tree(&self) -> PathBuf {
        crate::storage::material_root(&self.root)
    }

    fn folder(&self, course: &CourseKey, section: &str, kind: &str) -> Result<PathBuf, NoteError> {
        storage_dir(&self.tree(), &[&course.term, &course.course, section, kind])
            .map_err(NoteError::from)
    }

    fn note_path(&self, course: &CourseKey, section: &str, id: &str) -> Result<PathBuf, NoteError> {
        uuid::Uuid::parse_str(id).map_err(|_| NoteError::Input("Invalid note ID".into()))?;
        let path = self
            .folder(course, section, "notes")?
            .join(format!("{id}.md"));
        reject_symlink(&path, "Cannot edit symbolic link")?;
        Ok(path)
    }
}

/// The folder of the course a note belongs to.
fn course(data: &DataStore, id: &str) -> Result<CourseKey, NoteError> {
    course_folder(data, id).ok_or_else(|| NoteError::Input("Unknown course".into()))
}

impl From<StorageError> for NoteError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::Input(message) => Self::Input(message),
            StorageError::Storage(message) | StorageError::Denied(message) => {
                Self::Storage(message)
            }
        }
    }
}

fn write_note(path: &Path, contents: &[u8]) -> Result<(), NoteError> {
    Ok(storage::durable_replace(path, contents)?)
}

fn reject_symlink(path: &Path, message: &str) -> Result<(), NoteError> {
    if path.is_symlink() {
        Err(NoteError::Storage(message.into()))
    } else {
        Ok(())
    }
}

fn canonical_root_if_present(root: &Path) -> Result<Option<PathBuf>, NoteError> {
    if root.exists() {
        root.canonicalize().map(Some).map_err(storage_error)
    } else {
        Ok(None)
    }
}

fn ensure_directory(path: &Path, root: Option<&Path>) -> Result<(), NoteError> {
    if path.is_symlink() {
        return Err(NoteError::Storage(
            "Notes folder points outside storage root".into(),
        ));
    }
    if let Some(root) = root {
        if !path
            .canonicalize()
            .map_err(storage_error)?
            .starts_with(root)
        {
            return Err(NoteError::Storage(
                "Notes folder points outside storage root".into(),
            ));
        }
    }
    Ok(())
}

fn storage_error(error: impl std::fmt::Display + 'static) -> NoteError {
    NoteError::Storage(storage::failure_text(error))
}

#[cfg(test)]
#[path = "tests/notes.rs"]
mod tests;
