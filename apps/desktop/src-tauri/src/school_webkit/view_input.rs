use super::{in_front, MAIN_VIEWS, POPUPS};
use objc2::runtime::{AnyObject, Bool, Sel};
use objc2::{msg_send, sel, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSEvent, NSResponder, NSView};
use objc2_foundation::NSPoint;
use objc2_web_kit::WKWebView;
use std::{cell::Cell, ptr::null_mut};

/// A command key at a school view or popup. WKWebView alone keeps ⌘C and ⌘V
/// from the Edit menu in a popup (its context menu's Copy works), and wry's
/// main views answer no at once, so a page never sees ⌘B. The focused view
/// asks the menu first and gives `webkit` the rest; any other view leaves the
/// key to the one that has focus.
pub(super) fn key_equivalent(
    view: &NSView,
    event: &NSEvent,
    webkit: impl FnOnce() -> bool,
) -> bool {
    let this: &NSResponder = view;
    let focused = view
        .window()
        .and_then(|window| window.firstResponder())
        .is_some_and(|responder| std::ptr::eq(&*responder, this));
    focused
        && (NSApplication::sharedApplication(view.mtm())
            .mainMenu()
            .is_some_and(|menu| menu.performKeyEquivalent(event))
            || webkit())
}

static WEBKIT_KEY_EQUIVALENT: std::sync::OnceLock<objc2::runtime::Imp> = std::sync::OnceLock::new();

unsafe extern "C-unwind" fn main_view_key_equivalent(
    this: &AnyObject,
    selector: Sel,
    event: &NSEvent,
) -> Bool {
    // SAFETY: the class this is installed on is a WKWebView subclass.
    let view = unsafe { &*(this as *const AnyObject as *const NSView) };
    Bool::new(key_equivalent(view, event, || {
        let Some(original) = WEBKIT_KEY_EQUIVALENT.get() else {
            return false;
        };
        // SAFETY: the stored IMP is WKWebView's own `performKeyEquivalent:`.
        let original: unsafe extern "C-unwind" fn(&AnyObject, Sel, &NSEvent) -> Bool =
            unsafe { std::mem::transmute(*original) };
        unsafe { original(this, selector, event) }.as_bool()
    }))
}

/// wry's class under the title observer's KVO subclass.
fn wry_class(web_view: &WKWebView) -> Option<&'static objc2::runtime::AnyClass> {
    let mut class = web_view.class();
    while class.name().to_bytes().starts_with(b"NSKVONotifying_") {
        class = class.superclass()?;
    }
    Some(class)
}

/// Replaces wry's `performKeyEquivalent:` on its class, once.
pub(super) fn ask_menu_first(web_view: &WKWebView) {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        let selector = sel!(performKeyEquivalent:);
        let (Some(class), Some(method)) = (
            wry_class(web_view),
            <WKWebView as objc2::ClassType>::class().instance_method(selector),
        ) else {
            log::error!("[school-web] command keys stay with wry's class");
            return;
        };
        let _ = WEBKIT_KEY_EQUIVALENT.set(method.implementation());
        // SAFETY: the replacement has the `BOOL (id, SEL, NSEvent *)` signature
        // of the method whose type encoding it is registered with.
        unsafe {
            objc2::ffi::class_replaceMethod(
                class as *const _ as *mut _,
                selector,
                std::mem::transmute::<
                    unsafe extern "C-unwind" fn(&AnyObject, Sel, &NSEvent) -> Bool,
                    objc2::runtime::Imp,
                >(main_view_key_equivalent),
                objc2::ffi::method_getTypeEncoding(method),
            );
        }
    });
}

static WEBKIT_ACCEPTS_FIRST_RESPONDER: std::sync::OnceLock<objc2::runtime::Imp> =
    std::sync::OnceLock::new();

unsafe extern "C-unwind" fn accepts_first_responder(this: &AnyObject, selector: Sel) -> Bool {
    let hidden: bool = unsafe { msg_send![this, isHiddenOrHasHiddenAncestor] };
    if hidden {
        return Bool::NO;
    }
    let Some(original) = WEBKIT_ACCEPTS_FIRST_RESPONDER.get() else {
        return Bool::YES;
    };
    // SAFETY: the stored IMP is WKWebView's own `acceptsFirstResponder`.
    let original: unsafe extern "C-unwind" fn(&AnyObject, Sel) -> Bool =
        unsafe { std::mem::transmute(*original) };
    unsafe { original(this, selector) }
}

static WEBKIT_DRAG_HIT_TEST: std::sync::OnceLock<objc2::runtime::Imp> = std::sync::OnceLock::new();

unsafe extern "C-unwind" fn drag_hit_test(
    this: &AnyObject,
    selector: Sel,
    point: *mut NSPoint,
    types: *mut AnyObject,
) -> *mut AnyObject {
    // SAFETY: the class this is installed on is a WKWebView subclass.
    let view = unsafe { &*(this as *const AnyObject as *const NSView) };
    let hidden: bool = unsafe { msg_send![this, isHiddenOrHasHiddenAncestor] };
    if hidden {
        log_drag(view, None, "hidden");
        return null_mut();
    }
    let Some(original) = WEBKIT_DRAG_HIT_TEST.get() else {
        log_drag(view, None, "no-webkit-hit-test");
        return null_mut();
    };
    // SAFETY: the stored IMP is WKWebView's own `_hitTest:dragTypes:`.
    let original: unsafe extern "C-unwind" fn(
        &AnyObject,
        Sel,
        *mut NSPoint,
        *mut AnyObject,
    ) -> *mut AnyObject = unsafe { std::mem::transmute(*original) };
    let hit = unsafe { original(this, selector, point, types) };
    log_drag(view, Some(hit), if hit.is_null() { "webkit" } else { "" });
    hit
}

thread_local! {
    /// The last drag answer logged: the view's address and whether it took
    /// the drag. AppKit asks on every mouse move; a line is written only
    /// when the answer changes.
    static LAST_DRAG: Cell<Option<(usize, bool)>> = const { Cell::new(None) };
}

/// Diagnostics for a file dragged over a school view or popup: which view
/// instance AppKit asked, in which window, whether it is mounted and hidden,
/// whether it is the page in front of its site, and why it refused (`hidden`,
/// `webkit`: WebKit itself found no target). `hit` is `None` when the view
/// refused before asking WebKit. Never a URL, a cookie or a page's content.
pub(super) fn log_drag(view: &NSView, hit: Option<*mut AnyObject>, reason: &str) {
    let address = view as *const NSView as usize;
    let accepted = hit.is_some_and(|hit| !hit.is_null());
    if LAST_DRAG.get() == Some((address, accepted)) {
        return;
    }
    LAST_DRAG.set(Some((address, accepted)));
    let main = MAIN_VIEWS.with_borrow(|views| {
        views
            .iter()
            .find(|(_, web_view)| {
                web_view
                    .load()
                    .is_some_and(|web_view| &*web_view as *const WKWebView as usize == address)
            })
            .map(|(id, _)| *id)
    });
    let popup = POPUPS.with_borrow(|popups| {
        popups
            .iter()
            .find(|popup| &*popup.web_view as *const WKWebView as usize == address)
            .map(|popup| popup.id)
    });
    let id = main.or(popup);
    let window = view.window().map_or(-1, |window| window.windowNumber());
    let mounted = unsafe { view.superview() }.is_some();
    log::info!(
        "[school-web] drag hit view={:?} kind={} instance={address:#x} window={window} mounted={mounted} hidden={} hiddenAncestor={} front={} accepted={accepted} reason={reason}",
        id.map(|id| (id.site, id.generation)),
        if popup.is_some() { "popup" } else if main.is_some() { "main" } else { "unknown" },
        view.isHidden(),
        view.isHiddenOrHasHiddenAncestor(),
        id.is_some_and(in_front),
    );
}

/// A school view the person left stays resident and hidden, keeping its
/// frame, and wry's view class answers two questions for it as if it were
/// shown:
///
/// - Its page may still load and focus a field (ASP.NET `WebForm_AutoFocus`).
///   WebKit then makes that view the window's first responder, and the
///   visible app stops receiving keys until it is clicked.
/// - WKWebView answers AppKit's drag hit test (`_hitTest:dragTypes:`) with
///   itself for any point in its frame, hidden or not. A hidden view above
///   the visible one in the window (教務Web opened after the LMS) took every
///   file dropped on the LMS draft area and tried to load it as a `file://`
///   page, which Core denies.
///
/// The class therefore refuses both while a view is hidden; a shown view
/// keeps WebKit's answers. It is installed once on the class.
pub(super) fn stay_out_while_hidden(web_view: &WKWebView) {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        // wry registers its class under a versioned module path.
        let Some(class) = wry_class(web_view) else {
            return;
        };
        let selector = sel!(acceptsFirstResponder);
        let Some(method) = <WKWebView as objc2::ClassType>::class().instance_method(selector)
        else {
            return;
        };
        let _ = WEBKIT_ACCEPTS_FIRST_RESPONDER.set(method.implementation());
        // SAFETY: the replacement has the `BOOL (id, SEL)` signature of the
        // method whose type encoding it is registered with.
        let added = unsafe {
            objc2::ffi::class_addMethod(
                class as *const _ as *mut _,
                selector,
                std::mem::transmute::<
                    unsafe extern "C-unwind" fn(&AnyObject, Sel) -> Bool,
                    objc2::runtime::Imp,
                >(accepts_first_responder),
                objc2::ffi::method_getTypeEncoding(method),
            )
        };
        if !added.as_bool() {
            log::error!(
                "[school-web] {} already defines acceptsFirstResponder; hidden views keep focus",
                class.name().to_string_lossy()
            );
        }
        let selector = sel!(_hitTest:dragTypes:);
        let Some(method) = <WKWebView as objc2::ClassType>::class().instance_method(selector)
        else {
            log::error!("[school-web] WKWebView has no drag hit test; hidden views may take drops");
            return;
        };
        let _ = WEBKIT_DRAG_HIT_TEST.set(method.implementation());
        // SAFETY: the replacement has the `NSView *(id, SEL, NSPoint *, NSSet *)`
        // signature of the method whose type encoding it is registered with.
        let added = unsafe {
            objc2::ffi::class_addMethod(
                class as *const _ as *mut _,
                selector,
                std::mem::transmute::<
                    unsafe extern "C-unwind" fn(
                        &AnyObject,
                        Sel,
                        *mut NSPoint,
                        *mut AnyObject,
                    ) -> *mut AnyObject,
                    objc2::runtime::Imp,
                >(drag_hit_test),
                objc2::ffi::method_getTypeEncoding(method),
            )
        };
        if !added.as_bool() {
            log::error!(
                "[school-web] {} already defines _hitTest:dragTypes:; hidden views may take drops",
                class.name().to_string_lossy()
            );
        }
    });
}
