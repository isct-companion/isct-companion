//! Course storage path primitives shared by notes, materials and assignment
//! workspaces: safe folder components, canonical-root checks, file
//! descriptions and atomic writes. Download policy stays in `course_materials`.
//!
//! A course-files root holds two trees, so each can be opened, cleaned or
//! handed to an editor on its own:
//!
//! ```text
//! <root>/material/<term>/<course>/materials/…            downloads, cleanable
//! <root>/material/<term>/<course>/<section>/notes/…      notes, never cleaned
//! <root>/workspace/<term>/<course>/<assignment>/…        assignment workspaces
//! ```

use serde::Serialize;
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock, PoisonError},
};

pub(crate) const MAX_TEXT_PREVIEW_BYTES: u64 = 1_000_000;

/// Downloaded materials, with the notes beside them.
pub const MATERIAL_TREE: &str = "material";
/// Assignment workspaces.
pub const WORKSPACE_TREE: &str = "workspace";

/// `<root>/material`, with the layout before the trees moved in first.
pub fn material_root(root: &Path) -> PathBuf {
    settle(root);
    root.join(MATERIAL_TREE)
}

/// `<root>/workspace`, with the layout before the trees moved in first.
pub fn workspace_root(root: &Path) -> PathBuf {
    settle(root);
    root.join(WORKSPACE_TREE)
}

/// Moves a root's old layout into the trees the first time this process uses
/// that root. The lock is held while it runs, so nothing writes into a tree
/// before the old files have arrived. A failure is logged and retried at the
/// next start; the files it left stay where they were.
fn settle(root: &Path) {
    static SETTLED: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    let mut settled = SETTLED
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if !settled.insert(root.to_path_buf()) {
        return;
    }
    match migrate_layout(root) {
        Ok(Moved { moved: 0, kept: 0 }) => {}
        Ok(Moved { moved, kept }) => {
            log::info!("course files: moved {moved} entries into the material and workspace trees, kept {kept} in place")
        }
        Err(error) => log::warn!("course files: layout migration stopped: {error}"),
    }
}

/// What a layout migration did: entries renamed into a tree, and entries left
/// in place because their destination already existed.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Moved {
    pub moved: usize,
    pub kept: usize,
}

/// Moves the layout before the two trees into them:
/// `<term>/<course>/workspaces/<name>` to `workspace/<term>/<course>/<name>`,
/// and every other entry of a course (its `materials`, its sections' notes) to
/// `material/<term>/<course>/`. Only top-level folders named like a quarter
/// (`2026-3Q`) are touched, since a chosen root may hold unrelated folders.
/// Renames only, never copies or overwrites, and running it again finishes
/// what an earlier run left.
pub fn migrate_layout(root: &Path) -> Result<Moved, StorageError> {
    let mut moved = Moved::default();
    if !root.is_dir() {
        return Ok(moved);
    }
    for term in plain_directories(root)? {
        let Some(term_name) = term.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if crate::schedule::Term::parse(term_name).is_err() {
            continue;
        }
        for course in plain_directories(&term)? {
            let Some(course_name) = course.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            for entry in fs::read_dir(&course).map_err(storage_error)? {
                let entry = entry.map_err(storage_error)?.path();
                if fs::symlink_metadata(&entry)
                    .map_err(storage_error)?
                    .is_symlink()
                {
                    moved.kept += 1;
                    continue;
                }
                let destination = if entry.file_name() == Some("workspaces".as_ref()) {
                    root.join(WORKSPACE_TREE).join(term_name).join(course_name)
                } else {
                    let name = entry.file_name().unwrap_or_default();
                    root.join(MATERIAL_TREE)
                        .join(term_name)
                        .join(course_name)
                        .join(name)
                };
                move_into(&entry, &destination, &mut moved)?;
            }
            let _ = fs::remove_dir(&course);
        }
        let _ = fs::remove_dir(&term);
    }
    Ok(moved)
}

/// Renames `source` to `destination`; two folders merge entry by entry, and
/// anything else already at the destination keeps the source where it is.
fn move_into(source: &Path, destination: &Path, moved: &mut Moved) -> Result<(), StorageError> {
    let source_is_dir = fs::symlink_metadata(source)
        .map_err(storage_error)?
        .is_dir();
    match fs::symlink_metadata(destination) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(storage_error)?;
            }
            fs::rename(source, destination).map_err(storage_error)?;
            moved.moved += 1;
        }
        Err(error) => return Err(storage_error(error)),
        Ok(existing) if source_is_dir && existing.is_dir() && !existing.is_symlink() => {
            for entry in fs::read_dir(source).map_err(storage_error)? {
                let entry = entry.map_err(storage_error)?.path();
                if fs::symlink_metadata(&entry)
                    .map_err(storage_error)?
                    .is_symlink()
                {
                    moved.kept += 1;
                    continue;
                }
                move_into(
                    &entry,
                    &destination.join(entry.file_name().unwrap_or_default()),
                    moved,
                )?;
            }
            let _ = fs::remove_dir(source);
        }
        Ok(_) => moved.kept += 1,
    }
    Ok(())
}

/// Folders directly under `path`, symbolic links left out.
fn plain_directories(path: &Path) -> Result<Vec<PathBuf>, StorageError> {
    let mut directories = Vec::new();
    for entry in fs::read_dir(path).map_err(storage_error)? {
        let entry = entry.map_err(storage_error)?;
        let kind = entry.file_type().map_err(storage_error)?;
        if kind.is_dir() && !kind.is_symlink() {
            directories.push(entry.path());
        }
    }
    Ok(directories)
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum StorageError {
    #[error("{0}")]
    Input(String),
    #[error("{0}")]
    Storage(String),
    /// The system refused access; the message says how to allow it.
    #[error("{0}")]
    Denied(String),
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalFile {
    pub id: String,
    pub name: String,
    pub section: String,
    pub path: String,
    pub bytes: u64,
    pub cached: bool,
    pub text: Option<String>,
}

/// A safe folder name for a course or term, matching the folders desktop
/// already created (`folderComponent` in `features/lms/files.ts`): every
/// other UTF-16 unit becomes `_`, at most 120 characters.
pub fn folder_name(value: &str) -> String {
    let mut name = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-') {
            name.push(character);
        } else {
            name.extend(std::iter::repeat_n('_', character.len_utf16()));
        }
    }
    name.truncate(120);
    if name.is_empty() || name == "." || name == ".." {
        "course".into()
    } else {
        name
    }
}

pub fn component(value: &str) -> Result<&str, StorageError> {
    if value.is_empty()
        || value.len() > 120
        || value == "."
        || value == ".."
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        return Err(StorageError::Input(
            "Invalid course folder component".into(),
        ));
    }
    Ok(value)
}

pub fn storage_dir(root: &Path, parts: &[&str]) -> Result<PathBuf, StorageError> {
    fs::create_dir_all(root).map_err(storage_error)?;
    let canonical_root = root.canonicalize().map_err(storage_error)?;
    let mut path = canonical_root.clone();
    for name in parts {
        path.push(component(name)?);
        if path.exists() {
            let metadata = fs::symlink_metadata(&path).map_err(storage_error)?;
            if metadata.file_type().is_symlink()
                || !path
                    .canonicalize()
                    .map_err(storage_error)?
                    .starts_with(&canonical_root)
            {
                return Err(StorageError::Storage(
                    "Course folder points outside storage root".into(),
                ));
            }
        }
        fs::create_dir_all(&path).map_err(storage_error)?;
    }
    Ok(path)
}

pub fn describe(
    path: PathBuf,
    id: String,
    section: String,
    cached: bool,
    read_text: bool,
) -> Result<LocalFile, StorageError> {
    if fs::symlink_metadata(&path)
        .map_err(storage_error)?
        .file_type()
        .is_symlink()
    {
        return Err(StorageError::Storage("Cannot read symbolic link".into()));
    }
    let bytes = path.metadata().map_err(storage_error)?.len();
    let text = if read_text && bytes <= MAX_TEXT_PREVIEW_BYTES {
        fs::read_to_string(&path).ok()
    } else {
        None
    };
    let name = if uuid::Uuid::parse_str(&id).is_ok() {
        let mut prefix = [0u8; 512];
        let size = fs::File::open(&path)
            .and_then(|mut file| file.read(&mut prefix))
            .unwrap_or(0);
        String::from_utf8_lossy(&prefix[..size])
            .lines()
            .next()
            .unwrap_or("Note")
            .trim_start_matches('#')
            .trim()
            .chars()
            .take(100)
            .collect()
    } else {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    };
    Ok(LocalFile {
        id,
        name,
        section,
        path: path.to_string_lossy().into(),
        bytes,
        cached,
        text,
    })
}

/// Validate a file selected from a material result before the shell opens it.
pub fn stored_file(root: &Path, raw_path: &str) -> Result<PathBuf, StorageError> {
    let supplied_root = root;
    let root = supplied_root.canonicalize().map_err(storage_error)?;
    let raw = PathBuf::from(raw_path);
    // macOS commonly reports the same temporary path as both `/tmp/...` and
    // `/private/tmp/...`. Derive components from either spelling, then walk
    // them below the canonical root so symlinks are still rejected.
    let relative = raw
        .strip_prefix(supplied_root)
        .or_else(|_| raw.strip_prefix(&root))
        .map_err(|_| outside_storage())?;
    let mut checked = root.clone();
    for component in relative.components() {
        let std::path::Component::Normal(component) = component else {
            return Err(outside_storage());
        };
        checked.push(component);
        if fs::symlink_metadata(&checked)
            .map_err(storage_error)?
            .file_type()
            .is_symlink()
        {
            return Err(outside_storage());
        }
    }
    let path = raw.canonicalize().map_err(storage_error)?;
    if !path.is_file() || !path.starts_with(root) {
        return Err(outside_storage());
    }
    Ok(path)
}

fn outside_storage() -> StorageError {
    StorageError::Storage("File is outside the course folder".into())
}

pub fn regular_file(path: &Path) -> Result<bool, StorageError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(StorageError::Storage("Cannot read symbolic link".into()))
        }
        Ok(metadata) => Ok(metadata.is_file()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(storage_error(error)),
    }
}

fn storage_error(error: impl std::fmt::Display + 'static) -> StorageError {
    if denied(&error) {
        StorageError::Denied(failure_text(error))
    } else {
        StorageError::Storage(error.to_string())
    }
}

/// What to do when the system refuses a write into the course folder. On
/// Windows that is usually Controlled folder access, which guards Documents
/// against apps it does not trust yet.
#[cfg(windows)]
pub const WRITE_DENIED: &str = "Windows blocked writing to the course folder. If Controlled folder access is on (Windows Security > Virus & threat protection > Ransomware protection), allow IsctCompanion there, or choose another course folder in Settings.";
#[cfg(not(windows))]
pub const WRITE_DENIED: &str = "The system blocked writing to the course folder. Check its permissions, or choose another course folder in Settings.";

/// Text for a failed course-folder operation, shared by notes, materials and
/// workspaces. A refused write says how to allow it.
pub(crate) fn failure_text(error: impl std::fmt::Display + 'static) -> String {
    if denied(&error) {
        format!("{WRITE_DENIED} ({error})")
    } else {
        error.to_string()
    }
}

pub(crate) fn denied(error: &(impl std::any::Any + 'static)) -> bool {
    (error as &dyn std::any::Any)
        .downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.kind() == std::io::ErrorKind::PermissionDenied)
}

/// Write `contents` to a 0600 temporary file beside `path`, fsync it, then
/// replace `path`. On Windows an existing target is moved aside first and
/// restored if the replacement fails.
pub fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), StorageError> {
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp).map_err(storage_error)?;
        file.write_all(contents).map_err(storage_error)?;
        file.sync_all().map_err(storage_error)?;
        replace_file(&temp, path)
    })();
    let _ = fs::remove_file(temp);
    result
}

#[cfg(not(windows))]
fn replace_file(source: &Path, target: &Path) -> Result<(), StorageError> {
    fs::rename(source, target).map_err(storage_error)
}

#[cfg(windows)]
fn replace_file(source: &Path, target: &Path) -> Result<(), StorageError> {
    if !target.exists() {
        return fs::rename(source, target).map_err(storage_error);
    }
    let backup = target.with_extension(format!("{}.bak", uuid::Uuid::new_v4()));
    fs::rename(target, &backup).map_err(storage_error)?;
    if let Err(error) = fs::rename(source, target) {
        return match fs::rename(&backup, target) {
            Ok(()) => Err(storage_error(error)),
            Err(rollback) => Err(StorageError::Storage(format!(
                "Could not replace {} ({error}); rollback also failed ({rollback}). The previous file is preserved at {}",
                target.display(),
                backup.display()
            ))),
        };
    }
    let _ = fs::remove_file(backup);
    Ok(())
}
