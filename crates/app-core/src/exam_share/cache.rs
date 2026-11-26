use super::remote::RemoteDocument;
use super::{ExamAccountStatus, ExamDocument, ExamError, ExamLocalDocument, ExamShare};
use crate::locale::ContentLanguage;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use tokio::io::AsyncWriteExt;

/// A cached copy's record, beside its file. It took this name (1.4.0's was
/// `metadata.json`) when the record changed, so 1.4.0's copies are never
/// read: maintenance sweeps them like any stale file.
pub(super) const MANIFEST: &str = "document.json";
const DOWNLOAD_LIFETIME: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const PREVIEW_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Cached {
    pub(super) document: RemoteDocument,
    pub(super) updated_at: u64,
    pub(super) downloaded: bool,
    // A new download uses a different name, so replacing the manifest works
    // while a native preview still holds the previous file open.
    pub(super) file: uuid::Uuid,
}

pub(super) fn account_folder(folder: &Path, uid: &str) -> PathBuf {
    folder.join(key(uid))
}

fn key(value: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(value.as_bytes()))
}

impl ExamShare {
    pub(super) fn account_folder(&self, uid: &str) -> PathBuf {
        account_folder(&self.folder, uid)
    }

    pub(super) fn document_folder(&self, uid: &str, id: &str) -> PathBuf {
        self.account_folder(uid).join(key(id))
    }

    /// The cache folder of `id` for the connected account. The returned
    /// session gate serializes the caller with new versions, sign-out and
    /// cache maintenance.
    async fn connected_document(
        &self,
        id: &str,
    ) -> Result<(tokio::sync::MutexGuard<'_, super::Session>, PathBuf), ExamError> {
        let session = self.session.lock().await;
        let saved = session.saved.as_ref().ok_or(ExamError::SignedOut)?;
        if session.status != ExamAccountStatus::Connected || !saved.registered {
            return Err(ExamError::SignedOut);
        }
        let base = self.document_folder(&saved.uid, id);
        Ok((session, base))
    }

    pub(crate) async fn retain_open_copy(
        &self,
        id: &str,
        opened_path: &Path,
    ) -> Result<bool, ExamError> {
        let (_session, base) = self.connected_document(id).await?;
        retain(&base, opened_path).await
    }

    pub(crate) async fn open_copy_export_name(
        &self,
        id: &str,
        opened_path: &Path,
    ) -> Result<String, ExamError> {
        let (_session, base) = self.connected_document(id).await?;
        export_name(&base, opened_path).await
    }
}

/// The extensions a validated type may carry, the cache file's own first.
fn extensions(file_type: &str) -> Result<&'static [&'static str], ExamError> {
    Ok(match file_type {
        "application/pdf" => &["pdf"],
        "image/jpeg" => &["jpg", "jpeg"],
        "image/png" => &["png"],
        "image/webp" => &["webp"],
        "text/plain" => &["txt", "md"],
        _ => return Err(ExamError::Response),
    })
}

pub(super) fn file_path(base: &Path, cached: &Cached) -> Result<PathBuf, ExamError> {
    let suffix = extensions(&cached.document.file_type)?[0];
    Ok(base.join(format!("{}.{suffix}", cached.file)))
}

pub(super) async fn local(
    base: &Path,
    cached: Cached,
    offline: bool,
    saved: Option<bool>,
    language: ContentLanguage,
) -> Result<ExamLocalDocument, ExamError> {
    let path = file_path(base, &cached)?;
    // The cached file has a generated name; the upload's name and validated
    // type say what it is.
    let (name, mime) = (
        cached.document.file_name.clone(),
        cached.document.file_type.clone(),
    );
    let (path, viewer) = blocking(move || {
        touch(&path)?;
        let viewer = crate::file_preview::viewer_of_file(&path, &name, Some(&mime));
        let path = path
            .into_os_string()
            .into_string()
            .map_err(|_| ExamError::Storage)?;
        Ok((path, viewer))
    })
    .await?;
    Ok(ExamLocalDocument {
        export_name: document_export_name(&cached.document)?,
        viewer,
        web_url: super::client::site_page(&format!("view/{}", cached.document.id)),
        document: cached.document.present(language),
        path,
        offline,
        downloaded: cached.downloaded,
        saved,
    })
}

fn touch(path: &Path) -> Result<(), ExamError> {
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(SystemTime::now()))
        .map_err(storage)
}

/// The cached version, provided it is exactly the one the caller opened.
async fn opened(base: &Path, opened_path: &Path) -> Result<(Cached, PathBuf), ExamError> {
    let cached = cache_file(base).await?.ok_or(ExamError::NotFound)?;
    let path = file_path(base, &cached)?;
    if path.as_path() != opened_path {
        return Err(ExamError::NotFound);
    }
    Ok((cached, path))
}

/// Retain exactly the version the caller opened.
async fn retain(base: &Path, opened_path: &Path) -> Result<bool, ExamError> {
    let (mut cached, path) = opened(base, opened_path).await?;
    if cached.downloaded {
        return Ok(false);
    }
    blocking(move || touch(&path)).await?;
    cached.downloaded = true;
    save_metadata(base, &cached).await?;
    Ok(true)
}

/// The name a copy of the opened version is exported under. The uploader
/// chose the name, so an extension its validated type does not allow (a
/// `text/plain` upload named `answers.bat`) gets the type's own suffix.
async fn export_name(base: &Path, opened_path: &Path) -> Result<String, ExamError> {
    let (cached, _) = opened(base, opened_path).await?;
    document_export_name(&cached.document)
}

/// [`export_name`] of a document already in hand: every shell saves or
/// shares the opened copy under it.
pub(super) fn document_export_name(document: &RemoteDocument) -> Result<String, ExamError> {
    let allowed = extensions(&document.file_type)?;
    let name = crate::download_names::safe_name(&document.file_name, &document.file_type);
    let (_, extension) = crate::download_names::split_extension(&name);
    let extension = extension.trim_start_matches('.');
    if allowed
        .iter()
        .any(|allowed| extension.eq_ignore_ascii_case(allowed))
    {
        Ok(name)
    } else {
        Ok(format!("{name}.{}", allowed[0]))
    }
}

struct CacheEntry {
    cached: Cached,
    expired: bool,
}

pub(super) async fn cache_file(base: &Path) -> Result<Option<Cached>, ExamError> {
    let base = base.to_owned();
    blocking(move || {
        Ok(read_cache(&base, SystemTime::now())?
            .filter(|entry| !entry.expired)
            .map(|entry| entry.cached))
    })
    .await
}

fn read_cache(base: &Path, now: SystemTime) -> Result<Option<CacheEntry>, ExamError> {
    let body = match fs::read(base.join(MANIFEST)) {
        Ok(body) => body,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(storage(error)),
    };
    let cached: Cached = serde_json::from_slice(&body).map_err(storage)?;
    let path = file_path(base, &cached)?;
    let facts = match fs::symlink_metadata(path) {
        Ok(facts) if facts.is_file() => facts,
        Ok(_) => return Err(ExamError::Storage),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(storage(error)),
    };
    let retention = if cached.downloaded {
        DOWNLOAD_LIFETIME
    } else {
        PREVIEW_LIFETIME
    };
    // The payload mtime records its last open, or initial download for copies
    // created before access tracking. The manifest still records remote freshness.
    Ok(Some(CacheEntry {
        cached,
        expired: older_than(facts, now, retention)?,
    }))
}

pub(super) async fn save_metadata(base: &Path, cached: &Cached) -> Result<(), ExamError> {
    let data = serde_json::to_vec(cached).map_err(|_| ExamError::Storage)?;
    write_atomically(&base.join(MANIFEST), &data).await
}

/// Replace `path` with `data` whole: a crash leaves the old file or the new.
pub(super) async fn write_atomically(path: &Path, data: &[u8]) -> Result<(), ExamError> {
    let temp = TempFile(path.with_extension("new"));
    let mut file = tokio::fs::File::create(&temp.0)
        .await
        .map_err(|_| ExamError::Storage)?;
    file.write_all(data).await.map_err(|_| ExamError::Storage)?;
    file.sync_all().await.map_err(|_| ExamError::Storage)?;
    drop(file);
    tokio::fs::rename(&temp.0, path)
        .await
        .map_err(|_| ExamError::Storage)
}

pub(super) async fn erase(base: &Path) -> Result<(), ExamError> {
    match tokio::fs::remove_dir_all(base).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(storage(error)),
    }
}

fn remove_folder(base: &Path) -> Result<(), ExamError> {
    match fs::remove_dir_all(base) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(storage(error)),
    }
}

pub(super) async fn downloads(
    base: &Path,
    language: ContentLanguage,
) -> Result<Vec<ExamDocument>, ExamError> {
    let base = base.to_owned();
    blocking(move || {
        let now = SystemTime::now();
        let mut list = Vec::new();
        for folder in directories(&base)? {
            if let Some(entry) =
                read_cache(&folder, now)?.filter(|entry| !entry.expired && entry.cached.downloaded)
            {
                list.push(entry.cached.document.present(language));
            }
        }
        list.sort_by(|a, b| a.file_name.cmp(&b.file_name).then_with(|| a.id.cmp(&b.id)));
        Ok(list)
    })
    .await
}

pub(super) fn clean_root(base: &Path, now: SystemTime) -> Result<(), ExamError> {
    for owner in directories(base)? {
        for folder in directories(&owner)? {
            sweep(&folder, now)?;
        }
    }
    Ok(())
}

fn sweep(folder: &Path, now: SystemTime) -> Result<(), ExamError> {
    let retained = match read_cache(folder, now)? {
        Some(entry) if entry.expired => return remove_folder(folder),
        Some(entry) => Some(file_path(folder, &entry.cached)?),
        None => None,
    };
    let Some(entries) = read_dir(folder)? else {
        return Ok(());
    };
    for entry in entries {
        let entry = entry.map_err(storage)?;
        let path = entry.path();
        if retained
            .as_ref()
            .is_some_and(|live| live == &path || entry.file_name() == MANIFEST)
        {
            continue;
        }
        let facts = entry.metadata().map_err(storage)?;
        if facts.is_file() && older_than(facts, now, PREVIEW_LIFETIME)? {
            fs::remove_file(path).map_err(storage)?;
        }
    }
    if retained.is_none() {
        // Interrupted saves can leave an empty document directory.
        match fs::remove_dir(folder) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::DirectoryNotEmpty => {}
            Err(error) => return Err(storage(error)),
        }
    }
    Ok(())
}

fn directories(base: &Path) -> Result<Vec<PathBuf>, ExamError> {
    let Some(entries) = read_dir(base)? else {
        return Ok(Vec::new());
    };
    let mut folders = Vec::new();
    for entry in entries {
        let entry = entry.map_err(storage)?;
        if entry.file_type().map_err(storage)?.is_dir() {
            folders.push(entry.path());
        }
    }
    Ok(folders)
}

fn read_dir(base: &Path) -> Result<Option<fs::ReadDir>, ExamError> {
    match fs::read_dir(base) {
        Ok(entries) => Ok(Some(entries)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(storage(error)),
    }
}

fn older_than(facts: fs::Metadata, now: SystemTime, lifetime: Duration) -> Result<bool, ExamError> {
    Ok(now
        .duration_since(facts.modified().map_err(storage)?)
        .is_ok_and(|age| age >= lifetime))
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, ExamError> + Send + 'static,
) -> Result<T, ExamError> {
    tokio::task::spawn_blocking(work).await.map_err(storage)?
}

fn storage(_: impl std::error::Error) -> ExamError {
    ExamError::Storage
}

pub(super) struct TempFile(pub(super) PathBuf);
impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(test)]
#[path = "../tests/exam_share__cache.rs"]
mod tests;
