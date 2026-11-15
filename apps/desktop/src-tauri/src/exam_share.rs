use crate::{
    coded, commands::cloud::SystemBrowser, require_local_main, secrets::OsSecretVault, CommandError,
};
use app_core::{
    exam_share::{ExamAction, ExamAnswer, ExamError, ExamShare, ExamUploadFile},
    oauth::{code_exchange::OAuthClient, NativeBrowserAuth},
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tauri::{Manager, Webview};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub(crate) fn service(
    app: tauri::AppHandle,
    vault: OsSecretVault,
    folder: PathBuf,
) -> Option<ExamShare> {
    let version = app.package_info().version.to_string();
    let browser = option_env!("ISCT_GOOGLE_DESKTOP_CLIENT_ID").map(|id| {
        let client = OAuthClient {
            id: id.to_owned(),
            secret: option_env!("ISCT_GOOGLE_DESKTOP_CLIENT_SECRET").map(str::to_owned),
        };
        let browser: Arc<dyn NativeBrowserAuth> = Arc::new(SystemBrowser(app));
        (client, browser)
    });
    ExamShare::new(Arc::new(vault), folder, version, browser).ok()
}

#[tauri::command]
pub(crate) async fn exam_share(
    webview: Webview,
    service: tauri::State<'_, Option<ExamShare>>,
    selection: tauri::State<'_, UploadSelection>,
    mut action: ExamAction,
) -> Result<ExamAnswer, CommandError> {
    require_local_main(&webview)?;
    let service = service
        .inner()
        .as_ref()
        .ok_or_else(|| coded(ExamError::Unavailable))?;
    let _selected_files = match &mut action {
        ExamAction::Upload { files, .. } => Some(selection.resolve(files).map_err(coded)?),
        _ => None,
    };
    let cancel = CancellationToken::new();
    let _guard = cancel.clone().drop_guard();
    let answer = service.act(action, cancel).await.map_err(coded)?;
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
