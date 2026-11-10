//! The snapshots LMS writes upload, by sha256
//! (`approach/snapshot-reference-publication`).
//!
//! A capture copies `submit/` into a private `staging/<boot>/<capture>/`,
//! hashing the bytes as they are copied, and syncs them; nothing else looks
//! there. [`Snapshots::publish`] then holds the publication lock while it
//! places each object and runs the caller's ledger transaction that records
//! the references: an object already there and intact is reused, a missing
//! or damaged one is replaced by the complete staged file in one rename
//! (never deleted first), and the folder is synced before the references
//! commit. A crash in between leaves objects nobody names, never a name
//! without its object. Collection runs in maintenance only, under the same
//! lock, after the references it drops have committed.
use super::{SnapshotFile, SubmissionError, UploadFile};
use crate::data::StoreRoot;
use crate::workspaces::HandInError;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
};

const OBJECTS: &str = "objects";
const STAGING: &str = "staging";

pub(super) struct Snapshots {
    objects: PathBuf,
    staging: PathBuf,
    /// This run's staging folder; other runs' are left by crashes.
    boot: String,
    publication: Mutex<()>,
    /// A publication on a blocking thread may outlive its caller; the
    /// data folder stays this process's until it ends.
    _root: Arc<StoreRoot>,
}

/// One capture's staged copies; its folder goes when it is dropped.
pub(super) struct Capture {
    folder: PathBuf,
    staged: Vec<(SnapshotFile, PathBuf)>,
}

impl Capture {
    pub(super) fn files(&self) -> Vec<SnapshotFile> {
        self.staged.iter().map(|(file, _)| file.clone()).collect()
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.folder);
    }
}

impl Snapshots {
    pub(super) fn open(generation: &Path, root: Arc<StoreRoot>) -> Result<Self, SubmissionError> {
        let objects = generation.join(OBJECTS);
        let staging = generation.join(STAGING);
        fs::create_dir_all(&objects)?;
        fs::create_dir_all(&staging)?;
        Ok(Self {
            objects,
            staging,
            boot: root.boot().to_owned(),
            publication: Mutex::new(()),
            _root: root,
        })
    }

    /// Copies the regular files `names` of `submit` into a new staging
    /// folder, hashing the bytes as they are copied, so the digest is that
    /// of what is kept, whatever happens to `submit/` meanwhile. By name.
    pub(super) fn capture(&self, submit: &Path, names: &[String]) -> Result<Capture, SubmissionError> {
        if names.is_empty() {
            return Err(HandInError::Empty.into());
        }
        let folder = self
            .staging
            .join(&self.boot)
            .join(uuid::Uuid::now_v7().to_string());
        fs::create_dir_all(&folder)?;
        let mut capture = Capture {
            folder,
            staged: Vec::new(),
        };
        for (index, name) in names.iter().enumerate() {
            let source = submit.join(name);
            let kind = fs::symlink_metadata(&source)?;
            if kind.is_dir() {
                return Err(HandInError::NeedsZip.into());
            }
            if !kind.is_file() {
                return Err(HandInError::FilesOnly.into());
            }
            let staged = capture.folder.join(index.to_string());
            let (sha256, bytes) = copy_hashed(&source, &staged)?;
            capture.staged.push((
                SnapshotFile {
                    name: name.clone(),
                    bytes,
                    sha256,
                },
                staged,
            ));
        }
        crate::storage::sync_directory(&capture.folder)?;
        capture
            .staged
            .sort_by(|(a, _), (b, _)| a.name.cmp(&b.name));
        Ok(capture)
    }

    /// Places the capture's objects, then runs `commit`, the ledger
    /// transaction that records the references to them, all under the
    /// publication lock.
    pub(super) fn publish<T>(
        &self,
        capture: Option<Capture>,
        commit: impl FnOnce() -> Result<T, SubmissionError>,
    ) -> Result<T, SubmissionError> {
        let _publication = self.publication.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(capture) = &capture {
            for (file, staged) in &capture.staged {
                let object = self.objects.join(&file.sha256);
                if digest_of(&object).ok() == Some((file.sha256.clone(), file.bytes)) {
                    continue;
                }
                // A damaged copy under this name is replaced whole.
                fs::rename(staged, &object)?;
            }
            crate::storage::sync_directory(&self.objects)?;
        }
        commit()
    }

    /// Whether every file's object is still there with its size and digest.
    pub(super) fn intact(&self, files: &[SnapshotFile]) -> bool {
        files.iter().all(|file| {
            digest_of(&self.objects.join(&file.sha256)).ok()
                == Some((file.sha256.clone(), file.bytes))
        })
    }

    /// The snapshot files as the transport uploads them.
    pub(super) fn uploads(&self, files: &[SnapshotFile]) -> Vec<UploadFile> {
        files
            .iter()
            .map(|file| UploadFile {
                name: file.name.clone(),
                path: self.objects.join(&file.sha256),
                bytes: file.bytes,
            })
            .collect()
    }

    /// Maintenance: `expire` commits the ledger's dropped references, then
    /// every object no reference names and the staging folders of earlier
    /// runs go, all under the publication lock. Returns how many objects
    /// went.
    pub(super) fn collect(
        &self,
        expire: impl FnOnce() -> Result<BTreeSet<String>, SubmissionError>,
    ) -> Result<usize, SubmissionError> {
        let _publication = self.publication.lock().unwrap_or_else(PoisonError::into_inner);
        let referenced = expire()?;
        let mut removed = 0;
        for entry in fs::read_dir(&self.objects)?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !referenced.contains(&name) && fs::remove_file(entry.path()).is_ok() {
                removed += 1;
            }
        }
        for entry in fs::read_dir(&self.staging)?.flatten() {
            if entry.file_name().to_string_lossy() != self.boot.as_str() {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
        Ok(removed)
    }
}

/// Copies `source` to a new file `target`, synced, and returns the copied
/// bytes' sha256 and size.
fn copy_hashed(source: &Path, target: &Path) -> Result<(String, u64), SubmissionError> {
    let mut input = fs::File::open(source)?;
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    let mut output = options.open(target)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut bytes = 0_u64;
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        output.write_all(&buffer[..read])?;
        bytes += read as u64;
    }
    output.sync_all()?;
    Ok((data_encoding::HEXLOWER.encode(&hasher.finalize()), bytes))
}

/// A file's sha256 and size.
pub(super) fn digest_of(path: &Path) -> std::io::Result<(String, u64)> {
    let mut input = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut bytes = 0_u64;
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        bytes += read as u64;
    }
    Ok((data_encoding::HEXLOWER.encode(&hasher.finalize()), bytes))
}
