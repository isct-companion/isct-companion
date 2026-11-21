//! The traffic lights centred in the shell's title row (`--titlebar` in
//! styles.css). Tauri's `traffic_light_position` only reaches the webview,
//! which leaves the buttons where AppKit put them in this window, and AppKit
//! lays the title bar out again on every resize; so the host places them
//! after the window is built and after each change of size or focus.
use objc2_app_kit::{NSWindow, NSWindowButton};

/// The title row's height and the first button's left edge, in points.
const ROW: f64 = 46.0;
const LEFT: f64 = 14.0;

pub(crate) fn place(window: &tauri::Window) {
    let Ok(pointer) = window.ns_window() else {
        return;
    };
    // SAFETY: Tauri hands out this window's live NSWindow; window events and
    // setup run on the main thread, which AppKit requires.
    let ns_window = unsafe { &*(pointer as *const NSWindow) };
    let (Some(close), Some(minimize), Some(zoom)) = (
        ns_window.standardWindowButton(NSWindowButton::CloseButton),
        ns_window.standardWindowButton(NSWindowButton::MiniaturizeButton),
        ns_window.standardWindowButton(NSWindowButton::ZoomButton),
    ) else {
        return;
    };
    // SAFETY: reading the button's view hierarchy on the main thread.
    let Some(container) = (unsafe { close.superview().and_then(|bar| bar.superview()) }) else {
        return;
    };
    let size = close.frame().size;
    let spacing = minimize.frame().origin.x - close.frame().origin.x;
    let mut bar = container.frame();
    bar.size.height = ROW;
    bar.origin.y = ns_window.frame().size.height - ROW;
    container.setFrame(bar);
    for (index, button) in [close, minimize, zoom].iter().enumerate() {
        let mut origin = button.frame().origin;
        origin.x = LEFT + index as f64 * spacing;
        origin.y = (ROW - size.height) / 2.0;
        button.setFrameOrigin(origin);
    }
}
