//! Assignment workspaces (desktop only): one folder per assignment holding the
//! unpacked attachments, the student's own work, and a `submit/` folder whose
//! contents are what gets handed in — one by one, or packed into a named zip.
//!
//! Workspaces live in `<term>/<course>/workspaces/`, apart from `materials/`,
//! so the material auto-clean never touches them. They are removed only by
//! hand or by the explicit one-month cleanup.
use crate::lms_files::{root, storage_dir, stored_file, CourseKey};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime},
};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

const META: &str = ".workspace.json";
const SUBMIT_DIR: &str = "submit";
const BRIEF: &str = "ASSIGNMENT.md";
const MAX_UNPACK_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 50_000;
const STALE_AFTER: Duration = Duration::from_secs(30 * 24 * 60 * 60);

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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfo {
    path: String,
    name: String,
    /// Names directly inside `submit/`.
    submit_files: Vec<String>,
    zip: bool,
    zip_name: String,
}

/// An attachment already downloaded into the course folder.
#[derive(Deserialize)]
pub struct Attachment {
    path: String,
    name: String,
}

fn workspaces_dir(app: &AppHandle, course: &CourseKey) -> Result<PathBuf, String> {
    storage_dir(app, &[&course.term, &course.course, "workspaces"])
}

fn read_meta(dir: &Path) -> Option<Meta> {
    serde_json::from_slice(&fs::read(dir.join(META)).ok()?).ok()
}

fn write_meta(dir: &Path, meta: &Meta) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(meta).map_err(|e| e.to_string())?;
    fs::write(dir.join(META), json).map_err(|e| e.to_string())
}

/// A workspace folder the app created under the storage root; anything else is refused.
fn workspace_dir(app: &AppHandle, path: &str) -> Result<(PathBuf, Meta), String> {
    let (root, _) = root(app)?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let dir = PathBuf::from(path)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let inside = dir
        .strip_prefix(&root)
        .ok()
        .map(|rest| rest.components().count());
    // <term>/<course>/workspaces/<name>
    if inside != Some(4) || dir.parent().and_then(Path::file_name) != Some("workspaces".as_ref()) {
        return Err("Folder is not an assignment workspace".into());
    }
    let meta = read_meta(&dir).ok_or("Folder is not an assignment workspace")?;
    Ok((dir, meta))
}

fn info(dir: &Path, meta: &Meta) -> WorkspaceInfo {
    let mut submit_files: Vec<String> = fs::read_dir(dir.join(SUBMIT_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
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

fn default_zip_name(dir: &Path) -> String {
    format!(
        "{}.zip",
        dir.file_name().unwrap_or_default().to_string_lossy()
    )
}

fn is_junk(name: &str) -> bool {
    name == ".DS_Store" || name == "__MACOSX" || name == "Thumbs.db"
}

/// A folder or file name safe on every desktop file system.
fn safe_name(raw: &str, fallback: &str) -> String {
    let name: String = raw
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '_'
            } else {
                c
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

#[tauri::command]
pub async fn assignment_workspace(
    webview: tauri::Webview,
    app: AppHandle,
    context: CourseKey,
    assignment: String,
) -> Result<Option<WorkspaceInfo>, String> {
    crate::require_local_main(&webview)?;
    let dir = workspaces_dir(&app, &context)?;
    Ok(find(&dir, &assignment).map(|(path, meta)| info(&path, &meta)))
}

/// Create the workspace once: unpack zip attachments, copy the rest, write the
/// assignment brief and an empty `submit/`. Existing files are never overwritten.
#[tauri::command]
pub async fn create_assignment_workspace(
    webview: tauri::Webview,
    app: AppHandle,
    context: CourseKey,
    assignment: String,
    title: String,
    brief: String,
    attachments: Vec<Attachment>,
) -> Result<WorkspaceInfo, String> {
    crate::require_local_main(&webview)?;
    let parent = workspaces_dir(&app, &context)?;
    if let Some((path, meta)) = find(&parent, &assignment) {
        return Ok(info(&path, &meta));
    }
    let sources = attachments
        .into_iter()
        .map(|a| {
            Ok((
                stored_file(&app, &a.path)?,
                safe_name(&a.name, "attachment"),
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let base = safe_name(&title, "Assignment");
    let dir = (1..)
        .map(|n| {
            if n == 1 {
                parent.join(&base)
            } else {
                parent.join(format!("{base} ({n})"))
            }
        })
        .find(|path| !path.exists())
        .expect("an unused name exists");
    let meta = Meta {
        assignment,
        title,
        zip: false,
        zip_name: String::new(),
    };
    let created = dir.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        fs::create_dir_all(dir.join(SUBMIT_DIR)).map_err(|e| e.to_string())?;
        write_meta(&dir, &meta)?;
        fs::write(dir.join(BRIEF), brief).map_err(|e| e.to_string())?;
        for (source, name) in sources {
            let is_zip = name.to_ascii_lowercase().ends_with(".zip");
            if is_zip {
                let stem = Path::new(&name)
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy();
                unpack(&source, &dir, &stem)?;
            } else {
                let target = dir.join(&name);
                if !target.exists() {
                    fs::copy(&source, &target).map_err(|e| e.to_string())?;
                }
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
    .inspect_err(|_| {
        let _ = fs::remove_dir_all(&created);
    })?;
    let meta = read_meta(&created).ok_or("Workspace metadata is missing")?;
    Ok(info(&created, &meta))
}

/// Unpack `archive` into `dir`. An archive with one top-level folder unpacks
/// as that folder; anything else goes into a folder named after the archive.
fn unpack(archive: &Path, dir: &Path, stem: &str) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    if zip.len() > MAX_ENTRIES {
        return Err("The archive has too many entries".into());
    }
    let mut entries = Vec::new();
    for index in 0..zip.len() {
        let entry = zip.by_index(index).map_err(|e| e.to_string())?;
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let junk = name
            .components()
            .any(|c| matches!(c, Component::Normal(part) if is_junk(&part.to_string_lossy())));
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
        .is_some_and(|first| top.iter().all(|c| c == first))
        && entries
            .iter()
            .any(|(_, name, _)| name.components().count() > 1);
    let target = if single_folder {
        dir.to_path_buf()
    } else {
        dir.join(safe_name(stem, "attachment"))
    };
    let mut total = 0u64;
    for (index, name, is_dir) in entries {
        let path = target.join(&name);
        if is_dir {
            fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            continue;
        }
        if path.exists() {
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
        let mut out = fs::File::create(&path).map_err(|e| e.to_string())?;
        // The declared size can lie; count what is actually written.
        let budget = MAX_UNPACK_BYTES.saturating_sub(total);
        let written =
            io::copy(&mut (&mut entry).take(budget + 1), &mut out).map_err(|e| e.to_string())?;
        total += written;
        if written > budget {
            return Err("The archive is too large to unpack".into());
        }
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(mode & 0o755));
        }
    }
    Ok(())
}

/// `finder`: the workspace in the file manager; `vscode`: the whole folder in VS Code.
#[tauri::command]
pub async fn open_workspace(
    webview: tauri::Webview,
    app: AppHandle,
    path: String,
    target: String,
) -> Result<(), String> {
    crate::require_local_main(&webview)?;
    let (dir, _) = workspace_dir(&app, &path)?;
    match target.as_str() {
        "finder" => app.opener().open_path(dir.to_string_lossy(), None::<&str>),
        // VS Code's URL handler opens a folder without needing `code` on PATH.
        "vscode" => {
            let url = url::Url::from_directory_path(&dir).map_err(|_| "Invalid workspace path")?;
            let url = format!("vscode://file{}", url.path().trim_end_matches('/'));
            app.opener().open_url(url, None::<&str>)
        }
        _ => return Err("Unknown target".into()),
    }
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_workspace_submission(
    webview: tauri::Webview,
    app: AppHandle,
    path: String,
    zip: bool,
    zip_name: String,
) -> Result<WorkspaceInfo, String> {
    crate::require_local_main(&webview)?;
    let (dir, mut meta) = workspace_dir(&app, &path)?;
    meta.zip = zip;
    meta.zip_name = zip_file_name(&zip_name, &dir);
    write_meta(&dir, &meta)?;
    Ok(info(&dir, &meta))
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

/// Make `submit/` ready to hand in and show it: the folder itself, or the zip
/// packed from it next to the folder.
#[tauri::command]
pub async fn prepare_workspace_submission(
    webview: tauri::Webview,
    app: AppHandle,
    path: String,
) -> Result<WorkspaceInfo, String> {
    crate::require_local_main(&webview)?;
    let (dir, meta) = workspace_dir(&app, &path)?;
    let current = info(&dir, &meta);
    if current.submit_files.is_empty() {
        return Err("The submit folder is empty".into());
    }
    let submit = dir.join(SUBMIT_DIR);
    if !meta.zip {
        app.opener()
            .open_path(submit.to_string_lossy(), None::<&str>)
            .map_err(|e| e.to_string())?;
        return Ok(current);
    }
    let output = dir.join(&current.zip_name);
    let packed = output.clone();
    tauri::async_runtime::spawn_blocking(move || pack(&submit, &packed))
        .await
        .map_err(|e| e.to_string())??;
    app.opener()
        .reveal_item_in_dir(&output)
        .map_err(|e| e.to_string())?;
    Ok(current)
}

fn pack(source: &Path, output: &Path) -> Result<(), String> {
    let partial = output.with_extension("zip.partial");
    let file = fs::File::create(&partial).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);
    let mut pending = vec![source.to_path_buf()];
    let mut count = 0;
    while let Some(dir) = pending.pop() {
        let mut children: Vec<_> = fs::read_dir(&dir)
            .map_err(|e| e.to_string())?
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
                return Err("The submit folder has too many files".into());
            }
            let relative = path
                .strip_prefix(source)
                .map_err(|e| e.to_string())?
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            if path.is_dir() {
                zip.add_directory(relative, options)
                    .map_err(|e| e.to_string())?;
                pending.push(path);
            } else {
                zip.start_file(relative, options)
                    .map_err(|e| e.to_string())?;
                io::copy(
                    &mut fs::File::open(&path).map_err(|e| e.to_string())?,
                    &mut zip,
                )
                .map_err(|e| e.to_string())?;
            }
        }
    }
    zip.finish()
        .map_err(|e| e.to_string())?
        .flush()
        .map_err(|e| e.to_string())?;
    fs::rename(&partial, output).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_workspace(
    webview: tauri::Webview,
    app: AppHandle,
    path: String,
) -> Result<(), String> {
    crate::require_local_main(&webview)?;
    let (dir, _) = workspace_dir(&app, &path)?;
    tauri::async_runtime::spawn_blocking(move || fs::remove_dir_all(dir))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Remove workspaces with nothing changed for a month. Only run on request.
#[tauri::command]
pub async fn clean_workspaces(webview: tauri::Webview, app: AppHandle) -> Result<usize, String> {
    crate::require_local_main(&webview)?;
    let (root, _) = root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let now = SystemTime::now();
        let mut removed = 0;
        let children = |dir: &Path| -> Vec<PathBuf> {
            fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir() && !path.is_symlink())
                .collect()
        };
        for term in children(&root) {
            for course in children(&term) {
                for workspace in children(&course.join("workspaces")) {
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
        removed
    })
    .await
    .map_err(|e| e.to_string())
}

/// The newest modification anywhere inside `dir`.
fn last_change(dir: &Path) -> Option<SystemTime> {
    let mut newest = dir.metadata().ok()?.modified().ok()?;
    let mut pending = vec![dir.to_path_buf()];
    let mut seen = 0;
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).ok()?.flatten() {
            seen += 1;
            if seen > MAX_ENTRIES {
                // Too large to scan fully: treat as recently used rather than guess.
                return None;
            }
            let Ok(meta) = entry.path().symlink_metadata() else {
                continue;
            };
            if let Ok(time) = meta.modified() {
                newest = newest.max(time);
            }
            if meta.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    Some(newest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("workspace-test-{name}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn archive(path: &Path, files: &[(&str, &str)]) {
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for (name, body) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn names_are_safe_and_zip_names_get_an_extension() {
        assert_eq!(safe_name("OOP 1B: A/B?", "x"), "OOP 1B_ A_B_");
        assert_eq!(safe_name(" .. ", "x"), "x");
        assert_eq!(safe_name("..hidden", "x"), "hidden");
        let dir = Path::new("/tmp/Report 3");
        assert_eq!(zip_file_name("", dir), "Report 3.zip");
        assert_eq!(zip_file_name("24B12345", dir), "24B12345.zip");
        assert_eq!(zip_file_name("a.ZIP", dir), "a.ZIP");
    }

    #[test]
    fn unpacks_a_single_folder_in_place_and_skips_junk_and_traversal() {
        let dir = temp("unpack");
        let source = dir.join("src.zip");
        archive(
            &source,
            &[
                ("problem/Main.java", "class Main {}"),
                ("__MACOSX/problem/._Main.java", "x"),
                ("../evil", "x"),
            ],
        );
        let out = dir.join("ws");
        fs::create_dir_all(&out).unwrap();
        unpack(&source, &out, "src").unwrap();
        assert_eq!(
            fs::read_to_string(out.join("problem/Main.java")).unwrap(),
            "class Main {}"
        );
        assert!(!out.join("__MACOSX").exists());
        assert!(!dir.join("evil").exists());

        let loose = dir.join("loose.zip");
        archive(&loose, &[("a.txt", "a"), ("b.txt", "b")]);
        unpack(&loose, &out, "loose").unwrap();
        assert!(out.join("loose/a.txt").is_file());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn packs_the_submit_folder_recursively() {
        let dir = temp("pack");
        let submit = dir.join("submit");
        fs::create_dir_all(submit.join("src")).unwrap();
        fs::write(submit.join("report.pdf"), "pdf").unwrap();
        fs::write(submit.join("src/Main.java"), "java").unwrap();
        fs::write(submit.join(".DS_Store"), "junk").unwrap();
        let output = dir.join("out.zip");
        pack(&submit, &output).unwrap();
        let mut zip = zip::ZipArchive::new(fs::File::open(&output).unwrap()).unwrap();
        let mut names: Vec<_> = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().unwrap().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["report.pdf", "src/", "src/Main.java"]);
        fs::remove_dir_all(dir).unwrap();
    }
}
