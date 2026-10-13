//! Cross-platform course material resolution, download and cache ownership.

use crate::{
    contracts::FolderFile,
    data::DataStore,
    downloads::{DownloadSession, Downloader, FetchError, Fetched, ResolvedResource},
    school_parser::lms::ResolvedFile,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};
use url::Url;

const MAX_AUTO_DOWNLOAD_BYTES: u64 = 1024 * 1024 * 1024;
const DEFAULT_AUTO_DOWNLOAD_BYTES: u64 = 20 * 1024 * 1024;
const MAX_RETENTION_DAYS: u64 = 3650;
const MAX_TEXT_PREVIEW_BYTES: u64 = 1_000_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
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

#[derive(Debug, thiserror::Error)]
pub enum MaterialError {
    #[error("{0}")]
    Input(String),
    #[error("{0}")]
    Storage(String),
    #[error("Course files are resolved in the desktop app")]
    UnsupportedResolution,
    #[error("Course files are downloaded in the desktop app")]
    UnsupportedDownload,
}

#[derive(Clone)]
pub struct CourseMaterials {
    store: DataStore,
    root: PathBuf,
    downloads: Downloader,
    session: Option<Arc<dyn DownloadSession>>,
}

impl CourseMaterials {
    pub fn new(
        store: DataStore,
        root: PathBuf,
        downloads: Downloader,
        session: Option<Arc<dyn DownloadSession>>,
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
        let limit = (!explicit).then(|| self.automatic_download_limit());
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
                let stored = self.store.lms_material_for_url(&source).ok().flatten();
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
                            let _ = self.store.update_lms_resource_url(cmid, &source, &url);
                            url_value
                        }
                        Ok(ResolvedResource::External { url }) => {
                            if !safe_external_url(&url) {
                                return Ok(failed(MaterialFailure::Invalid));
                            }
                            let _ = self.store.update_lms_resource_url(cmid, &source, &url);
                            return Ok(Material::External { url });
                        }
                        Ok(ResolvedResource::Folder { files }) => {
                            if !valid_folder_files(&files) {
                                return Ok(failed(MaterialFailure::Invalid));
                            }
                            let _ = self.store.update_lms_folder_files(
                                cmid,
                                &source,
                                &folder_contract(&files),
                            );
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

        let path = material_path(&self.root, &context, &url)?;
        if !regular_file(&path)? {
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
            if !regular_file(&path)? {
                return Err(MaterialError::Storage(
                    "Downloaded course material is unavailable".into(),
                ));
            }
        }
        let _ = fs::File::options()
            .write(true)
            .open(&path)
            .and_then(|file| file.set_modified(SystemTime::now()));
        Ok(Material::Ready {
            file: describe(path, url.to_string(), String::new(), true, read_text)?,
        })
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

    /// Remove stale cache files from the current and legacy material layouts.
    /// User notes, workspace files, symlinks and active partial downloads are
    /// never traversed or removed.
    pub async fn clean(&self, days: u64) -> Result<usize, MaterialError> {
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || clean_materials(&root, days))
            .await
            .map_err(|error| MaterialError::Storage(error.to_string()))?
    }
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
    })
}

fn bounded_download_limit(limit: Option<u64>) -> Option<u64> {
    limit.map(|bytes| bytes.clamp(1, MAX_AUTO_DOWNLOAD_BYTES))
}

pub fn component(value: &str) -> Result<&str, MaterialError> {
    if value.is_empty()
        || value.len() > 120
        || value == "."
        || value == ".."
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        return Err(MaterialError::Input(
            "Invalid course folder component".into(),
        ));
    }
    Ok(value)
}

pub fn storage_dir(root: &Path, parts: &[&str]) -> Result<PathBuf, MaterialError> {
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
                return Err(MaterialError::Storage(
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
) -> Result<LocalFile, MaterialError> {
    if fs::symlink_metadata(&path)
        .map_err(storage_error)?
        .file_type()
        .is_symlink()
    {
        return Err(MaterialError::Storage("Cannot read symbolic link".into()));
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

/// Validate a file selected from a material result before the shell opens it.
pub fn stored_file(root: &Path, raw_path: &str) -> Result<PathBuf, MaterialError> {
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

fn outside_storage() -> MaterialError {
    MaterialError::Storage("File is outside the course folder".into())
}

fn regular_file(path: &Path) -> Result<bool, MaterialError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(MaterialError::Storage("Cannot read symbolic link".into()))
        }
        Ok(metadata) => Ok(metadata.is_file()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(storage_error(error)),
    }
}

fn storage_error(error: impl ToString) -> MaterialError {
    MaterialError::Storage(error.to_string())
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
        if !file_type.is_file() || !is_material_cache_name(&path) {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|elapsed| elapsed > age);
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
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    fn store(name: &str) -> DataStore {
        let path =
            std::env::temp_dir().join(format!("course-materials-{name}-{}.db", Uuid::new_v4()));
        DataStore::initialize(path).unwrap().0
    }

    fn root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("course-materials-{name}-{}", Uuid::new_v4()))
    }

    fn course() -> CourseKey {
        CourseKey {
            term: "2026-3Q".into(),
            course: "CSC.T251".into(),
        }
    }

    #[test]
    fn url_paths_and_names_preserve_the_desktop_contract() {
        for value in ["..", ".", "x/y", "x\\y", "", "../../notes"] {
            assert!(component(value).is_err());
        }
        assert!(component("2026-3Q").is_ok());
        assert!(component("CSC.T251").is_ok());
        let japanese = material_name(&Url::parse("https://lms.s.isct.ac.jp/2025/pluginfile.php/224957/mod_resource/content/1/01%E5%9B%9E%E7%9B%AE%20SW-0%20v6.pdf").unwrap());
        assert!(japanese.starts_with("01回目 SW-0 v6-") && japanese.ends_with(".pdf"));
        assert_ne!(
            material_name(
                &Url::parse("https://lms.s.isct.ac.jp/2025/pluginfile.php/1/a.pdf").unwrap()
            ),
            material_name(
                &Url::parse("https://lms.s.isct.ac.jp/2025/pluginfile.php/2/a.pdf").unwrap()
            )
        );
        assert!(matches!(
            lms_resource_url("https://lms.s.isct.ac.jp/2025/mod/resource/view.php?id=42"),
            Ok((_, LmsResourceKind::View { cmid: 42, .. }))
        ));
        assert!(!safe_external_url("https://user:secret@example.test/file"));
    }

    #[tokio::test]
    async fn cached_hit_and_missing_do_not_require_a_download_session() {
        let root = root("cache");
        let service =
            CourseMaterials::new(store("cache"), root.clone(), Downloader::default(), None);
        let source = "https://lms.s.isct.ac.jp/2026/pluginfile.php/1/readme.txt";
        let url = Url::parse(source).unwrap();
        let path = material_path(&root, &course(), &url).unwrap();
        fs::write(&path, "cached text").unwrap();

        let material = service
            .open(course(), source.into(), true, true, Some(1))
            .await
            .unwrap();
        let Material::Ready { file } = material else {
            panic!("cached material should be ready");
        };
        assert!(file.cached);
        assert_eq!(file.text.as_deref(), Some("cached text"));

        let missing = service
            .open(
                course(),
                "https://lms.s.isct.ac.jp/2026/pluginfile.php/2/missing.pdf".into(),
                false,
                false,
                None,
            )
            .await
            .unwrap();
        assert_eq!(missing, Material::Missing);
    }

    #[tokio::test]
    async fn unavailable_native_operations_keep_distinct_typed_errors() {
        let service = CourseMaterials::new(
            store("unsupported"),
            root("unsupported"),
            Downloader::default(),
            None,
        );
        assert!(matches!(
            service
                .open(
                    course(),
                    "https://lms.s.isct.ac.jp/2026/mod/resource/view.php?id=9".into(),
                    false,
                    false,
                    None,
                )
                .await,
            Err(MaterialError::UnsupportedResolution)
        ));
        assert!(matches!(
            service
                .open(
                    course(),
                    "https://lms.s.isct.ac.jp/2026/pluginfile.php/9/a.pdf".into(),
                    true,
                    false,
                    None,
                )
                .await,
            Err(MaterialError::UnsupportedDownload)
        ));
    }

    #[test]
    fn stored_folders_validate_every_file_and_fetch_errors_keep_wire_reasons() {
        let valid = json!({"folderFiles":[
            {"name":"one","url":"https://lms.s.isct.ac.jp/2026/pluginfile.php/1/one.pdf"},
            {"name":"two","url":"https://lms.s.isct.ac.jp/2026/pluginfile.php/2/two.pdf"}
        ]});
        assert_eq!(stored_folder_files(Some(&valid)).unwrap().len(), 2);
        let invalid = json!({"folderFiles":[
            {"name":"one","url":"https://lms.s.isct.ac.jp/2026/pluginfile.php/1/one.pdf"},
            {"name":"bad","url":"https://example.test/two.pdf"}
        ]});
        assert!(stored_folder_files(Some(&invalid)).is_none());
        for (error, reason) in [
            (FetchError::Login, MaterialFailure::Login),
            (FetchError::Invalid, MaterialFailure::Invalid),
            (FetchError::Network, MaterialFailure::Network),
            (FetchError::Timeout, MaterialFailure::Timeout),
            (FetchError::Cancelled, MaterialFailure::Cancelled),
        ] {
            assert_eq!(material_for_fetch_error(error), failed(reason));
        }
        assert_eq!(
            serde_json::to_value(failed(MaterialFailure::Login)).unwrap()["reason"],
            "login"
        );
        assert_eq!(bounded_download_limit(Some(0)), Some(1));
        assert_eq!(
            bounded_download_limit(Some(u64::MAX)),
            Some(MAX_AUTO_DOWNLOAD_BYTES)
        );
        assert_eq!(bounded_download_limit(None), None);
        assert!(is_material_cache_name(Path::new("foo.bar-12345678")));
        assert!(is_material_cache_name(Path::new("foo.bar-12345678.pdf")));
        assert!(!is_material_cache_name(Path::new(
            "README-12345678.part-10-2"
        )));
    }

    #[cfg(unix)]
    #[test]
    fn stored_file_accepts_an_alias_for_the_same_root_without_following_children() {
        use std::os::unix::fs::symlink;

        let actual = root("stored-alias");
        fs::create_dir_all(&actual).unwrap();
        let alias = actual.with_extension("alias");
        symlink(&actual, &alias).unwrap();
        let file = alias.join("handout.txt");
        fs::write(&file, "safe").unwrap();
        assert_eq!(
            stored_file(&alias, &file.to_string_lossy()).unwrap(),
            actual.join("handout.txt").canonicalize().unwrap()
        );

        let outside = actual.with_extension("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.txt"), "secret").unwrap();
        symlink(&outside, actual.join("linked")).unwrap();
        assert!(stored_file(&alias, &alias.join("linked/secret.txt").to_string_lossy()).is_err());

        fs::remove_file(alias).unwrap();
        fs::remove_dir_all(actual).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn binary_preview_is_optional_and_text_preview_is_bounded() {
        let root = root("preview");
        fs::create_dir_all(&root).unwrap();
        let binary = root.join("image.png");
        fs::write(&binary, [0x89, b'P', b'N', b'G', 0xff]).unwrap();
        assert_eq!(
            describe(binary, "url".into(), String::new(), true, true)
                .unwrap()
                .text,
            None
        );
        let large = root.join("large.txt");
        fs::write(&large, "x".repeat(MAX_TEXT_PREVIEW_BYTES as usize + 1)).unwrap();
        assert_eq!(
            describe(large, "url".into(), String::new(), true, true)
                .unwrap()
                .text,
            None
        );
    }

    #[tokio::test]
    async fn cleanup_removes_only_stale_complete_material_cache_files() {
        let root = root("cleanup");
        let current = root.join("2026-3Q/CSC.T251/materials");
        let legacy = root.join("2026-3Q/CSC.T251/section-1/materials");
        let notes = root.join("2026-3Q/CSC.T251/section-1/notes");
        let workspace = root.join("workspaces/local/materials");
        let nested_notes_materials = root.join("2026-3Q/CSC.T251/notes/materials");
        let nested_workspace_materials = root.join("2026-3Q/CSC.T251/workspace/materials");
        for directory in [
            &current,
            &legacy,
            &notes,
            &workspace,
            &nested_notes_materials,
            &nested_workspace_materials,
        ] {
            fs::create_dir_all(directory).unwrap();
        }
        let stale = [
            current.join("one-1234abcd.pdf"),
            legacy.join("two-abcdef12"),
        ];
        for path in &stale {
            fs::write(path, "cache").unwrap();
            fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
                .unwrap();
        }
        let part = current.join("active-1234abcd.pdf.part");
        let note = notes.join("note.md");
        let workspace_file = workspace.join("user-1234abcd.pdf");
        let nested_note = nested_notes_materials.join("user-1234abcd.pdf");
        let nested_workspace = nested_workspace_materials.join("user-abcdef12.pdf");
        for path in [
            &part,
            &note,
            &workspace_file,
            &nested_note,
            &nested_workspace,
        ] {
            fs::write(path, "keep").unwrap();
            fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
                .unwrap();
        }
        let service =
            CourseMaterials::new(store("cleanup"), root.clone(), Downloader::default(), None);
        assert_eq!(service.clean(1).await.unwrap(), 2);
        assert!(stale.iter().all(|path| !path.exists()));
        assert!(part.exists());
        assert!(note.exists());
        assert!(workspace_file.exists());
        assert!(nested_note.exists());
        assert!(nested_workspace.exists());
    }
}
