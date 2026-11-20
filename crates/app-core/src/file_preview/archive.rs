//! Reading a downloaded ZIP without unpacking all of it: its files are
//! listed when the archive is small enough to show as a folder, and one file
//! at a time is extracted when it is opened (`course_materials`,
//! `Material::Archive`).

use crate::course_materials::MaterialError;
use serde::Serialize;
use std::{
    collections::HashSet,
    fs,
    io::{self, Read},
    path::Path,
};

/// More files than this are not listed: the archive opens as a file.
const MAX_LISTED_FILES: usize = 1_000;
/// Nor is an archive whose files add up to more than this unpacked.
const MAX_LISTED_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// One file inside an archive: its path there, with `/` separators, and its
/// unpacked size.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ArchiveEntry {
    pub name: String,
    pub bytes: u64,
}

/// What extracting one file gave.
#[derive(Debug, PartialEq, Eq)]
pub enum Extracted {
    Saved,
    /// The file is larger than the limit; nothing was written.
    TooLarge(u64),
    /// The archive lists no such file.
    Missing,
}

/// The files of the ZIP at `archive`, sorted by path so each folder's files
/// stay together (archive order is the packer's), or `None` when it
/// is not one to show as a folder: unreadable, more than `MAX_LISTED_FILES`
/// files or `MAX_LISTED_BYTES` unpacked. Folders, symbolic links, unsafe
/// paths, system leftovers (`__MACOSX`, `.DS_Store`) and a second file of
/// the same path are left out.
pub fn list(archive: &Path) -> Option<Vec<ArchiveEntry>> {
    let mut zip = open(archive).ok()?;
    if zip.len() > MAX_LISTED_FILES * 2 {
        return None;
    }
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    let mut total = 0_u64;
    for index in 0..zip.len() {
        let entry = zip.by_index_raw(index).ok()?;
        let Some(name) = listed(&entry) else {
            continue;
        };
        if !seen.insert(name.clone()) {
            continue;
        }
        total = total.saturating_add(entry.size());
        entries.push(ArchiveEntry {
            name,
            bytes: entry.size(),
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    (entries.len() <= MAX_LISTED_FILES && total <= MAX_LISTED_BYTES).then_some(entries)
}

/// Extracts the file `name` of the ZIP at `archive` to `target`, unless it
/// is larger than `limit` bytes.
pub fn extract(
    archive: &Path,
    name: &str,
    target: &Path,
    limit: Option<u64>,
) -> Result<Extracted, MaterialError> {
    let mut zip = open(archive)?;
    let Some(index) = (0..zip.len()).find(|&index| {
        zip.by_index_raw(index)
            .ok()
            .and_then(|entry| listed(&entry))
            .is_some_and(|listed| listed == name)
    }) else {
        return Ok(Extracted::Missing);
    };
    let mut entry = zip.by_index(index).map_err(unreadable)?;
    let size = entry.size();
    let budget = limit.unwrap_or(MAX_LISTED_BYTES).min(MAX_LISTED_BYTES);
    if size > budget {
        return Ok(Extracted::TooLarge(size));
    }
    let partial = target.with_extension("partial");
    let mut output = fs::File::create(&partial).map_err(io_error)?;
    // The declared size can lie; never write more than the budget.
    let written = io::copy(&mut (&mut entry).take(budget + 1), &mut output);
    drop(output);
    match written {
        Ok(written) if written <= budget => {
            fs::rename(&partial, target).map_err(io_error)?;
            Ok(Extracted::Saved)
        }
        Ok(written) => {
            let _ = fs::remove_file(&partial);
            Ok(Extracted::TooLarge(written))
        }
        Err(_) => {
            let _ = fs::remove_file(&partial);
            Err(unreadable(()))
        }
    }
}

fn open(archive: &Path) -> Result<zip::ZipArchive<fs::File>, MaterialError> {
    let file = fs::File::open(archive).map_err(io_error)?;
    zip::ZipArchive::new(file).map_err(unreadable)
}

/// The name of an entry `list` shows, or `None` for one it leaves out.
fn listed(entry: &zip::read::ZipFile<'_, fs::File>) -> Option<String> {
    if !entry.is_file() || entry.is_symlink() || entry.enclosed_name().is_none() {
        return None;
    }
    let decoded = entry.name().ok();
    let name = entry_name(entry.name_raw(), decoded.as_deref());
    (!name.split('/').any(is_junk)).then_some(name)
}

/// A file or folder the system leaves in archives and folders, never the
/// person's own: listings and hand-ins skip it.
pub(crate) fn is_junk(name: &str) -> bool {
    name == ".DS_Store" || name == "__MACOSX" || name == "Thumbs.db"
}

/// An entry's path as the person named it. UTF-8 (flagged or not, as macOS
/// writes it) is taken as is; Japanese Windows writes Shift_JIS without a
/// flag; anything else is what the ZIP library decoded (CP437).
fn entry_name(raw: &[u8], decoded: Option<&str>) -> String {
    let name = match std::str::from_utf8(raw) {
        Ok(name) => name.to_owned(),
        Err(_) => {
            match encoding_rs::SHIFT_JIS.decode_without_bom_handling_and_without_replacement(raw) {
                Some(name) => name.into_owned(),
                None => {
                    decoded.map_or_else(|| String::from_utf8_lossy(raw).into_owned(), str::to_owned)
                }
            }
        }
    };
    name.replace('\\', "/")
}

fn unreadable(_: impl Sized) -> MaterialError {
    MaterialError::Input("This archive can't be read.".into())
}

fn io_error(error: io::Error) -> MaterialError {
    if crate::storage::denied(&error) {
        MaterialError::Denied(crate::storage::failure_text(error))
    } else {
        MaterialError::Storage(error.to_string())
    }
}

#[cfg(test)]
#[path = "../tests/file_preview__archive.rs"]
pub(crate) mod tests;
