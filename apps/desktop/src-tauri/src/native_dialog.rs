//! Native file dialogs through rfd, parented to the calling window. Not
//! tauri-plugin-dialog: Tauri injects every plugin's init script into the
//! school webviews too, and that plugin's script replaced `window.confirm`
//! with an async function and `window.alert` with an IPC call there, so
//! school pages skipped their confirmations
//! (slotledger approach/desktop-no-plugin-scripts-in-school-views).
use crate::{shell, CommandError};
use rfd::{AsyncFileDialog, FileHandle};
use std::future::Future;
use std::path::PathBuf;
use tauri::Window;

/// A folder the person picks; None when they cancel.
pub(crate) async fn pick_folder(
    window: &Window,
    title: &'static str,
) -> Result<Option<PathBuf>, CommandError> {
    show(window, move |dialog| dialog.set_title(title).pick_folder()).await
}

/// Where to save `name`, offered with a filter for its extension; None when
/// the person cancels.
pub(crate) async fn save_file(
    window: &Window,
    name: String,
) -> Result<Option<PathBuf>, CommandError> {
    show(window, move |dialog| {
        let dialog = match name.rsplit_once('.') {
            Some((_, extension)) => dialog.add_filter(extension.to_uppercase(), &[extension]),
            None => dialog,
        };
        dialog.set_file_name(name).save_file()
    })
    .await
}

/// AppKit dialogs are created on the main thread; the pending dialog is
/// awaited here.
async fn show<F>(
    window: &Window,
    open: impl FnOnce(AsyncFileDialog) -> F + Send + 'static,
) -> Result<Option<PathBuf>, CommandError>
where
    F: Future<Output = Option<FileHandle>> + Send + 'static,
{
    let (send, receive) = tokio::sync::oneshot::channel();
    let parent = window.clone();
    window
        .run_on_main_thread(move || {
            let dialog = AsyncFileDialog::new()
                .set_parent(&parent)
                .set_can_create_directories(true);
            let _ = send.send(open(dialog));
        })
        .map_err(shell)?;
    let pending = receive.await.map_err(shell)?;
    Ok(pending.await.map(|file| file.path().to_path_buf()))
}
