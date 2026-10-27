//! Versioned local course notes and their backup representation.

use crate::{
    data::cloud_merge,
    storage::{self, component, describe, storage_dir, LocalFile, StorageError},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

const MAX_NOTE_BYTES: u64 = 1_000_000;
const MAX_BACKUP_NOTES: usize = 1_000;
const MAX_BACKUP_NOTE_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CourseFolder {
    pub term: String,
    pub course: String,
    pub section: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalNoteBackup {
    pub id: String,
    pub term: String,
    pub course: String,
    pub section: String,
    pub text: String,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub updated_by: String,
    #[serde(default)]
    pub rev: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
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
    /// Told after a note is saved or moved; Core uses it to schedule cloud
    /// capture.
    changed: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>>,
}

impl NoteStore {
    pub(crate) fn on_change(&self, changed: impl Fn() + Send + Sync + 'static) {
        let _ = self.0.changed.set(Box::new(changed));
    }

    fn changed(&self) {
        if let Some(changed) = self.0.changed.get() {
            changed();
        }
    }

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
        context: CourseFolder,
        id: String,
        text: String,
        device_id: &str,
    ) -> Result<LocalFile, NoteError> {
        if text.len() as u64 > MAX_NOTE_BYTES {
            return Err(NoteError::TooLarge);
        }
        let id = if id.is_empty() {
            uuid::Uuid::now_v7().to_string()
        } else {
            id
        };
        let _guard = self.lock_selected()?;
        let path = self.note_path(&context, &id)?;
        write_note(&path, text.as_bytes())?;
        stamp_note(&path, device_id)?;
        self.store.changed();
        describe(path, id, context.section, false, false).map_err(NoteError::from)
    }

    pub fn read(&self, context: CourseFolder, id: String) -> Result<LocalFile, NoteError> {
        let _guard = self.lock_selected()?;
        describe(
            self.note_path(&context, &id)?,
            id,
            context.section,
            false,
            true,
        )
        .map_err(NoteError::from)
    }

    pub fn move_note(
        &self,
        context: CourseFolder,
        id: String,
        restore: bool,
        device_id: &str,
    ) -> Result<bool, NoteError> {
        let _guard = self.lock_selected()?;
        let note = self.note_path(&context, &id)?;
        let trash = self
            .folder(&context, "deleted-notes")?
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
        let source_meta = source.with_extension("sync.json");
        let target_meta = target.with_extension("sync.json");
        let _ = fs::rename(&source_meta, &target_meta);
        fs::File::options()
            .write(true)
            .open(&target)
            .and_then(|file| {
                file.set_times(fs::FileTimes::new().set_modified(std::time::SystemTime::now()))
            })
            .map_err(storage_error)?;
        stamp_note(&target, device_id)?;
        let _ = fs::remove_file(source_meta);
        self.store.changed();
        Ok(true)
    }

    pub fn list(
        &self,
        context: CourseFolder,
        sections: Vec<String>,
    ) -> Result<Vec<LocalFile>, NoteError> {
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
                if notes.len() >= MAX_BACKUP_NOTES {
                    return Ok(notes);
                }
            }
        }
        Ok(notes)
    }

    pub fn export(&self) -> Result<Vec<LocalNoteBackup>, NoteError> {
        let _guard = self.lock_selected()?;
        self.export_unlocked()
    }

    pub fn restore(&self, notes: &[LocalNoteBackup]) -> Result<(), NoteError> {
        self.write_backup(notes, false)
    }

    pub fn merge(&self, notes: &[LocalNoteBackup]) -> Result<(), NoteError> {
        self.write_backup(notes, true)
    }

    /// Keep a backup snapshot and its eventual note apply under one gate.
    /// The callback may combine this transaction with a DataStore operation.
    pub fn transaction<T, E>(
        &self,
        operation: impl FnOnce(&NotesTransaction<'_>) -> Result<T, E>,
    ) -> Result<T, E>
    where
        E: From<NoteError>,
    {
        let _guard = self.lock_selected().map_err(E::from)?;
        operation(&NotesTransaction { notes: self })
    }

    fn write_backup(&self, notes: &[LocalNoteBackup], merge: bool) -> Result<(), NoteError> {
        validate_notes(notes)?;
        let _guard = self.lock_selected()?;
        self.write_backup_unlocked(notes, merge)
    }

    fn write_backup_unlocked(
        &self,
        notes: &[LocalNoteBackup],
        merge: bool,
    ) -> Result<(), NoteError> {
        let current = self.export_unlocked()?;

        // Write and fsync every incoming replacement before removing records
        // omitted by a full restore. A failed import therefore cannot erase
        // the only copy of an existing note before its replacements are safe.
        'incoming: for note in notes {
            let matching = current.iter().filter(|local| local.id == note.id);
            for local in matching.clone() {
                if merge && backup_order(note, local)?.is_lt() {
                    continue 'incoming;
                }
            }
            if !matching.clone().any(|local| same_backup(note, local)) {
                self.write_backup_note(note)?;
            }
            // IDs are global note identities. Once the selected copy is safe,
            // remove stale copies left in an earlier course/section or state.
            for local in matching.filter(|local| !same_note_location(note, local)) {
                self.remove_backup_note(local)?;
            }
        }

        if !merge {
            for old in current
                .iter()
                .filter(|old| !notes.iter().any(|note| note.id == old.id))
            {
                self.remove_backup_note(old)?;
            }
        }
        Ok(())
    }

    fn remove_backup_note(&self, note: &LocalNoteBackup) -> Result<(), NoteError> {
        let context = CourseFolder {
            term: note.term.clone(),
            course: note.course.clone(),
            section: note.section.clone(),
        };
        let path = self
            .folder(
                &context,
                if note.deleted {
                    "deleted-notes"
                } else {
                    "notes"
                },
            )?
            .join(format!("{}.md", note.id));
        reject_symlink(&path, "Cannot restore a note through a symbolic link")?;
        if path.exists() {
            fs::remove_file(&path).map_err(storage_error)?;
        }
        let _ = fs::remove_file(path.with_extension("sync.json"));
        Ok(())
    }

    fn write_backup_note(&self, note: &LocalNoteBackup) -> Result<(), NoteError> {
        let context = CourseFolder {
            term: note.term.clone(),
            course: note.course.clone(),
            section: note.section.clone(),
        };
        let path = self
            .folder(
                &context,
                if note.deleted {
                    "deleted-notes"
                } else {
                    "notes"
                },
            )?
            .join(format!("{}.md", note.id));
        reject_symlink(&path, "Cannot restore a note through a symbolic link")?;
        write_note(&path, note.text.as_bytes())?;
        if let Ok(updated_at) = chrono::DateTime::parse_from_rfc3339(&note.updated_at) {
            fs::File::options()
                .write(true)
                .open(&path)
                .and_then(|file| {
                    file.set_times(fs::FileTimes::new().set_modified(updated_at.into()))
                })
                .map_err(storage_error)?;
        }
        write_json_atomic(
            &path.with_extension("sync.json"),
            &json!({"updatedBy":note.updated_by,"rev":note.rev}),
        )?;
        let opposite = self
            .folder(
                &context,
                if note.deleted {
                    "notes"
                } else {
                    "deleted-notes"
                },
            )?
            .join(format!("{}.md", note.id));
        reject_symlink(&opposite, "Cannot restore a note through a symbolic link")?;
        if opposite.exists() {
            fs::remove_file(&opposite).map_err(storage_error)?;
        }
        let _ = fs::remove_file(opposite.with_extension("sync.json"));
        Ok(())
    }

    fn export_unlocked(&self) -> Result<Vec<LocalNoteBackup>, NoteError> {
        let tree = self.tree();
        if !tree.is_dir() {
            return Ok(Vec::new());
        }
        let canonical = tree.canonicalize().map_err(storage_error)?;
        let mut result = Vec::new();
        let mut total = 0_u64;
        for term in fs::read_dir(&tree).map_err(storage_error)? {
            let term = term.map_err(storage_error)?;
            if term.path().is_symlink() || !term.path().is_dir() {
                continue;
            }
            let term_name = term.file_name().to_string_lossy().into_owned();
            component(&term_name)?;
            for course in fs::read_dir(term.path()).map_err(storage_error)? {
                let course = course.map_err(storage_error)?;
                if course.path().is_symlink() || !course.path().is_dir() {
                    continue;
                }
                let course_name = course.file_name().to_string_lossy().into_owned();
                component(&course_name)?;
                for section in fs::read_dir(course.path()).map_err(storage_error)? {
                    let section = section.map_err(storage_error)?;
                    if section.path().is_symlink() || !section.path().is_dir() {
                        continue;
                    }
                    let section_name = section.file_name().to_string_lossy().into_owned();
                    if section_name == "materials" {
                        continue;
                    }
                    component(&section_name)?;
                    for (kind, deleted) in [("notes", false), ("deleted-notes", true)] {
                        let dir = section.path().join(kind);
                        if !dir.is_dir() || dir.is_symlink() {
                            continue;
                        }
                        ensure_directory(&dir, Some(&canonical))?;
                        for entry in fs::read_dir(dir).map_err(storage_error)? {
                            let path = entry.map_err(storage_error)?.path();
                            if path.is_symlink()
                                || path.extension().and_then(|extension| extension.to_str())
                                    != Some("md")
                            {
                                continue;
                            }
                            let id = path
                                .file_stem()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned();
                            uuid::Uuid::parse_str(&id).map_err(|_| {
                                NoteError::Input("Invalid note ID in storage".into())
                            })?;
                            let metadata = path.metadata().map_err(storage_error)?;
                            let bytes = metadata.len();
                            if bytes > MAX_NOTE_BYTES {
                                return Err(NoteError::Input(
                                    "A note exceeds the 1 MB backup limit".into(),
                                ));
                            }
                            total = total.checked_add(bytes).ok_or_else(|| {
                                NoteError::Input("Note backup size overflow".into())
                            })?;
                            if total > MAX_BACKUP_NOTE_BYTES || result.len() >= MAX_BACKUP_NOTES {
                                return Err(NoteError::Input(
                                    "Notes exceed the local backup safety limit".into(),
                                ));
                            }
                            let updated_at = chrono::DateTime::<chrono::Utc>::from(
                                metadata.modified().map_err(storage_error)?,
                            )
                            .to_rfc3339();
                            let meta: serde_json::Value =
                                fs::read(path.with_extension("sync.json"))
                                    .ok()
                                    .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                                    .unwrap_or_default();
                            let text = fs::read_to_string(&path)
                                .map_err(|_| NoteError::Input("A note is not UTF-8 text".into()))?;
                            result.push(LocalNoteBackup {
                                id,
                                term: term_name.clone(),
                                course: course_name.clone(),
                                section: section_name.clone(),
                                text,
                                deleted,
                                deleted_at: deleted.then(|| updated_at.clone()),
                                updated_at,
                                updated_by: meta["updatedBy"]
                                    .as_str()
                                    .unwrap_or("legacy")
                                    .to_string(),
                                rev: meta["rev"].as_u64().unwrap_or(1),
                            });
                        }
                    }
                }
            }
        }
        Ok(result)
    }

    /// Notes live in the material tree, beside the course's downloads.
    fn tree(&self) -> PathBuf {
        crate::storage::material_root(&self.root)
    }

    fn folder(&self, context: &CourseFolder, kind: &str) -> Result<PathBuf, NoteError> {
        storage_dir(
            &self.tree(),
            &[&context.term, &context.course, &context.section, kind],
        )
        .map_err(NoteError::from)
    }

    fn note_path(&self, context: &CourseFolder, id: &str) -> Result<PathBuf, NoteError> {
        uuid::Uuid::parse_str(id).map_err(|_| NoteError::Input("Invalid note ID".into()))?;
        let path = self.folder(context, "notes")?.join(format!("{id}.md"));
        reject_symlink(&path, "Cannot edit symbolic link")?;
        Ok(path)
    }
}

pub struct NotesTransaction<'a> {
    notes: &'a Notes,
}

impl NotesTransaction<'_> {
    pub fn export(&self) -> Result<Vec<LocalNoteBackup>, NoteError> {
        self.notes.export_unlocked()
    }

    pub fn restore(&self, notes: &[LocalNoteBackup]) -> Result<(), NoteError> {
        validate_notes(notes)?;
        self.notes.write_backup_unlocked(notes, false)
    }

    pub fn merge(&self, notes: &[LocalNoteBackup]) -> Result<(), NoteError> {
        validate_notes(notes)?;
        self.notes.write_backup_unlocked(notes, true)
    }
}

pub fn validate_notes(notes: &[LocalNoteBackup]) -> Result<(), NoteError> {
    if notes.len() > MAX_BACKUP_NOTES {
        return Err(NoteError::Input("Too many notes in backup".into()));
    }
    let mut total = 0_usize;
    for note in notes {
        uuid::Uuid::parse_str(&note.id)
            .map_err(|_| NoteError::Input("Invalid note ID in backup".into()))?;
        component(&note.term)?;
        component(&note.course)?;
        component(&note.section)?;
        if note.text.len() as u64 > MAX_NOTE_BYTES {
            return Err(NoteError::Input("A note exceeds the 1 MB limit".into()));
        }
        total = total
            .checked_add(note.text.len())
            .ok_or_else(|| NoteError::Input("Note backup size overflow".into()))?;
        if total as u64 > MAX_BACKUP_NOTE_BYTES {
            return Err(NoteError::Input(
                "Notes exceed the local backup safety limit".into(),
            ));
        }
    }
    Ok(())
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

fn same_backup(left: &LocalNoteBackup, right: &LocalNoteBackup) -> bool {
    left == right
}

fn same_note_location(left: &LocalNoteBackup, right: &LocalNoteBackup) -> bool {
    left.term == right.term
        && left.course == right.course
        && left.section == right.section
        && left.deleted == right.deleted
}

fn backup_order(
    left: &LocalNoteBackup,
    right: &LocalNoteBackup,
) -> Result<std::cmp::Ordering, NoteError> {
    let left = serde_json::to_value(left).map_err(storage_error)?;
    let right = serde_json::to_value(right).map_err(storage_error)?;
    Ok(cloud_merge::compare(&left, &right))
}

fn write_note(path: &Path, contents: &[u8]) -> Result<(), NoteError> {
    atomic_write(path, contents)
}

pub(crate) fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), NoteError> {
    Ok(storage::atomic_write(path, contents)?)
}

fn write_json_atomic(path: &Path, value: &serde_json::Value) -> Result<(), NoteError> {
    let bytes = serde_json::to_vec(value).map_err(storage_error)?;
    atomic_write(path, &bytes)
}

fn stamp_note(path: &Path, device_id: &str) -> Result<(), NoteError> {
    let old: serde_json::Value = fs::read(path.with_extension("sync.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    write_json_atomic(
        &path.with_extension("sync.json"),
        &json!({
            "updatedBy": device_id,
            "rev": old["rev"].as_u64().unwrap_or(0).saturating_add(1)
        }),
    )
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
