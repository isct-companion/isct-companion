use crate::{
    coded, commands::cloud::SystemBrowser, require_local_main, secrets::OsSecretVault, shell,
    CommandError,
};
use app_core::{
    exam_share::{ExamAction, ExamAnswer, ExamError, ExamSignIn, ExamUploadFile},
    oauth::code_exchange,
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tauri::{Manager, Webview};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Open Core's ScienceTokyoExam client with the system browser for sign-in.
pub(crate) fn open(app: &tauri::AppHandle, vault: OsSecretVault) {
    let sign_in = code_exchange::installed_client().map(|client| ExamSignIn {
        client,
        browser: Arc::new(SystemBrowser(app.clone())),
    });
    crate::core_ports::core(app).open_exam_share(
        Arc::new(vault),
        sign_in,
        app.package_info().version.to_string(),
    );
}

#[tauri::command]
pub(crate) async fn exam_share(
    webview: Webview,
    selection: tauri::State<'_, UploadSelection>,
    mut action: ExamAction,
) -> Result<ExamAnswer, CommandError> {
    require_local_main(&webview)?;
    let _selected_files = match &mut action {
        ExamAction::Upload { files, .. } => Some(selection.resolve(files).map_err(coded)?),
        _ => None,
    };
    let cancel = CancellationToken::new();
    let _guard = cancel.clone().drop_guard();
    let core = crate::core_ports::core(webview.app_handle());
    let answer = core.exam_share(action, cancel).await.map_err(coded)?;
    if let ExamAnswer::Document(file) = &answer {
        webview
            .app_handle()
            .asset_protocol_scope()
            .allow_file(&file.path)
            .map_err(|_| coded(ExamError::Storage))?;
    }
    Ok(answer)
}

/// The renderer receives names and grants from the picker. Only the native
/// side knows the paths, and only the current selection is available.
#[derive(Default)]
pub(crate) struct UploadSelection(Mutex<Vec<SelectedUploadFile>>);

#[derive(Clone)]
struct SelectedUploadFile {
    grant: String,
    file: ExamUploadFile,
    // A converted HEIC stays on disk only while selected or being uploaded.
    _converted: Option<Arc<ConvertedUploadFile>>,
}

impl UploadSelection {
    fn resolve(&self, files: &mut [ExamUploadFile]) -> Result<Vec<SelectedUploadFile>, ExamError> {
        let selection = self.0.lock().map_err(|_| ExamError::Storage)?;
        files
            .iter_mut()
            .map(|file| {
                let selected = selection
                    .iter()
                    .find(|selected| selected.grant == file.path)
                    .ok_or(ExamError::InvalidInput)?;
                *file = selected.file.clone();
                Ok(selected.clone())
            })
            .collect()
    }
}

#[tauri::command]
pub(crate) async fn exam_upload_pick_files(
    webview: Webview,
    selection: tauri::State<'_, UploadSelection>,
    title: String,
) -> Result<Option<Vec<ExamUploadFile>>, CommandError> {
    require_local_main(&webview)?;
    let Some(paths) = crate::native_dialog::pick_files(
        &webview.window(),
        title,
        &["pdf", "jpg", "jpeg", "png", "webp", "heic", "heif", "txt"],
    )
    .await?
    else {
        return Ok(None);
    };
    let files = tokio::task::spawn_blocking(move || {
        paths
            .into_iter()
            .map(SelectedUploadFile::new)
            .collect::<Result<Vec<_>, _>>()
    })
    .await
    .map_err(|_| coded(ExamError::Storage))?
    .map_err(coded)?;
    let grants = files
        .iter()
        .map(|selected| ExamUploadFile {
            path: selected.grant.clone(),
            name: selected.file.name.clone(),
        })
        .collect();
    *selection.0.lock().map_err(|_| coded(ExamError::Storage))? = files;
    Ok(Some(grants))
}

#[tauri::command]
pub(crate) fn exam_upload_clear_files(
    webview: Webview,
    selection: tauri::State<'_, UploadSelection>,
) -> Result<(), CommandError> {
    require_local_main(&webview)?;
    selection
        .0
        .lock()
        .map_err(|_| coded(ExamError::Storage))?
        .clear();
    Ok(())
}

/// Saves a copy of an opened past exam in the Downloads folder under its
/// uploaded name and returns where. Core keeps the reader's copy in its own
/// folder under a generated name, which nobody can find, so a download
/// needs this file too. Only a file Core opened this session, granted to
/// the asset protocol for the reader, is copied.
#[tauri::command]
pub(crate) async fn exam_share_save_copy(
    webview: Webview,
    path: String,
    name: String,
) -> Result<String, CommandError> {
    require_local_main(&webview)?;
    let app = webview.app_handle();
    let source = PathBuf::from(path);
    if !source.is_file() || !app.asset_protocol_scope().is_allowed(&source) {
        return Err(coded(ExamError::InvalidInput));
    }
    let folder = app.path().download_dir().map_err(shell)?;
    let name = download_name(&name);
    tauri::async_runtime::spawn_blocking(move || copy_new(&source, &folder, &name))
        .await
        .map_err(shell)?
        .map(|saved| saved.to_string_lossy().into_owned())
        .map_err(shell)
}

/// Shows a copy `exam_share_save_copy` made in the Downloads folder; any
/// other path is refused.
#[tauri::command]
pub(crate) fn exam_share_reveal(webview: Webview, path: String) -> Result<(), CommandError> {
    use tauri_plugin_opener::OpenerExt;
    require_local_main(&webview)?;
    let app = webview.app_handle();
    let path = PathBuf::from(path);
    let folder = app.path().download_dir().map_err(shell)?;
    if path.parent() != Some(folder.as_path()) || !path.is_file() {
        return Err(coded(ExamError::InvalidInput));
    }
    app.opener().reveal_item_in_dir(&path).map_err(shell)
}

/// The uploaded name as one file name every desktop system accepts.
fn download_name(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    // Windows drops trailing dots and spaces; a leading dot hides the file.
    let cleaned = cleaned.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if cleaned.is_empty() {
        "document".into()
    } else {
        cleaned.to_owned()
    }
}

/// Copies `source` into `folder` as `name`, or `name (1)`, `name (2)`…
/// when that is taken: an existing file is never replaced.
fn copy_new(source: &Path, folder: &Path, name: &str) -> std::io::Result<PathBuf> {
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, Some(extension)),
        _ => (name, None),
    };
    for attempt in 0..1000 {
        let candidate = match (attempt, extension) {
            (0, _) => name.to_owned(),
            (_, Some(extension)) => format!("{stem} ({attempt}).{extension}"),
            (_, None) => format!("{stem} ({attempt})"),
        };
        let target = folder.join(candidate);
        let mut output = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let copied = std::fs::File::open(source)
            .and_then(|mut input| std::io::copy(&mut input, &mut output));
        drop(output);
        if let Err(error) = copied {
            let _ = std::fs::remove_file(&target);
            return Err(error);
        }
        return Ok(target);
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "no free file name",
    ))
}

impl SelectedUploadFile {
    fn new(source: PathBuf) -> Result<Self, ExamError> {
        let mut name = source
            .file_name()
            .ok_or(ExamError::InvalidInput)?
            .to_string_lossy()
            .into_owned();
        let converted = if source
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("heic") || extension.eq_ignore_ascii_case("heif")
            }) {
            let scratch = Arc::new(ConvertedUploadFile::new()?);
            convert_heic(&source, &scratch.path())?;
            name = Path::new(&name)
                .with_extension("jpg")
                .to_string_lossy()
                .into_owned();
            Some(scratch)
        } else {
            None
        };
        let path = converted.as_ref().map_or(source, |file| file.path());
        let path = path
            .into_os_string()
            .into_string()
            .map_err(|_| ExamError::InvalidInput)?;
        Ok(Self {
            grant: Uuid::new_v4().to_string(),
            file: ExamUploadFile { path, name },
            _converted: converted,
        })
    }
}

struct ConvertedUploadFile(PathBuf);

impl ConvertedUploadFile {
    fn new() -> Result<Self, ExamError> {
        let dir = std::env::temp_dir().join(format!("isct-exam-upload-{}", Uuid::new_v4()));
        std::fs::create_dir(&dir).map_err(|_| ExamError::Storage)?;
        Ok(Self(dir))
    }

    fn path(&self) -> PathBuf {
        self.0.join("photo.jpg")
    }
}

impl Drop for ConvertedUploadFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(target_os = "macos")]
fn convert_heic(source: &Path, destination: &Path) -> Result<(), ExamError> {
    // sips uses the system ImageIO decoder, including HEIF.
    let status = std::process::Command::new("/usr/bin/sips")
        .args([
            "-s",
            "format",
            "jpeg",
            "-s",
            "formatOptions",
            "90",
            "-Z",
            "4000",
        ])
        .arg(source)
        .arg("--out")
        .arg(destination)
        .output()
        .map_err(|_| ExamError::Storage)?
        .status;
    status
        .success()
        .then_some(())
        .ok_or(ExamError::InvalidInput)
}

#[cfg(target_os = "windows")]
fn convert_heic(source: &Path, destination: &Path) -> Result<(), ExamError> {
    use windows::{
        core::HSTRING,
        Graphics::Imaging::{
            BitmapAlphaMode, BitmapDecoder, BitmapEncoder, BitmapPixelFormat, BitmapTransform,
            ColorManagementMode, ExifOrientationMode,
        },
        Storage::{FileAccessMode, StorageFile},
        Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
    };

    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            // SAFETY: this balances RoInitialize on the same blocking thread.
            unsafe { RoUninitialize() }
        }
    }

    std::fs::File::create(destination).map_err(|_| ExamError::Storage)?;
    let convert = || -> windows::core::Result<()> {
        // SAFETY: this blocking thread owns and balances its WinRT initialization.
        unsafe { RoInitialize(RO_INIT_MULTITHREADED)? };
        let _apartment = Apartment;
        let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(source))?.join()?;
        let input = file.OpenAsync(FileAccessMode::Read)?.join()?;
        let decoder = BitmapDecoder::CreateAsync(&input)?.join()?;
        let transform = BitmapTransform::new()?;
        let (width, height) = (decoder.PixelWidth()?, decoder.PixelHeight()?);
        let longest = width.max(height);
        if longest > 4000 {
            transform
                .SetScaledWidth(((u64::from(width) * 4000 / u64::from(longest)) as u32).max(1))?;
            transform
                .SetScaledHeight(((u64::from(height) * 4000 / u64::from(longest)) as u32).max(1))?;
        }
        let bitmap = decoder
            .GetSoftwareBitmapTransformedAsync(
                BitmapPixelFormat::Bgra8,
                BitmapAlphaMode::Ignore,
                &transform,
                ExifOrientationMode::RespectExifOrientation,
                ColorManagementMode::ColorManageToSRgb,
            )?
            .join()?;
        let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(destination))?.join()?;
        let output = file.OpenAsync(FileAccessMode::ReadWrite)?.join()?;
        let encoder =
            BitmapEncoder::CreateAsync(BitmapEncoder::JpegEncoderId()?, &output)?.join()?;
        encoder.SetSoftwareBitmap(&bitmap)?;
        encoder.FlushAsync()?.join()
    };
    // Windows exposes HEIF decoding through the codecs installed on the device.
    convert().map_err(|_| ExamError::InvalidInput)
}

#[cfg(test)]
mod tests {
    use super::{copy_new, download_name};

    #[test]
    fn download_names_are_one_safe_file_name() {
        assert_eq!(download_name("期末 2019.md"), "期末 2019.md");
        assert_eq!(download_name("a/b\\c:d?.txt"), "a_b_c_d_.txt");
        assert_eq!(download_name(" .hidden. "), "hidden");
        assert_eq!(download_name("..."), "document");
    }

    #[test]
    fn copies_never_replace_an_existing_file() {
        let folder = std::env::temp_dir().join(format!("exam-save-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&folder).unwrap();
        let source = folder.join("cached.txt");
        std::fs::write(&source, "exam").unwrap();
        std::fs::write(folder.join("notes.md"), "mine").unwrap();
        let first = copy_new(&source, &folder, "notes.md").unwrap();
        let second = copy_new(&source, &folder, "notes.md").unwrap();
        assert_eq!(first.file_name().unwrap(), "notes (1).md");
        assert_eq!(second.file_name().unwrap(), "notes (2).md");
        assert_eq!(
            std::fs::read_to_string(folder.join("notes.md")).unwrap(),
            "mine"
        );
        assert_eq!(std::fs::read_to_string(first).unwrap(), "exam");
        std::fs::remove_dir_all(folder).unwrap();
    }
}
