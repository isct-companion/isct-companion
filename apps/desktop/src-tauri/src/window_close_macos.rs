//! macOS half of `window_close`: an NSAlert sheet whose suppression
//! checkbox is "Remember my choice". A hidden window comes back from the
//! Dock (`RunEvent::Reopen`).
use crate::window_close::{answered, CloseAction, Prompt};
use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertSecondButtonReturn, NSControlStateValueOn,
    NSModalResponse, NSWindow,
};
use objc2_foundation::NSString;
use tauri::Manager;

pub(crate) fn ask(window: &tauri::Window, prompt: Prompt) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Ok(handle) = window.ns_window() else {
        return;
    };
    // SAFETY: tauri's live NSWindow for this window; close requests arrive
    // on the main thread.
    let Some(ns_window) = (unsafe { (handle as *const NSWindow).as_ref() }) else {
        return;
    };
    // A second close while the sheet shows changes nothing.
    if ns_window.attachedSheet().is_some() {
        return;
    }
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(&prompt.title));
    alert.setInformativeText(&NSString::from_str(&prompt.message));
    alert.addButtonWithTitle(&NSString::from_str(&prompt.quit));
    alert.addButtonWithTitle(&NSString::from_str(&prompt.keep));
    // AppKit binds Escape only to a button titled "Cancel" in English.
    alert
        .addButtonWithTitle(&NSString::from_str(&prompt.cancel))
        .setKeyEquivalent(&NSString::from_str("\u{1b}"));
    alert.setShowsSuppressionButton(true);
    if let Some(button) = alert.suppressionButton() {
        button.setTitle(&NSString::from_str(&prompt.remember));
    }
    let app = window.app_handle().clone();
    let answering = alert.clone();
    let handler = RcBlock::new(move |response: NSModalResponse| {
        let remember = answering
            .suppressionButton()
            .is_some_and(|button| button.state() == NSControlStateValueOn);
        let action = if response == NSAlertFirstButtonReturn {
            Some(CloseAction::Quit)
        } else if response == NSAlertSecondButtonReturn {
            Some(CloseAction::Background)
        } else {
            None
        };
        answered(&app, action, remember);
    });
    alert.beginSheetModalForWindow_completionHandler(ns_window, Some(&handler));
}
