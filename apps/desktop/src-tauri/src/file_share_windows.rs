//! Windows half of `file_share`: CF_HDROP on the clipboard, and the
//! Windows 10/11 share UI through IDataTransferManagerInterop.
use crate::file_share::{on_main, Anchor};
use std::cell::Cell;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::Webview;
use windows::core::{factory, Interface, Ref, HSTRING};
use windows::ApplicationModel::DataTransfer::{DataRequestedEventArgs, DataTransferManager};
use windows::Foundation::TypedEventHandler;
use windows::Storage::{IStorageItem, StorageFile};
use windows::Win32::Foundation::{GlobalFree, HANDLE, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_HDROP;
use windows::Win32::UI::Shell::{IDataTransferManagerInterop, DROPFILES};
use windows_collections::IIterable;

/// The file the share UI asks for once it opens.
static SHARED: Mutex<Option<PathBuf>> = Mutex::new(None);
thread_local! {
    // The window's DataRequested handler, added once on the main thread.
    static LISTENING: Cell<bool> = const { Cell::new(false) };
}

/// The main window's handle, as a number so it can cross to the main thread.
fn window(webview: &Webview) -> Result<isize, String> {
    let hwnd = webview.window().hwnd().map_err(|error| error.to_string())?;
    Ok(hwnd.0 as isize)
}

/// Puts the file on the clipboard as CF_HDROP, what Explorer's own Copy
/// leaves, so pasting gives the file.
pub(crate) async fn copy(webview: &Webview, path: PathBuf) -> Result<(), String> {
    let hwnd = window(webview)?;
    on_main(webview, move |_| {
        // SAFETY: on the main thread, with the window the clipboard opens for.
        unsafe { write_drop_list(HWND(hwnd as _), &path) }.map_err(|error| error.to_string())
    })
    .await
}

/// A DROPFILES header followed by the wide, double-null-terminated path.
unsafe fn write_drop_list(hwnd: HWND, path: &Path) -> windows::core::Result<()> {
    let mut list: Vec<u16> = path.as_os_str().encode_wide().collect();
    list.extend([0, 0]);
    let header = size_of::<DROPFILES>();
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, header + list.len() * 2) }?;
    let base = unsafe { GlobalLock(memory) }.cast::<u8>();
    if base.is_null() {
        let _ = unsafe { GlobalFree(Some(memory)) };
        return Err(windows::core::Error::from_thread());
    }
    // SAFETY: the block holds the header and the list; DROPFILES is packed,
    // and its 20 bytes keep the list two-byte aligned.
    unsafe {
        base.cast::<DROPFILES>().write_unaligned(DROPFILES {
            pFiles: header as u32,
            fWide: true.into(),
            ..Default::default()
        });
        std::ptr::copy_nonoverlapping(list.as_ptr(), base.add(header).cast::<u16>(), list.len());
        // Reports an "error" once the lock count reaches zero; nothing to do.
        let _ = GlobalUnlock(memory);
    }
    let placed = unsafe { OpenClipboard(Some(hwnd)) }.and_then(|()| {
        let placed = unsafe { EmptyClipboard() }.and_then(|()| unsafe {
            SetClipboardData(u32::from(CF_HDROP.0), Some(HANDLE(memory.0)))
        });
        let _ = unsafe { CloseClipboard() };
        placed
    });
    // The clipboard owns the block only once SetClipboardData succeeded.
    if placed.is_err() {
        let _ = unsafe { GlobalFree(Some(memory)) };
    }
    placed.map(|_| ())
}

/// Opens the share UI for the window; Windows places it itself, so the
/// anchor is not used.
pub(crate) async fn share(webview: &Webview, path: PathBuf, _anchor: Anchor) -> Result<(), String> {
    let hwnd = window(webview)?;
    *SHARED.lock().map_err(|error| error.to_string())? = Some(path);
    on_main(webview, move |_| {
        show_share(HWND(hwnd as _)).map_err(|error| error.to_string())
    })
    .await
}

fn show_share(hwnd: HWND) -> windows::core::Result<()> {
    let interop = factory::<DataTransferManager, IDataTransferManagerInterop>()?;
    if !LISTENING.get() {
        // SAFETY: `hwnd` is this process's live main window.
        let manager: DataTransferManager = unsafe { interop.GetForWindow(hwnd) }?;
        manager.DataRequested(&TypedEventHandler::new(
            |_, args: Ref<DataRequestedEventArgs>| fill(args.ok()?),
        ))?;
        LISTENING.set(true);
    }
    // SAFETY: as above.
    unsafe { interop.ShowShareUIForWindow(hwnd) }
}

/// Answers the share UI's request with the file chosen last.
fn fill(args: &DataRequestedEventArgs) -> windows::core::Result<()> {
    let Some(path) = SHARED.lock().ok().and_then(|shared| shared.clone()) else {
        return Ok(());
    };
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path.as_path()))?.join()?;
    let data = args.Request()?.Data()?;
    // The share UI refuses a package without a title.
    data.Properties()?.SetTitle(&file.Name()?)?;
    let items: IIterable<IStorageItem> = vec![Some(file.cast::<IStorageItem>()?)].into();
    data.SetStorageItemsReadOnly(&items)
}
