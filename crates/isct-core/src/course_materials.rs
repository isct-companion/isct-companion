//! Cross-platform course material resolution, download and cache ownership.

use crate::{
    contracts::FolderFile,
    data::DataStore,
    downloads::{Downloader, FetchError, Fetched, ResolvedResource},
    net::SessionCookies,
    school_parser::lms::ResolvedFile,
    storage::{self, component, describe, regular_file, storage_dir, LocalFile, StorageError},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};
use url::Url;

const MAX_AUTO_DOWNLOAD_BYTES: u64 = 1024 * 1024 * 1024;
const DEFAULT_AUTO_DOWNLOAD_BYTES: u64 = 20 * 1024 * 1024;
const MAX_RETENTION_DAYS: u64 = 3650;
/// An orphaned partial download of another process is reclaimed after this
/// long without writes; an active download keeps touching its file.
const STALE_PARTIAL_AGE: Duration = Duration::from_secs(6 * 60 * 60);

/// A course's download folder: `<term>/<course>` under the files directory.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CourseKey {
    pub term: String,
    pub course: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LmsResourceKind {
    File,
    View { module: String, cmid: u64 },
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Material {
    Ready { file: LocalFile },
    Missing,
    TooLarge { bytes: u64 },
    Folder { files: Vec<ResolvedFile> },
    External { url: String },
    Failed { reason: MaterialFailure },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MaterialFailure {
    Login,
    Invalid,
    Network,
    Timeout,
    Cancelled,
    Unresolved,
    Denied,
}

#[derive(Debug, thiserror::Error)]
pub enum MaterialError {
    #[error("{0}")]
    Input(String),
    #[error("{0}")]
    Storage(String),
    /// The course folder refused the write; the message says how to allow it.
    #[error("{0}")]
    Denied(String),
    #[error("Course files are resolved in the desktop app")]
    UnsupportedResolution,
    #[error("Course files are downloaded in the desktop app")]
    UnsupportedDownload,
}

impl From<StorageError> for MaterialError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::Input(message) => Self::Input(message),
            StorageError::Storage(message) => Self::Storage(message),
            StorageError::Denied(message) => Self::Denied(message),
        }
    }
}

#[derive(Clone)]
pub struct CourseMaterials {
    store: DataStore,
    root: PathBuf,
    downloads: Downloader,
    session: Option<Arc<dyn SessionCookies>>,
}

impl CourseMaterials {
    pub fn new(
        store: DataStore,
        root: PathBuf,
        downloads: Downloader,
        session: Option<Arc<dyn SessionCookies>>,
    ) -> Self {
        Self {
            store,
            root,
            downloads,
            session,
        }
    }

    pub async fn open(
        &self,
        context: CourseKey,
        source: String,
        download: bool,
        read_text: bool,
        limit: Option<u64>,
    ) -> Result<Material, MaterialError> {
        self.open_with_priority(context, source, download, read_text, limit, true)
            .await
    }

    /// Prepare a material for the custom preview protocol. Explicit requests
    /// are unbounded and take the foreground download lane.
    pub async fn preview(
        &self,
        context: CourseKey,
        source: String,
        explicit: bool,
        foreground: bool,
    ) -> Result<Material, MaterialError> {
        let limit = if explicit {
            None
        } else {
            let materials = self.clone();
            Some(blocking(move || Ok(materials.automatic_download_limit())).await?)
        };
        self.open_with_priority(context, source, true, false, limit, foreground || explicit)
            .await
    }

    async fn open_with_priority(
        &self,
        context: CourseKey,
        source: String,
        download: bool,
        read_text: bool,
        limit: Option<u64>,
        foreground: bool,
    ) -> Result<Material, MaterialError> {
        component(&context.term)?;
        component(&context.course)?;
        let (source_url, kind) = lms_resource_url(&source)?;
        let url = match kind {
            LmsResourceKind::File => source_url,
            LmsResourceKind::View { module, cmid } => {
                let stored = {
                    let store = self.store.clone();
                    let source = source.clone();
                    blocking(move || Ok(store.lms_material_for_url(&source).ok().flatten())).await?
                };
                if module == "folder" {
                    if let Some(files) = stored_folder_files(stored.as_ref()) {
                        return Ok(Material::Folder { files });
                    }
                }
                let known = stored
                    .as_ref()
                    .and_then(|row| row.get("resourceUrl"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
                if let Some(known) = known {
                    match lms_resource_url(&known) {
                        Ok((url, LmsResourceKind::File)) => url,
                        _ if safe_external_url(&known) => {
                            return Ok(Material::External { url: known });
                        }
                        _ => return Ok(failed(MaterialFailure::Invalid)),
                    }
                } else {
                    let session = self
                        .session
                        .clone()
                        .ok_or(MaterialError::UnsupportedResolution)?;
                    match self
                        .downloads
                        .resolve(session, &source_url, cmid, &module)
                        .await
                    {
                        Ok(ResolvedResource::File { url }) => {
                            let Ok((url_value, LmsResourceKind::File)) = lms_resource_url(&url)
                            else {
                                return Ok(failed(MaterialFailure::Invalid));
                            };
                            self.remember_resource(cmid, &source, &url).await;
                            url_value
                        }
                        Ok(ResolvedResource::External { url }) => {
                            if !safe_external_url(&url) {
                                return Ok(failed(MaterialFailure::Invalid));
                            }
                            self.remember_resource(cmid, &source, &url).await;
                            return Ok(Material::External { url });
                        }
                        Ok(ResolvedResource::Folder { files }) => {
                            if !valid_folder_files(&files) {
                                return Ok(failed(MaterialFailure::Invalid));
                            }
                            let store = self.store.clone();
                            let (source, contract) = (source.clone(), folder_contract(&files));
                            // A cached folder listing is an optimization; failure is not fatal.
                            let _ = blocking(move || {
                                let _ = store.update_lms_folder_files(cmid, &source, &contract);
                                Ok(())
                            })
                            .await;
                            return Ok(Material::Folder { files });
                        }
                        Ok(ResolvedResource::Login) => {
                            return Ok(failed(MaterialFailure::Login));
                        }
                        Ok(ResolvedResource::Unresolved) => {
                            return Ok(failed(MaterialFailure::Unresolved));
                        }
                        Err(error) => return Ok(material_for_fetch_error(error)),
                    }
                }
            }
        };

        // Path checks canonicalize and stat under a user-chosen root (possibly a
        // network or cloud folder); keep them on a blocking worker.
        let (path, present) = {
            let (root, context, url) = (self.root.clone(), context.clone(), url.clone());
            blocking(move || {
                let path = material_path(&root, &context, &url)?;
                let present = regular_file(&path)?;
                Ok((path, present))
            })
            .await?
        };
        if !present {
            if !download {
                return Ok(Material::Missing);
            }
            let session = self
                .session
                .clone()
                .ok_or(MaterialError::UnsupportedDownload)?;
            let limit = bounded_download_limit(limit);
            match self
                .downloads
                .fetch(session, &url, path.clone(), limit, foreground)
                .await
            {
                Ok(Fetched::Saved) => {}
                Ok(Fetched::TooLarge(bytes)) => return Ok(Material::TooLarge { bytes }),
                Err(error) => return Ok(material_for_fetch_error(error)),
            }
            let downloaded = path.clone();
            if !blocking(move || Ok(regular_file(&downloaded)?)).await? {
                return Err(MaterialError::Storage(
                    "Downloaded course material is unavailable".into(),
                ));
            }
        }
        let url = url.to_string();
        let file = blocking(move || {
            let _ = fs::File::options()
                .write(true)
                .open(&path)
                .and_then(|file| file.set_modified(SystemTime::now()));
            Ok(describe(path, url, String::new(), true, read_text)?)
        })
        .await?;
        Ok(Material::Ready { file })
    }

    async fn remember_resource(&self, cmid: u64, source: &str, url: &str) {
        let store = self.store.clone();
        let (source, url) = (source.to_owned(), url.to_owned());
        // A remembered resolution is an optimization; failure is not fatal.
        let _ = blocking(move || {
            let _ = store.update_lms_resource_url(cmid, &source, &url);
            Ok(())
        })
        .await;
    }

    fn automatic_download_limit(&self) -> u64 {
        self.store
            .load_workspace()
            .ok()
            .and_then(|workspace| {
                workspace
                    .pointer("/settings/autoDownloadLimitMb")
                    .and_then(serde_json::Value::as_u64)
            })
            .map(|mb| mb.clamp(1, 1024) * 1024 * 1024)
            .unwrap_or(DEFAULT_AUTO_DOWNLOAD_BYTES)
    }

    /// Remove stale cache files from the current and legacy material layouts,
    /// and partial downloads orphaned by another (crashed or killed) process.
    /// User notes, workspace files, symlinks and this process's partial
    /// downloads are never traversed or removed.
    pub async fn clean(&self, days: u64) -> Result<usize, MaterialError> {
        let root = self.root.clone();
        blocking(move || clean_materials(&root, days)).await
    }
}

/// Filesystem and SQLite work runs on a blocking worker, never on an async
/// executor thread.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, MaterialError> + Send + 'static,
) -> Result<T, MaterialError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| MaterialError::Storage(error.to_string()))?
}

fn failed(reason: MaterialFailure) -> Material {
    Material::Failed { reason }
}

fn material_for_fetch_error(error: FetchError) -> Material {
    failed(match error {
        FetchError::Login => MaterialFailure::Login,
        FetchError::Invalid => MaterialFailure::Invalid,
        FetchError::Network => MaterialFailure::Network,
        FetchError::Timeout => MaterialFailure::Timeout,
        FetchError::Cancelled => MaterialFailure::Cancelled,
        FetchError::WriteDenied => MaterialFailure::Denied,
    })
}

fn bounded_download_limit(limit: Option<u64>) -> Option<u64> {
    limit.map(|bytes| bytes.clamp(1, MAX_AUTO_DOWNLOAD_BYTES))
}

pub fn material_path(root: &Path, course: &CourseKey, url: &Url) -> Result<PathBuf, MaterialError> {
    let path =
        storage_dir(root, &[&course.term, &course.course, "materials"])?.join(material_name(url));
    if path.exists()
        && fs::symlink_metadata(&path)
            .map_err(storage_error)?
            .file_type()
            .is_symlink()
    {
        return Err(MaterialError::Storage("Cannot read symbolic link".into()));
    }
    Ok(path)
}

/// `<file name>-<digest>.<ext>` preserves the desktop cache byte-for-byte.
pub fn material_name(url: &Url) -> String {
    let segment = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .unwrap_or("file");
    let decoded = percent_encoding::percent_decode_str(segment).decode_utf8_lossy();
    let (stem, extension) = match decoded.rsplit_once('.') {
        Some((stem, extension))
            if !stem.is_empty()
                && extension.len() <= 12
                && extension
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric()) =>
        {
            (stem, Some(extension))
        }
        _ => (decoded.as_ref(), None),
    };
    let stem: String = stem
        .chars()
        .map(|character| {
            if character.is_control() || "/\\:".contains(character) {
                '_'
            } else {
                character
            }
        })
        .take(80)
        .collect();
    let digest = format!("{:x}", Sha256::digest(url.as_str()));
    match extension {
        Some(extension) => format!(
            "{}-{}.{}",
            stem.trim(),
            &digest[..8],
            extension.to_ascii_lowercase()
        ),
        None => format!("{}-{}", stem.trim(), &digest[..8]),
    }
}

pub fn lms_resource_url(raw: &str) -> Result<(Url, LmsResourceKind), MaterialError> {
    let url = Url::parse(raw).map_err(|_| MaterialError::Input("Invalid material URL".into()))?;
    if url.scheme() != "https"
        || url.host_str() != Some("lms.s.isct.ac.jp")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(MaterialError::Input(
            "Only allowlisted LMS resource URLs are accepted".into(),
        ));
    }
    let segments = url
        .path_segments()
        .map(Iterator::collect::<Vec<_>>)
        .unwrap_or_default();
    if segments.contains(&"pluginfile.php") {
        return Ok((url, LmsResourceKind::File));
    }
    let [.., "mod", module @ ("resource" | "url" | "folder"), "view.php"] = segments.as_slice()
    else {
        return Err(MaterialError::Input(
            "Only allowlisted LMS resource URLs are accepted".into(),
        ));
    };
    let cmid = url
        .query_pairs()
        .find(|(key, _)| key == "id")
        .and_then(|(_, value)| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| MaterialError::Input("LMS resource URL needs a numeric id".into()))?;
    let module = (*module).to_string();
    Ok((url, LmsResourceKind::View { module, cmid }))
}

pub fn folder_contract(files: &[ResolvedFile]) -> Vec<FolderFile> {
    files
        .iter()
        .map(|file| FolderFile {
            name: file.name.clone(),
            url: file.url.clone(),
        })
        .collect()
}

pub fn stored_folder_files(row: Option<&serde_json::Value>) -> Option<Vec<ResolvedFile>> {
    let files: Vec<ResolvedFile> = serde_json::from_value(row?.get("folderFiles")?.clone()).ok()?;
    (!files.is_empty() && valid_folder_files(&files)).then_some(files)
}

fn valid_folder_files(files: &[ResolvedFile]) -> bool {
    files
        .iter()
        .all(|file| matches!(lms_resource_url(&file.url), Ok((_, LmsResourceKind::File))))
}

pub fn safe_external_url(raw: &str) -> bool {
    Url::parse(raw).ok().is_some_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.port().is_none()
            && url.username().is_empty()
            && url.password().is_none()
    })
}

fn storage_error(error: impl std::fmt::Display + 'static) -> MaterialError {
    if storage::denied(&error) {
        MaterialError::Denied(storage::failure_text(error))
    } else {
        MaterialError::Storage(error.to_string())
    }
}

fn clean_materials(root: &Path, days: u64) -> Result<usize, MaterialError> {
    if !root.exists() {
        return Ok(0);
    }
    let root = root.canonicalize().map_err(storage_error)?;
    let age = Duration::from_secs(days.clamp(1, MAX_RETENTION_DAYS) * 24 * 60 * 60);
    let mut removed = 0;
    for term in child_directories(&root)? {
        let term_name = term
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if matches!(
            term_name,
            "debug" | "notes" | "deleted-notes" | "workspace" | "workspaces"
        ) {
            continue;
        }
        for course in child_directories(&term)? {
            removed += clean_material_directory(&course.join("materials"), age)?;
            for section in child_directories(&course)? {
                if section.file_name().is_some_and(|name| {
                    matches!(
                        name.to_str(),
                        Some("materials" | "notes" | "deleted-notes" | "workspace" | "workspaces")
                    )
                }) {
                    continue;
                }
                removed += clean_material_directory(&section.join("materials"), age)?;
            }
        }
    }
    Ok(removed)
}

fn child_directories(path: &Path) -> Result<Vec<PathBuf>, MaterialError> {
    let mut directories = Vec::new();
    for entry in fs::read_dir(path).map_err(storage_error)? {
        let entry = entry.map_err(storage_error)?;
        let metadata = entry.file_type().map_err(storage_error)?;
        if metadata.is_dir() && !metadata.is_symlink() {
            directories.push(entry.path());
        }
    }
    Ok(directories)
}

fn clean_material_directory(path: &Path, age: Duration) -> Result<usize, MaterialError> {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(0);
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Ok(0);
    }
    let mut removed = 0;
    for entry in fs::read_dir(path).map_err(storage_error)? {
        let entry = entry.map_err(storage_error)?;
        let file_type = entry.file_type().map_err(storage_error)?;
        let path = entry.path();
        if !file_type.is_file() {
            continue;
        }
        let orphaned_partial = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(crate::downloads::partial_download_owner)
            .map(|owner| owner != std::process::id());
        let limit = match orphaned_partial {
            Some(true) => STALE_PARTIAL_AGE,
            Some(false) => continue,
            None if is_material_cache_name(&path) => age,
            None => continue,
        };
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|elapsed| elapsed > limit);
        if stale && fs::remove_file(path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

fn is_material_cache_name(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if name.ends_with(".part") || name.ends_with(".tmp") || name.contains(".part-") {
        return false;
    }
    has_digest_suffix(name)
        || name
            .rsplit_once('.')
            .is_some_and(|(stem, _)| has_digest_suffix(stem))
}

fn has_digest_suffix(value: &str) -> bool {
    value.rsplit_once('-').is_some_and(|(_, digest)| {
        digest.len() == 8
            && digest
                .chars()
                .all(|character| character.is_ascii_hexdigit())
    })
}

#[cfg(test)]
#[path = "tests/course_materials.rs"]
mod tests;
