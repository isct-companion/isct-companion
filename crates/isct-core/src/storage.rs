//! Course storage path primitives shared by notes, materials and assignment
//! workspaces: safe folder components, canonical-root checks, file
//! descriptions and atomic writes. Download policy stays in `course_materials`.

use serde::Serialize;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub(crate) const MAX_TEXT_PREVIEW_BYTES: u64 = 1_000_000;

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum StorageError {
    #[error("{0}")]
    Input(String),
    #[error("{0}")]
    Storage(String),
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

fn storage_error(error: impl ToString) -> StorageError {
    StorageError::Storage(error.to_string())
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
