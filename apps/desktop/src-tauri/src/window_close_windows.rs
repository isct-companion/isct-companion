//! Windows half of `window_close`: a task dialog whose verification checkbox
//! is "Remember my choice", and the tray icon that brings a hidden window
//! back or quits.
use crate::window_close::{answered, show_main, text, CloseAction, Prompt};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};
use windows::core::{BOOL, HSTRING, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Controls::{
    TaskDialogIndirect, TASKDIALOGCONFIG, TASKDIALOG_BUTTON, TDCBF_CANCEL_BUTTON,
    TDF_ALLOW_DIALOG_CANCELLATION, TDF_POSITION_RELATIVE_TO_WINDOW,
};

const QUIT: i32 = 100;
const KEEP: i32 = 101;
const TRAY: &str = "main";

static ASKING: AtomicBool = AtomicBool::new(false);

pub(crate) fn ask(window: &tauri::Window, prompt: Prompt) {
    // A second close while the dialog shows changes nothing.
    if ASKING.swap(true, Ordering::SeqCst) {
        return;
    }
    let Ok(parent) = window.hwnd() else {
        ASKING.store(false, Ordering::SeqCst);
        return;
    };
    // HWND is not Send; the dialog only needs the handle's value.
    let parent = parent.0 as isize;
    let app = window.app_handle().clone();
    // The dialog runs its own message loop; keep it off the event loop.
    std::thread::spawn(move || {
        let (action, remember) = show(parent, &prompt);
        ASKING.store(false, Ordering::SeqCst);
        let handle = app.clone();
        let _ = handle.run_on_main_thread(move || answered(&app, action, remember));
    });
}

fn show(parent: isize, prompt: &Prompt) -> (Option<CloseAction>, bool) {
    let caption = HSTRING::from("IsctCompanion");
    let title = HSTRING::from(prompt.title.as_str());
    let message = HSTRING::from(prompt.message.as_str());
    let quit = HSTRING::from(prompt.quit.as_str());
    let keep = HSTRING::from(prompt.keep.as_str());
    let remember = HSTRING::from(prompt.remember.as_str());
    let buttons = [
        TASKDIALOG_BUTTON {
            nButtonID: QUIT,
            pszButtonText: PCWSTR(quit.as_ptr()),
        },
        TASKDIALOG_BUTTON {
            nButtonID: KEEP,
            pszButtonText: PCWSTR(keep.as_ptr()),
        },
    ];
    let config = TASKDIALOGCONFIG {
        cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: HWND(parent as *mut core::ffi::c_void),
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION | TDF_POSITION_RELATIVE_TO_WINDOW,
        dwCommonButtons: TDCBF_CANCEL_BUTTON,
        pszWindowTitle: PCWSTR(caption.as_ptr()),
        pszMainInstruction: PCWSTR(title.as_ptr()),
        pszContent: PCWSTR(message.as_ptr()),
        cButtons: buttons.len() as u32,
        pButtons: buttons.as_ptr(),
        nDefaultButton: QUIT,
        pszVerificationText: PCWSTR(remember.as_ptr()),
        ..Default::default()
    };
    let mut button = 0;
    let mut checked = BOOL(0);
    // SAFETY: every string and the button array outlive the call.
    let shown = unsafe {
        TaskDialogIndirect(
            &config,
            Some(&mut button as *mut i32),
            None,
            Some(&mut checked as *mut BOOL),
        )
    };
    if let Err(error) = shown {
        log::warn!("[window] close prompt failed: {error}");
        return (None, false);
    }
    let action = match button {
        QUIT => Some(CloseAction::Quit),
        KEEP => Some(CloseAction::Background),
        _ => None,
    };
    (action, checked.as_bool())
}

/// The tray icon while the window is hidden; it stays until the app quits.
pub(crate) fn show_tray(app: &AppHandle) {
    if app.tray_by_id(TRAY).is_some() {
        return;
    }
    let built = (|| -> tauri::Result<()> {
        let show = MenuItem::with_id(app, "tray-show", text(app, "tray.show"), true, None::<&str>)?;
        let quit = MenuItem::with_id(
            app,
            "tray-quit",
            text(app, "common.quit"),
            true,
            None::<&str>,
        )?;
        let menu = Menu::with_items(app, &[&show, &quit])?;
        let mut tray = TrayIconBuilder::with_id(TRAY)
            .tooltip("IsctCompanion")
            .menu(&menu)
            .show_menu_on_left_click(false)
            .on_menu_event(|app, event| match event.id().as_ref() {
                "tray-show" => show_main(app),
                "tray-quit" => app.exit(0),
                _ => {}
            })
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    show_main(tray.app_handle());
                }
            });
        if let Some(icon) = app.default_window_icon() {
            tray = tray.icon(icon.clone());
        }
        tray.build(app)?;
        Ok(())
    })();
    if let Err(error) = built {
        log::warn!("[window] tray icon failed: {error}");
    }
}
