//! Assignment workspace storage, archive and cleanup rules. Desktop opens a
//! workspace in Finder or VS Code; the phones fill its `submit/` with what was
//! shared into the app. Either way the hand-in reads `submit/`.

use crate::{
    course_materials::CourseKey,
    schedule::AssignmentDetail,
    storage::{self, LocalFile, StorageError},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime},
};

pub use crate::school_browser::{SubmissionFile, MAX_SUBMISSION_BYTES, MAX_SUBMISSION_FILES};

const META: &str = ".workspace.json";
const SUBMIT_DIR: &str = "submit";
const BRIEF: &str = "ASSIGNMENT.md";
#[cfg(feature = "workspace-archives")]
const MAX_UNPACK_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 50_000;
const STALE_AFTER: Duration = Duration::from_secs(30 * 24 * 60 * 60);

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("{0}")]
    Input(String),
    #[error("{0}")]
    Storage(String),
    #[error("{0}")]
    Archive(String),
    /// The folder was removed outside the app, for example in Finder.
    #[error("The workspace folder no longer exists")]
    Missing,
}

/// Why files cannot be handed in, beside the workspace's own failures.
#[derive(Debug, thiserror::Error)]
pub enum HandInError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error("Only files can be handed in.")]
    FilesOnly,
    #[error("The submit folder is empty")]
    Empty,
    #[error("Use ZIP to upload folders.")]
    NeedsZip,
    #[error("Upload assistance supports at most 20 MiB.")]
    TooLarge,
}

impl From<StorageError> for WorkspaceError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::Input(message) => Self::Input(message),
            StorageError::Storage(message) | StorageError::Denied(message) => {
                Self::Storage(message)
            }
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Meta {
    assignment: String,
    title: String,
    #[serde(default)]
    zip: bool,
    #[serde(default)]
    zip_name: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfo {
    pub path: String,
    pub name: String,
    /// Names directly inside `submit/`.
    pub submit_files: Vec<String>,
    pub zip: bool,
    pub zip_name: String,
}

/// What the assignment's main button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HandIn {
    /// Open the assignment's LMS page; nothing local is read.
    OpenLms,
    /// Upload `submit/` to the LMS draft area, then open it.
    PrepareDraft,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandInView {
    pub workspace: Option<WorkspaceInfo>,
    pub hand_in: HandIn,
}

pub struct PreparedUpload {
    pub info: WorkspaceInfo,
    pub files: Vec<SubmissionFile>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Attachment {
    pub path: String,
    pub name: String,
}

#[derive(Clone, Debug)]
pub enum SubmissionTarget {
    Folder(PathBuf),
    Archive(PathBuf),
}

#[derive(Clone, Debug)]
pub struct PreparedSubmission {
    pub info: WorkspaceInfo,
    pub target: SubmissionTarget,
}

#[derive(Clone)]
pub struct Workspaces {
    root: PathBuf,
}

impl Workspaces {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn find(
        &self,
        course: &CourseKey,
        assignment: &str,
    ) -> Result<Option<WorkspaceInfo>, WorkspaceError> {
        let parent = self.workspaces_dir(course)?;
        Ok(find(&parent, assignment).map(|(path, meta)| info(&path, &meta)))
    }

    /// The workspace and main action for one assignment. `closed` is the
    /// assignment's membership in `TaskView::closed_ids`: closed work, and
    /// open work without a workspace, go straight to LMS.
    pub fn hand_in(
        &self,
        course: &CourseKey,
        assignment: &str,
        closed: bool,
    ) -> Result<HandInView, WorkspaceError> {
        let workspace = self.find(course, assignment)?;
        let hand_in = if closed || workspace.is_none() {
            HandIn::OpenLms
        } else {
            HandIn::PrepareDraft
        };
        Ok(HandInView { workspace, hand_in })
    }

    /// Creates a workspace once. Existing files are never overwritten.
    pub fn create(
        &self,
        course: &CourseKey,
        assignment: String,
        title: String,
        brief: String,
        attachments: Vec<Attachment>,
    ) -> Result<WorkspaceInfo, WorkspaceError> {
        let parent = self.workspaces_dir(course)?;
        if let Some((path, meta)) = find(&parent, &assignment) {
            return Ok(info(&path, &meta));
        }
        let sources = attachments
            .into_iter()
            .map(|attachment| {
                Ok((
                    storage::stored_file(&self.root, &attachment.path)?,
                    safe_name(&attachment.name, "attachment"),
                ))
            })
            .collect::<Result<Vec<_>, WorkspaceError>>()?;
        let base = safe_name(&title, "Assignment");
        let dir = reserve_workspace(&parent, &base)?;
        let meta = Meta {
            assignment,
            title,
            zip: false,
            zip_name: String::new(),
        };
        let result = (|| {
            fs::create_dir_all(dir.join(SUBMIT_DIR)).map_err(storage_error)?;
            write_meta(&dir, &meta)?;
            fs::write(dir.join(BRIEF), brief).map_err(storage_error)?;
            for (source, name) in sources {
                if name.to_ascii_lowercase().ends_with(".zip") {
                    let stem = Path::new(&name)
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy();
                    unpack(&source, &dir, &stem)?;
                } else {
                    let target = dir.join(name);
                    if !target.exists() {
                        fs::copy(source, target).map_err(storage_error)?;
                    }
                }
            }
            Ok::<(), WorkspaceError>(())
        })();
        if let Err(error) = result {
            let _ = fs::remove_dir_all(&dir);
            return Err(error);
        }
        let meta = read_meta(&dir)
            .ok_or_else(|| WorkspaceError::Storage("Workspace metadata is missing".into()))?;
        Ok(info(&dir, &meta))
    }

    /// Assignments with files waiting to be handed in: a workspace whose
    /// `submit/` holds files, for an assignment LMS does not show as submitted
    /// or graded in `workspace` (the stored workspace fields Core reads). By assignment id.
    pub fn waiting(&self, workspace: &serde_json::Value) -> Vec<(String, usize)> {
        let done: std::collections::HashSet<&str> = workspace
            .get("assignments")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter(|row| {
                matches!(
                    row.get("submission").and_then(serde_json::Value::as_str),
                    Some("submitted" | "graded")
                )
            })
            .filter_map(|row| row.get("id").and_then(serde_json::Value::as_str))
            .collect();
        let tree = storage::workspace_root(&self.root);
        let mut waiting = Vec::new();
        for term in child_directories(&tree) {
            for course in child_directories(&term) {
                for dir in child_directories(&course) {
                    let Some(meta) = read_meta(&dir) else {
                        continue;
                    };
                    if done.contains(meta.assignment.as_str()) {
                        continue;
                    }
                    let files = submit_listing(&dir).map(|files| files.len()).unwrap_or(0);
                    if files > 0 {
                        waiting.push((meta.assignment, files));
                    }
                }
            }
        }
        waiting.sort();
        waiting
    }

    /// The files in an assignment's `submit/`, empty while it has no workspace.
    pub fn submit_files(
        &self,
        course: &CourseKey,
        assignment: &str,
    ) -> Result<Vec<LocalFile>, WorkspaceError> {
        match self.find(course, assignment)? {
            Some(info) => submit_listing(Path::new(&info.path)),
            None => Ok(Vec::new()),
        }
    }

    /// Copies files a person shared into the assignment's `submit/`, creating
    /// its workspace on first use. A name already taken gets a numbered one, so
    /// nothing is overwritten; the sources are left as they are.
    pub fn add_submit_files(
        &self,
        target: &HandInTarget,
        files: Vec<(PathBuf, String)>,
    ) -> Result<Vec<LocalFile>, HandInError> {
        let info = match self.find(&target.course, &target.assignment)? {
            Some(info) => info,
            None => self.create(
                &target.course,
                target.assignment.clone(),
                target.title.clone(),
                target.brief.clone(),
                Vec::new(),
            )?,
        };
        let dir = PathBuf::from(&info.path);
        let submit = dir.join(SUBMIT_DIR);
        fs::create_dir_all(&submit).map_err(storage_error)?;
        for (source, name) in files {
            let kind = fs::symlink_metadata(&source).map_err(storage_error)?;
            if !kind.is_file() {
                return Err(HandInError::FilesOnly);
            }
            let destination = unused_name(&submit, &safe_name(&name, "file"));
            fs::copy(&source, destination).map_err(storage_error)?;
        }
        Ok(submit_listing(&dir)?)
    }

    /// Removes one file from the assignment's `submit/`.
    pub fn remove_submit_file(
        &self,
        course: &CourseKey,
        assignment: &str,
        name: &str,
    ) -> Result<Vec<LocalFile>, WorkspaceError> {
        let info = self
            .find(course, assignment)?
            .ok_or(WorkspaceError::Missing)?;
        if safe_name(name, "") != name {
            return Err(WorkspaceError::Input(
                "Invalid submission file name.".into(),
            ));
        }
        let dir = PathBuf::from(&info.path);
        let path = dir.join(SUBMIT_DIR).join(name);
        if !fs::symlink_metadata(&path)
            .map_err(storage_error)?
            .is_file()
        {
            return Err(WorkspaceError::Input(
                "Invalid submission file name.".into(),
            ));
        }
        fs::remove_file(path).map_err(storage_error)?;
        submit_listing(&dir)
    }

    /// Validates a shell-provided workspace path before any native opener use.
    pub fn workspace_path(&self, path: &str) -> Result<PathBuf, WorkspaceError> {
        self.workspace_dir(path).map(|(path, _)| path)
    }

    pub fn set_submission(
        &self,
        path: &str,
        zip: bool,
        zip_name: &str,
    ) -> Result<WorkspaceInfo, WorkspaceError> {
        let (dir, mut meta) = self.workspace_dir(path)?;
        meta.zip = zip;
        meta.zip_name = zip_file_name(zip_name, &dir);
        write_meta(&dir, &meta)?;
        Ok(info(&dir, &meta))
    }

    pub fn prepare(&self, path: &str) -> Result<PreparedSubmission, HandInError> {
        let (dir, meta) = self.workspace_dir(path)?;
        let current = info(&dir, &meta);
        if current.submit_files.is_empty() {
            return Err(HandInError::Empty);
        }
        let submit = dir.join(SUBMIT_DIR);
        let target = if meta.zip {
            let output = dir.join(&current.zip_name);
            pack(&submit, &output)?;
            SubmissionTarget::Archive(output)
        } else {
            SubmissionTarget::Folder(submit)
        };
        Ok(PreparedSubmission {
            info: current,
            target,
        })
    }

    /// Read only the files explicitly placed in this assignment's submission area.
    pub fn prepare_upload(
        &self,
        path: &str,
        assignment: &str,
    ) -> Result<PreparedUpload, HandInError> {
        let (_, meta) = self.workspace_dir(path)?;
        if meta.assignment != assignment {
            return Err(WorkspaceError::Input(
                "This workspace belongs to a different assignment.".into(),
            )
            .into());
        }
        let prepared = self.prepare(path)?;
        let paths = match prepared.target {
            SubmissionTarget::Archive(path) => vec![path],
            SubmissionTarget::Folder(path) => prepared
                .info
                .submit_files
                .iter()
                .map(|name| path.join(name))
                .collect(),
        };
        if paths.len() > MAX_SUBMISSION_FILES {
            return Err(WorkspaceError::Input("Upload up to 32 files, or use ZIP.".into()).into());
        }
        let mut total = 0;
        let mut files = Vec::new();
        for path in paths {
            if fs::symlink_metadata(&path).map_err(storage_error)?.is_dir() {
                return Err(HandInError::NeedsZip);
            }
            let path = storage::stored_file(&self.root, &path.to_string_lossy())
                .map_err(WorkspaceError::from)?;
            let mut bytes = Vec::new();
            fs::File::open(&path)
                .map_err(storage_error)?
                .take(MAX_SUBMISSION_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(storage_error)?;
            total += bytes.len();
            if total > MAX_SUBMISSION_BYTES {
                return Err(HandInError::TooLarge);
            }
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| WorkspaceError::Input("Invalid submission file name.".into()))?
                .to_owned();
            files.push(SubmissionFile::from_bytes(name, &bytes));
        }
        Ok(PreparedUpload {
            info: prepared.info,
            files,
        })
    }

    /// Deleting a folder that is already gone succeeds.
    pub fn delete(&self, path: &str) -> Result<(), WorkspaceError> {
        let dir = match self.workspace_dir(path) {
            Ok((dir, _)) => dir,
            Err(WorkspaceError::Missing) => return Ok(()),
            Err(error) => return Err(error),
        };
        fs::remove_dir_all(dir).map_err(storage_error)
    }

    /// Removes workspaces with nothing changed for a month. Invalid folders,
    /// symlinks and trees too large to scan are retained.
    pub fn clean(&self) -> Result<usize, WorkspaceError> {
        let tree = storage::workspace_root(&self.root);
        if !tree.exists() {
            return Ok(0);
        }
        let tree = tree.canonicalize().map_err(storage_error)?;
        let now = SystemTime::now();
        let mut removed = 0;
        for term in child_directories(&tree) {
            for course in child_directories(&term) {
                for workspace in child_directories(&course) {
                    if read_meta(&workspace).is_none() {
                        continue;
                    }
                    let stale = last_change(&workspace)
                        .and_then(|time| now.duration_since(time).ok())
                        .is_some_and(|age| age >= STALE_AFTER);
                    if stale && fs::remove_dir_all(&workspace).is_ok() {
                        removed += 1;
                    }
                }
            }
        }
        Ok(removed)
    }

    fn workspaces_dir(&self, course: &CourseKey) -> Result<PathBuf, WorkspaceError> {
        storage::storage_dir(
            &storage::workspace_root(&self.root),
            &[&course.term, &course.course],
        )
        .map_err(WorkspaceError::from)
    }

    fn workspace_dir(&self, path: &str) -> Result<(PathBuf, Meta), WorkspaceError> {
        let root = storage::workspace_root(&self.root)
            .canonicalize()
            .map_err(lookup_error)?;
        let raw = PathBuf::from(path);
        let relative = raw.strip_prefix(&root).map_err(|_| not_workspace())?;
        let mut checked = root.clone();
        for component in relative.components() {
            let Component::Normal(component) = component else {
                return Err(not_workspace());
            };
            checked.push(component);
            if fs::symlink_metadata(&checked)
                .map_err(lookup_error)?
                .file_type()
                .is_symlink()
            {
                return Err(not_workspace());
            }
        }
        let dir = raw.canonicalize().map_err(lookup_error)?;
        let inside = dir
            .strip_prefix(&root)
            .ok()
            .map(|rest| rest.components().count());
        // `<term>/<course>/<assignment>` inside the workspace tree.
        if inside != Some(3) {
            return Err(not_workspace());
        }
        let meta = read_meta(&dir).ok_or_else(not_workspace)?;
        Ok((dir, meta))
    }
}

fn read_meta(dir: &Path) -> Option<Meta> {
    serde_json::from_slice(&fs::read(dir.join(META)).ok()?).ok()
}

fn write_meta(dir: &Path, meta: &Meta) -> Result<(), WorkspaceError> {
    let json = serde_json::to_vec_pretty(meta)
        .map_err(|error| WorkspaceError::Storage(error.to_string()))?;
    fs::write(dir.join(META), json).map_err(storage_error)
}

fn info(dir: &Path, meta: &Meta) -> WorkspaceInfo {
    let mut submit_files: Vec<String> = fs::read_dir(dir.join(SUBMIT_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            (!entry.file_type().ok()?.is_symlink())
                .then(|| entry.file_name().to_string_lossy().into_owned())
        })
        .filter(|name| !is_junk(name))
        .collect();
    submit_files.sort();
    WorkspaceInfo {
        path: dir.to_string_lossy().into_owned(),
        name: dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        submit_files,
        zip: meta.zip,
        zip_name: if meta.zip_name.is_empty() {
            default_zip_name(dir)
        } else {
            meta.zip_name.clone()
        },
    }
}

/// One assignment as a phone hands files to it: its course folder, title and
/// the `ASSIGNMENT.md` its workspace starts with, read from Core's
/// `AssignmentDetail`.
pub struct HandInTarget {
    pub course: CourseKey,
    pub assignment: String,
    pub title: String,
    pub brief: String,
}

impl HandInTarget {
    /// `None` while the assignment's course folder is not known yet.
    pub fn from_detail(detail: &AssignmentDetail) -> Option<Self> {
        let facts = [
            detail.due_at.as_ref().map(|due| format!("- Due: {due}")),
            detail
                .source_url
                .as_ref()
                .map(|url| format!("- LMS: {url}")),
            Some("- Hand in: put the files to submit in `submit/`".into()),
        ];
        let mut brief = format!(
            "# {}\n\n{}\n",
            detail.title,
            facts.into_iter().flatten().collect::<Vec<_>>().join("\n")
        );
        if !detail.details.trim().is_empty() {
            brief.push_str(&format!("\n{}\n", detail.details.trim()));
        }
        Some(Self {
            course: detail.files.clone()?,
            assignment: detail.assignment_id.clone(),
            title: detail.title.clone(),
            brief,
        })
    }
}

/// The regular files of a workspace's `submit/`, by name.
fn submit_listing(dir: &Path) -> Result<Vec<LocalFile>, WorkspaceError> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir.join(SUBMIT_DIR)).map_err(storage_error)? {
        let entry = entry.map_err(storage_error)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_junk(&name) || !entry.file_type().map_err(storage_error)?.is_file() {
            continue;
        }
        files.push(storage::describe(
            entry.path(),
            name,
            String::new(),
            true,
            false,
        )?);
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(files)
}

/// `name` in `dir`, or `name (2)`, `name (3)`… before its extension when taken.
fn unused_name(dir: &Path, name: &str) -> PathBuf {
    let path = Path::new(name);
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let extension = path
        .extension()
        .map(|extension| format!(".{}", extension.to_string_lossy()))
        .unwrap_or_default();
    (1..)
        .map(|index| match index {
            1 => dir.join(name),
            _ => dir.join(format!("{stem} ({index}){extension}")),
        })
        .find(|candidate| fs::symlink_metadata(candidate).is_err())
        .unwrap_or_else(|| dir.join(name))
}

fn default_zip_name(dir: &Path) -> String {
    format!(
        "{}.zip",
        dir.file_name().unwrap_or_default().to_string_lossy()
    )
}

fn is_junk(name: &str) -> bool {
    name == ".DS_Store" || name == "__MACOSX" || name == "Thumbs.db"
}

fn safe_name(raw: &str, fallback: &str) -> String {
    let name: String = raw
        .chars()
        .map(|character| {
            if character.is_control() || "/\\:*?\"<>|".contains(character) {
                '_'
            } else {
                character
            }
        })
        .take(80)
        .collect();
    let name = name
        .trim()
        .trim_end_matches(['.', ' '])
        .trim_start_matches('.');
    if name.is_empty() {
        fallback.into()
    } else {
        name.into()
    }
}

fn zip_file_name(raw: &str, dir: &Path) -> String {
    let name = safe_name(raw, "");
    if name.is_empty() {
        default_zip_name(dir)
    } else if name.to_ascii_lowercase().ends_with(".zip") {
        name
    } else {
        format!("{name}.zip")
    }
}

fn find(dir: &Path, assignment: &str) -> Option<(PathBuf, Meta)> {
    fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        let path = entry.path();
        if path.is_symlink() || !path.is_dir() {
            return None;
        }
        read_meta(&path)
            .filter(|meta| meta.assignment == assignment)
            .map(|meta| (path, meta))
    })
}

fn reserve_workspace(parent: &Path, base: &str) -> Result<PathBuf, WorkspaceError> {
    for index in 1..=u32::MAX {
        let path = if index == 1 {
            parent.join(base)
        } else {
            parent.join(format!("{base} ({index})"))
        };
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(storage_error(error)),
        }
    }
    Err(WorkspaceError::Storage(
        "No unused assignment workspace name is available".into(),
    ))
}

/// Without ZIP support (the phones), a workspace meant as an archive is
/// refused rather than handed in some other way.
#[cfg(not(feature = "workspace-archives"))]
fn no_archives() -> WorkspaceError {
    WorkspaceError::Archive("ZIP archives are handled on the desktop.".into())
}

#[cfg(not(feature = "workspace-archives"))]
fn unpack(_: &Path, _: &Path, _: &str) -> Result<(), WorkspaceError> {
    Err(no_archives())
}

#[cfg(not(feature = "workspace-archives"))]
fn pack(_: &Path, _: &Path) -> Result<(), WorkspaceError> {
    Err(no_archives())
}

#[cfg(feature = "workspace-archives")]
fn unpack(archive: &Path, dir: &Path, stem: &str) -> Result<(), WorkspaceError> {
    let file = fs::File::open(archive).map_err(storage_error)?;
    let mut zip = zip::ZipArchive::new(file).map_err(archive_error)?;
    if zip.len() > MAX_ENTRIES {
        return Err(WorkspaceError::Archive(
            "The archive has too many entries".into(),
        ));
    }
    let mut entries = Vec::new();
    for index in 0..zip.len() {
        let entry = zip.by_index(index).map_err(archive_error)?;
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let junk = name
            .components()
            .any(|component| matches!(component, Component::Normal(part) if is_junk(&part.to_string_lossy())));
        if !junk && !entry.is_symlink() {
            entries.push((index, name, entry.is_dir()));
        }
    }
    let top: Vec<_> = entries
        .iter()
        .filter_map(|(_, name, _)| name.components().next())
        .collect();
    let single_folder = top
        .first()
        .is_some_and(|first| top.iter().all(|component| component == first))
        && entries
            .iter()
            .any(|(_, name, _)| name.components().count() > 1);
    let target = if single_folder {
        dir.to_path_buf()
    } else {
        dir.join(safe_name(stem, "attachment"))
    };
    let mut total = 0_u64;
    for (index, name, is_dir) in entries {
        let path = target.join(name);
        if is_dir {
            fs::create_dir_all(path).map_err(storage_error)?;
            continue;
        }
        if path.exists() {
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(storage_error)?;
        }
        let mut entry = zip.by_index(index).map_err(archive_error)?;
        let mut output = fs::File::create(&path).map_err(storage_error)?;
        let budget = MAX_UNPACK_BYTES.saturating_sub(total);
        let written =
            io::copy(&mut (&mut entry).take(budget + 1), &mut output).map_err(storage_error)?;
        total = total.saturating_add(written);
        if written > budget {
            return Err(WorkspaceError::Archive(
                "The archive is too large to unpack".into(),
            ));
        }
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o755));
        }
    }
    Ok(())
}

#[cfg(feature = "workspace-archives")]
fn pack(source: &Path, output: &Path) -> Result<(), WorkspaceError> {
    use std::io::Write;
    let partial = output.with_extension("zip.partial");
    let result = (|| {
        let file = fs::File::create(&partial).map_err(storage_error)?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .large_file(true);
        let mut pending = vec![source.to_path_buf()];
        let mut count = 0;
        while let Some(dir) = pending.pop() {
            let mut children: Vec<_> = fs::read_dir(&dir)
                .map_err(storage_error)?
                .flatten()
                .collect();
            children.sort_by_key(|entry| entry.file_name());
            for entry in children {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if is_junk(&name) || path.is_symlink() {
                    continue;
                }
                count += 1;
                if count > MAX_ENTRIES {
                    return Err(WorkspaceError::Archive(
                        "The submit folder has too many files".into(),
                    ));
                }
                let relative = path
                    .strip_prefix(source)
                    .map_err(storage_error)?
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                if path.is_dir() {
                    zip.add_directory(relative, options)
                        .map_err(archive_error)?;
                    pending.push(path);
                } else {
                    zip.start_file(relative, options).map_err(archive_error)?;
                    io::copy(&mut fs::File::open(path).map_err(storage_error)?, &mut zip)
                        .map_err(storage_error)?;
                }
            }
        }
        zip.finish()
            .map_err(archive_error)?
            .flush()
            .map_err(storage_error)?;
        fs::rename(&partial, output).map_err(storage_error)
    })();
    if result.is_err() {
        let _ = fs::remove_file(partial);
    }
    result
}

fn child_directories(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && !path.is_symlink())
        .collect()
}

fn last_change(dir: &Path) -> Option<SystemTime> {
    let mut newest = dir.metadata().ok()?.modified().ok()?;
    let mut pending = vec![dir.to_path_buf()];
    let mut seen = 0;
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(dir).ok()?.flatten() {
            seen += 1;
            if seen > MAX_ENTRIES {
                return None;
            }
            let Ok(metadata) = entry.path().symlink_metadata() else {
                continue;
            };
            if let Ok(time) = metadata.modified() {
                newest = newest.max(time);
            }
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                pending.push(entry.path());
            }
        }
    }
    Some(newest)
}

fn not_workspace() -> WorkspaceError {
    WorkspaceError::Input("Folder is not an assignment workspace".into())
}

/// A workspace path that no longer resolves was deleted behind the app.
fn lookup_error(error: io::Error) -> WorkspaceError {
    if error.kind() == io::ErrorKind::NotFound {
        WorkspaceError::Missing
    } else {
        storage_error(error)
    }
}

fn storage_error(error: impl std::fmt::Display + 'static) -> WorkspaceError {
    WorkspaceError::Storage(storage::failure_text(error))
}

#[cfg(feature = "workspace-archives")]
fn archive_error(error: impl ToString) -> WorkspaceError {
    WorkspaceError::Archive(error.to_string())
}

#[cfg(test)]
#[path = "tests/workspaces.rs"]
mod tests;
