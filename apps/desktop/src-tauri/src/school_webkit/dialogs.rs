use super::{in_front, SchoolUiDelegate};
use app_core::school_browser::BrowserId;
use block2::RcBlock;
use objc2::rc::{Retained, Weak};
use objc2::{DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSModalResponse, NSModalResponseOK, NSOpenPanel,
    NSResponder, NSTextField, NSView,
};
use objc2_foundation::{ns_string, NSArray, NSPoint, NSRect, NSSize, NSString, NSURL};
use objc2_web_kit::{WKFrameInfo, WKOpenPanelParameters, WKWebView};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

/// The answer to a page's request, given to WebKit exactly once, as WebKit
/// requires: `Some` is OK (the entered text for `prompt()`, the chosen files
/// for a file input), `None` is Cancel. A request that ends without an
/// answer, never shown or torn down with its window or view, answers Cancel
/// when its last reference goes.
pub(super) struct Reply<T: ?Sized + 'static>(Cell<Option<SendAnswer<T>>>);

type SendAnswer<T> = Box<dyn FnOnce(Option<&T>)>;

impl<T: ?Sized + 'static> Reply<T> {
    pub(super) fn new(send: impl FnOnce(Option<&T>) + 'static) -> Rc<Self> {
        Rc::new(Self(Cell::new(Some(Box::new(send)))))
    }

    pub(super) fn send(&self, answer: Option<&T>) {
        if let Some(send) = self.0.take() {
            send(answer);
        }
    }
}

impl<T: ?Sized + 'static> Drop for Reply<T> {
    fn drop(&mut self) {
        self.send(None);
    }
}

pub(super) enum Dialog<'a> {
    Alert,
    Confirm,
    /// `prompt()` with its default text.
    Prompt(Option<&'a NSString>),
}

/// The initiating frame's origin, the dialog's title as in Safari, so an
/// embedded page cannot pass its text off as the school's. None for a frame
/// without a host (`about:blank`, `data:`).
fn origin_label(frame: &WKFrameInfo) -> Option<Retained<NSString>> {
    let origin = unsafe { frame.securityOrigin() };
    let (protocol, host, port) = unsafe { (origin.protocol(), origin.host(), origin.port()) };
    if host.length() == 0 {
        return None;
    }
    let label = match port {
        0 => format!("{protocol}://{host}"),
        port => format!("{protocol}://{host}:{port}"),
    };
    Some(NSString::from_str(&label))
}

thread_local! {
    /// File panels on screen and the view or popup each one answers.
    static FILE_PANELS: RefCell<Vec<(BrowserId, Retained<NSOpenPanel>)>> = const { RefCell::new(Vec::new()) };
}

/// Ends the file panels of the views and popups `owner` names, each
/// answering Cancel. Main thread only.
pub fn end_file_panels(owner: impl Fn(BrowserId) -> bool) {
    let ended: Vec<_> = FILE_PANELS.with_borrow_mut(|panels| {
        let (ended, kept) = std::mem::take(panels)
            .into_iter()
            .partition(|(id, _)| owner(*id));
        *panels = kept;
        ended
    });
    for (_, panel) in ended {
        // The panel's own Cancel ends the sheet and runs its completion
        // handler; the reply answers Cancel if it never runs.
        unsafe { panel.cancel(None) };
    }
}

impl SchoolUiDelegate {
    /// Shows a page dialog as a sheet on the window hosting the view, only
    /// for the school view the person is looking at. A hidden view (background
    /// work) raises no UI: dropping `reply` answers Cancel, as WebKit does
    /// without a delegate.
    pub(super) fn present(
        &self,
        web_view: &WKWebView,
        message: &NSString,
        frame: &WKFrameInfo,
        dialog: Dialog<'_>,
        reply: Rc<Reply<NSString>>,
    ) {
        if !self.ivars().host.foreground() {
            return;
        }
        let Some(window) = web_view.window().filter(|window| window.isVisible()) else {
            return;
        };
        let alert = NSAlert::new(self.mtm());
        match origin_label(frame) {
            Some(origin) => {
                alert.setMessageText(&origin);
                alert.setInformativeText(message);
            }
            None => alert.setMessageText(message),
        }
        alert.addButtonWithTitle(ns_string!("OK"));
        if !matches!(dialog, Dialog::Alert) {
            alert.addButtonWithTitle(ns_string!("Cancel"));
        }
        let field = match dialog {
            Dialog::Prompt(default_text) => {
                let field = NSTextField::initWithFrame(
                    self.mtm().alloc(),
                    NSRect::new(NSPoint::new(0., 0.), NSSize::new(260., 24.)),
                );
                field.setStringValue(default_text.unwrap_or(ns_string!("")));
                let view: &NSView = &field;
                alert.setAccessoryView(Some(view));
                Some(field)
            }
            Dialog::Alert | Dialog::Confirm => None,
        };
        let focus = field.clone();
        let web_view = Weak::new(web_view);
        let done = RcBlock::new(move |response: NSModalResponse| {
            // A view closed or detached meanwhile has no page left to answer.
            let attached = web_view.load().is_some_and(|view| view.window().is_some());
            if response != NSAlertFirstButtonReturn || !attached {
                reply.send(None);
            } else if let Some(field) = &field {
                reply.send(Some(&field.stringValue()));
            } else {
                reply.send(Some(ns_string!("")));
            }
        });
        alert.beginSheetModalForWindow_completionHandler(&window, Some(&done));
        if let Some(field) = focus {
            let responder: &NSResponder = &field;
            alert.window().makeFirstResponder(Some(responder));
        }
    }

    /// Shows the system file panel as a sheet on the window hosting the
    /// view, for the page in front of the school view the person is looking
    /// at; any other request answers Cancel at once.
    pub(super) fn choose_files(
        &self,
        web_view: &WKWebView,
        parameters: &WKOpenPanelParameters,
        reply: Rc<Reply<NSArray<NSURL>>>,
    ) {
        let view = self.ivars().view;
        if !self.ivars().host.foreground()
            || !in_front(view)
            || web_view.isHiddenOrHasHiddenAncestor()
        {
            return;
        }
        let Some(window) = web_view.window().filter(|window| window.isVisible()) else {
            return;
        };
        // A sheet already on the window (another panel, a page dialog) keeps it.
        if window.attachedSheet().is_some() {
            return;
        }
        let panel = NSOpenPanel::openPanel(self.mtm());
        panel.setCanChooseFiles(true);
        panel.setAllowsMultipleSelection(unsafe { parameters.allowsMultipleSelection() });
        panel.setCanChooseDirectories(unsafe { parameters.allowsDirectories() });
        FILE_PANELS.with_borrow_mut(|panels| panels.push((view, panel.clone())));
        let shown = Weak::new(&*panel);
        let web_view = Weak::new(web_view);
        let done = RcBlock::new(move |response: NSModalResponse| {
            let panel = shown.load();
            if let Some(panel) = &panel {
                FILE_PANELS.with_borrow_mut(|panels| {
                    panels.retain(|(_, open)| !std::ptr::eq(&**open, &**panel))
                });
            }
            // A view closed or detached meanwhile has no page left to answer.
            let attached = web_view.load().is_some_and(|view| view.window().is_some());
            match panel.filter(|_| response == NSModalResponseOK && attached) {
                Some(panel) => reply.send(Some(&panel.URLs())),
                None => reply.send(None),
            }
        });
        panel.beginSheetModalForWindow_completionHandler(&window, &done);
    }
}
