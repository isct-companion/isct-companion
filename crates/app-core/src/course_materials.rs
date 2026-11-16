//! Cross-platform course material resolution, download and cache ownership.

use crate::{
    contracts::FolderFile,
    data::DataStore,
    downloads::{Downloader, FetchError, Fetched, ResolvedResource},
    lifecycle::SchoolSite,
    school_browser::{automation_target, browsing_url, SchoolWebTarget},
    storage::{self, describe, regular_file, storage_dir, LocalFile, StorageError},
};
use adpt_isct::lms::ResolvedFile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
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
enum LmsResourceKind {
    File,
    View { module: String, cmid: u64 },
}

/// What opening an LMS item or a link gives: every source has one answer, so
/// shells act on it and never guess from an error what the item was. `page`
/// is the LMS page an LMS source opens on, which a shell offers when it does
/// not show the file itself.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Material {
    Ready {
        file: LocalFile,
        page: SchoolWebTarget,
    },
    /// The file is not in the course folder and was not to be downloaded.
    Missing {
        page: SchoolWebTarget,
    },
    TooLarge {
        bytes: u64,
        page: SchoolWebTarget,
    },
    Folder {
        files: Vec<ResolvedFile>,
        page: SchoolWebTarget,
    },
    /// A school page with an adapter, opened in the App's signed-in school
    /// web view of `site`: an LMS activity that is not a file (forum,
    /// feedback, page, an LTI tool, any module Core does not know), or a link
    /// to another school site.
    Page {
        site: SchoolSite,
        url: String,
    },
    /// A page outside the school sites, for the system browser.
    External {
        url: String,
    },
    /// `page` is absent for a source that is not on the LMS.
    Failed {
        reason: MaterialFailure,
        page: Option<SchoolWebTarget>,
    },
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
}

impl CourseMaterials {
    pub fn new(store: DataStore, root: PathBuf, downloads: Downloader) -> Self {
        Self {
            store,
            root,
            downloads,
        }
    }

    /// What `source` opens as. `course` is the course's id; its folder keeps
    /// the file.
    pub async fn open(
        &self,
        course: String,
        source: String,
        download: bool,
        read_text: bool,
        limit: Option<u64>,
    ) -> Result<Material, MaterialError> {
        self.open_with_priority(course, source, download, read_text, limit, true)
            .await
    }

    /// Prepare a material for the custom preview protocol. Explicit requests
    /// are unbounded and take the foreground download lane.
    pub async fn preview(
        &self,
        course: String,
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
        self.open_with_priority(course, source, true, false, limit, foreground || explicit)
            .await
    }

    async fn open_with_priority(
        &self,
        course: String,
        source: String,
        download: bool,
        read_text: bool,
        limit: Option<u64>,
        foreground: bool,
    ) -> Result<Material, MaterialError> {
        let material = self
            .classify_and_open(course, &source, download, read_text, limit, foreground)
            .await?;
        if let Material::Failed { reason, .. } = &material {
            let parsed = Url::parse(&source).ok();
            let module = parsed.as_ref().map_or("-", module_of);
            log::info!(
                target: "app_core::course_materials",
                "material open failed module={module} reason={reason:?}"
            );
        }
        Ok(material)
    }

    /// The one decision of what an item does when opened. A source Core
    /// cannot read as a web address is corrupt data; an LMS address that is
    /// not a downloadable resource is its LMS page; any other link is a
    /// school page or the system browser (`link`).
    async fn classify_and_open(
        &self,
        course: String,
        source: &str,
        download: bool,
        read_text: bool,
        limit: Option<u64>,
        foreground: bool,
    ) -> Result<Material, MaterialError> {
        let Some(source_url) = browsing_url(source).and_then(|raw| Url::parse(&raw).ok()) else {
            log::error!(
                target: "app_core::course_materials",
                "material source is not a web address ({} bytes)",
                source.len()
            );
            return Ok(failed(MaterialFailure::Invalid, None));
        };
        if !adpt_isct::lms::is_lms_origin(&source_url) {
            return Ok(link(source));
        }
        // An LMS address that is not a download (forum, feedback, page, an LTI
        // tool, a module Core does not know, an address without an id) is
        // that LMS page, without a request.
        let Some((source_url, kind)) = lms_resource_url(source) else {
            return Ok(Material::Page {
                site: SchoolSite::ScienceLms,
                url: source_url.into(),
            });
        };
        // Whatever happens to the file, the item still opens on its LMS page.
        let page = SchoolWebTarget {
            site: SchoolSite::ScienceLms,
            url: source.to_owned(),
        };
        let lms_failed = |reason| failed(reason, Some(page.clone()));
        let source = source.to_owned();
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
                        return Ok(Material::Folder { files, page });
                    }
                }
                let known = stored
                    .as_ref()
                    .and_then(|row| row.get("resourceUrl"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
                if let Some(known) = known {
                    match lms_resource_url(&known) {
                        Some((url, LmsResourceKind::File)) => url,
                        _ => return Ok(link(&known)),
                    }
                } else {
                    match self.downloads.resolve(&source_url, cmid, &module).await {
                        Ok(ResolvedResource::File { url }) => {
                            let Some((url_value, LmsResourceKind::File)) = lms_resource_url(&url)
                            else {
                                return Ok(lms_failed(MaterialFailure::Invalid));
                            };
                            self.remember_resource(cmid, &source, &url).await;
                            url_value
                        }
                        Ok(ResolvedResource::External { url }) => {
                            let material = link(&url);
                            if !matches!(material, Material::Failed { .. }) {
                                self.remember_resource(cmid, &source, &url).await;
                            }
                            return Ok(material);
                        }
                        Ok(ResolvedResource::Folder { files }) => {
                            if !valid_folder_files(&files) {
                                return Ok(lms_failed(MaterialFailure::Invalid));
                            }
                            let store = self.store.clone();
                            let (source, contract) = (source.clone(), folder_contract(&files));
                            // A cached folder listing is an optimization; failure is not fatal.
                            let _ = blocking(move || {
                                let _ = store.update_lms_folder_files(cmid, &source, &contract);
                                Ok(())
                            })
                            .await;
                            return Ok(Material::Folder { files, page });
                        }
                        Ok(ResolvedResource::Login) => {
                            return Ok(lms_failed(MaterialFailure::Login));
                        }
                        Ok(ResolvedResource::Unresolved) => {
                            return Ok(lms_failed(MaterialFailure::Unresolved));
                        }
                        Err(error) => return Ok(lms_failed(failure_of(error))),
                    }
                }
            }
        };

        // The course's folder, as the course page names it; a course that is
        // gone has no folder to keep the file in.
        let context = {
            let store = self.store.clone();
            blocking(move || Ok(course_key(&store, &course))).await?
        };
        let Some(context) = context else {
            return Ok(lms_failed(MaterialFailure::Unresolved));
        };
        // Path checks canonicalize and stat under a user-chosen root (possibly a
        // network or cloud folder); keep them on a blocking worker.
        let (path, present) = {
            let (root, url) = (self.root.clone(), url.clone());
            blocking(move || {
                let path = material_path(&root, &context, &url)?;
                let present = regular_file(&path)?;
                Ok((path, present))
            })
            .await?
        };
        if !present {
            if !download {
                return Ok(Material::Missing { page });
            }
            let limit = bounded_download_limit(limit);
            match self
                .downloads
                .fetch(&url, path.clone(), limit, foreground)
                .await
            {
                Ok(Fetched::Saved) => {}
                Ok(Fetched::TooLarge(bytes)) => return Ok(Material::TooLarge { bytes, page }),
                Err(error) => return Ok(lms_failed(failure_of(error))),
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
        Ok(Material::Ready { file, page })
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
            .load_fields(&[])
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

fn failed(reason: MaterialFailure, page: Option<SchoolWebTarget>) -> Material {
    Material::Failed { reason, page }
}

/// The folder of the visible course `id` (`CourseDetail`'s own naming).
fn course_key(store: &DataStore, id: &str) -> Option<CourseKey> {
    let workspace = store.load_fields(&["courses"]).ok()?;
    crate::schedule::visible_by_id(&workspace, "courses", id).and_then(crate::schedule::files_of)
}

/// A link that is not an LMS resource: a school site with an adapter opens
/// in its signed-in web view; any other HTTPS page goes to the system
/// browser. Plain HTTP, embedded credentials or a port are refused, so a
/// school host over HTTP never becomes a page the web view then rejects.
fn link(raw: &str) -> Material {
    if let Some(site) = automation_target(raw).and_then(|url| SchoolSite::for_url(url.as_str())) {
        return Material::Page {
            site,
            url: raw.to_owned(),
        };
    }
    if safe_external_url(raw) {
        return Material::External {
            url: raw.to_owned(),
        };
    }
    failed(MaterialFailure::Invalid, None)
}

/// The LMS module an address belongs to (`mod/<module>/…`), `pluginfile`
/// for a file, else `-`; named in diagnostics only.
fn module_of(url: &Url) -> &str {
    let segments: Vec<&str> = url
        .path_segments()
        .map(Iterator::collect)
        .unwrap_or_default();
    if segments.contains(&"pluginfile.php") {
        return "pluginfile";
    }
    segments
        .windows(2)
        .find(|pair| pair[0] == "mod")
        .map_or("-", |pair| pair[1])
}

fn failure_of(error: FetchError) -> MaterialFailure {
    match error {
        FetchError::Login => MaterialFailure::Login,
        FetchError::Invalid => MaterialFailure::Invalid,
        FetchError::Network => MaterialFailure::Network,
        FetchError::Timeout => MaterialFailure::Timeout,
        FetchError::Cancelled => MaterialFailure::Cancelled,
        FetchError::WriteDenied => MaterialFailure::Denied,
    }
}

fn bounded_download_limit(limit: Option<u64>) -> Option<u64> {
    limit.map(|bytes| bytes.clamp(1, MAX_AUTO_DOWNLOAD_BYTES))
}

pub fn material_path(root: &Path, course: &CourseKey, url: &Url) -> Result<PathBuf, MaterialError> {
    let path = storage_dir(
        &crate::storage::material_root(root),
        &[&course.term, &course.course, "materials"],
    )?
    .join(material_name(url));
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
    let digest = data_encoding::HEXLOWER.encode(&Sha256::digest(url.as_str()));
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

/// An LMS address Core downloads from: a `pluginfile.php` file, or the view
/// page of a resource, link or folder with its id. Anything else is not a
/// download (`lms_page`, `link`).
fn lms_resource_url(raw: &str) -> Option<(Url, LmsResourceKind)> {
    let url = Url::parse(raw).ok()?;
    if !adpt_isct::lms::is_lms_origin(&url) {
        return None;
    }
    let segments = url
        .path_segments()
        .map(Iterator::collect::<Vec<_>>)
        .unwrap_or_default();
    if segments.contains(&"pluginfile.php") {
        return Some((url, LmsResourceKind::File));
    }
    let [.., "mod", module @ ("resource" | "url" | "folder"), "view.php"] = segments.as_slice()
    else {
        return None;
    };
    let cmid = url
        .query_pairs()
        .find(|(key, _)| key == "id")
        .and_then(|(_, value)| value.parse::<u64>().ok())
        .filter(|value| *value > 0)?;
    let module = (*module).to_string();
    Some((url, LmsResourceKind::View { module, cmid }))
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
    files.iter().all(|file| {
        matches!(
            lms_resource_url(&file.url),
            Some((_, LmsResourceKind::File))
        )
    })
}

/// An outside page the system browser may open: HTTPS on the default port,
/// without embedded credentials.
fn safe_external_url(raw: &str) -> bool {
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
    let age = Duration::from_secs(days.clamp(1, MAX_RETENTION_DAYS) * 24 * 60 * 60);
    let mut removed = 0;
    for directory in material_directories(root)? {
        removed += sweep_material_directory(&directory, Some(age), true)?.files;
    }
    Ok(removed)
}

/// Files a sweep removed, or would remove.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RemovedFiles {
    pub files: usize,
    pub bytes: u64,
}

/// The downloaded course files a reset removes, or with `delete` false would
/// remove: every material cache file and every partial download orphaned by
/// another process under `root`, whatever its age. Notes, workspaces,
/// symlinks and this process's partial downloads are never traversed or
/// removed.
pub fn purge_downloads(root: &Path, delete: bool) -> Result<RemovedFiles, MaterialError> {
    let mut removed = RemovedFiles::default();
    for directory in material_directories(root)? {
        let swept = sweep_material_directory(&directory, None, delete)?;
        removed.files += swept.files;
        removed.bytes += swept.bytes;
    }
    Ok(removed)
}

/// The `materials` folders of the material tree, current and legacy:
/// `<term>/<course>/materials` and `<term>/<course>/<section>/materials`.
fn material_directories(root: &Path) -> Result<Vec<PathBuf>, MaterialError> {
    let root = crate::storage::material_root(root);
    if !root.exists() {
        return Ok(Vec::new());
    }
    let root = root.canonicalize().map_err(storage_error)?;
    let mut directories = Vec::new();
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
            directories.push(course.join("materials"));
            for section in child_directories(&course)? {
                if section.file_name().is_some_and(|name| {
                    matches!(
                        name.to_str(),
                        Some("materials" | "notes" | "deleted-notes" | "workspace" | "workspaces")
                    )
                }) {
                    continue;
                }
                directories.push(section.join("materials"));
            }
        }
    }
    Ok(directories)
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

/// Cache files of one `materials` folder older than `age` (`None`: of any
/// age), and partial downloads another process left behind.
fn sweep_material_directory(
    path: &Path,
    age: Option<Duration>,
    delete: bool,
) -> Result<RemovedFiles, MaterialError> {
    let mut removed = RemovedFiles::default();
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(removed);
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Ok(removed);
    }
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
            Some(true) => Some(STALE_PARTIAL_AGE),
            Some(false) => continue,
            None if is_material_cache_name(&path) => age,
            None => continue,
        };
        let metadata = entry.metadata().ok();
        let stale = limit.is_none_or(|limit| {
            metadata
                .as_ref()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|elapsed| elapsed > limit)
        });
        if !stale {
            continue;
        }
        if !delete || fs::remove_file(path).is_ok() {
            removed.files += 1;
            removed.bytes += metadata.map_or(0, |metadata| metadata.len());
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
