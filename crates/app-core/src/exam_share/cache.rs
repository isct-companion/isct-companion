use super::{ExamDocument, ExamError, ExamLocalDocument};
use serde::Deserialize;
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use tokio::io::AsyncWriteExt;

const DOWNLOAD_LIFETIME: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const PREVIEW_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Cached {
    pub(super) document: ExamDocument,
    pub(super) updated_at: u64,
    pub(super) downloaded: bool,
    // A new download uses a different name, so replacing the manifest works
    // while a native preview still holds the previous file open.
    pub(super) file: uuid::Uuid,
}

pub(super) fn file_path(base: &Path, cached: &Cached) -> Result<PathBuf, ExamError> {
    let suffix = match cached.document.file_type.as_str() {
        "application/pdf" => "pdf",
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "text/plain" => "txt",
        _ => return Err(ExamError::Response),
    };
    Ok(base.join(format!("{}.{suffix}", cached.file)))
}

pub(super) async fn local(
    base: &Path,
    cached: Cached,
    offline: bool,
    saved: Option<bool>,
) -> Result<ExamLocalDocument, ExamError> {
    let path = file_path(base, &cached)?;
    let path = blocking(move || {
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .and_then(|file| file.set_modified(SystemTime::now()))
            .map_err(storage)?;
        path.into_os_string()
            .into_string()
            .map_err(|_| ExamError::Storage)
    })
    .await?;
    Ok(ExamLocalDocument {
        web_url: super::client::site_page(&format!("view/{}", cached.document.id)),
        document: cached.document,
        path,
        offline,
        downloaded: cached.downloaded,
        saved,
    })
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
    let body = match fs::read(base.join("metadata.json")) {
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
    write_atomically(&base.join("metadata.json"), &data).await
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

pub(super) async fn downloads(base: &Path) -> Result<Vec<ExamDocument>, ExamError> {
    let base = base.to_owned();
    blocking(move || {
        let now = SystemTime::now();
        let mut list = Vec::new();
        for folder in directories(&base)? {
            if let Some(entry) =
                read_cache(&folder, now)?.filter(|entry| !entry.expired && entry.cached.downloaded)
            {
                list.push(entry.cached.document);
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
            .is_some_and(|live| live == &path || entry.file_name() == "metadata.json")
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
