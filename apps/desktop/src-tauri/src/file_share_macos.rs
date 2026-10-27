//! macOS half of `file_share`: the general pasteboard and
//! NSSharingServicePicker (AirDrop, Messages, Mail, …).
use crate::file_share::{on_main, Anchor};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSPasteboard, NSPasteboardWriting, NSSharingServicePicker, NSView};
use objc2_foundation::{NSArray, NSPoint, NSRect, NSRectEdge, NSSize, NSString, NSURL};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use tauri::Webview;

thread_local! {
    // AppKit does not keep the picker alive while its menu shows.
    static PICKER: RefCell<Option<Retained<NSSharingServicePicker>>> = const { RefCell::new(None) };
}

fn file_url(path: &Path) -> Result<Retained<NSURL>, String> {
    let path = path
        .to_str()
        .ok_or_else(|| format!("{path:?} is not valid UTF-8"))?;
    Ok(NSURL::fileURLWithPath(&NSString::from_str(path)))
}

/// Puts the file's URL on the general pasteboard, which Finder and other
/// apps paste as the file.
pub(crate) async fn copy(webview: &Webview, path: PathBuf) -> Result<(), String> {
    on_main(webview, move |_| {
        let url = file_url(&path)?;
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        let objects = NSArray::from_retained_slice(&[
            ProtocolObject::<dyn NSPasteboardWriting>::from_retained(url),
        ]);
        pasteboard
            .writeObjects(&objects)
            .then_some(())
            .ok_or_else(|| "the pasteboard refused the file URL".into())
    })
    .await
}

/// Shows the share picker below the button.
pub(crate) async fn share(webview: &Webview, path: PathBuf, anchor: Anchor) -> Result<(), String> {
    on_main(webview, move |platform| {
        let mtm = MainThreadMarker::new().ok_or("not on the main thread")?;
        // SAFETY: `inner()` is tauri's live WKWebView, an NSView, and this
        // closure runs on the main thread inside `with_webview`.
        let view = unsafe { (platform.inner() as *const NSView).as_ref() }.ok_or("no web view")?;
        let url = file_url(&path)?;
        let items =
            NSArray::from_retained_slice(&[Retained::into_super(Retained::into_super(url))]);
        // SAFETY: the items are NSURLs, which the picker accepts.
        let picker = unsafe { NSSharingServicePicker::initWithItems(mtm.alloc(), &items) };
        // The page's coordinates run down from the top; AppKit's run up
        // unless the view is flipped, and "below" is the edge at the larger
        // y only in a flipped view.
        let (y, below) = if view.isFlipped() {
            (anchor.y, NSRectEdge::MaxY)
        } else {
            (
                view.bounds().size.height - anchor.y - anchor.height,
                NSRectEdge::MinY,
            )
        };
        let rect = NSRect::new(
            NSPoint::new(anchor.x, y),
            NSSize::new(anchor.width, anchor.height),
        );
        picker.showRelativeToRect_ofView_preferredEdge(rect, view, below);
        PICKER.with(|kept| kept.replace(Some(picker)));
        Ok(())
    })
    .await
}
