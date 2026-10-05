//! macOS: makes the download (↓) button of WebKit's built-in PDF viewer save
//! the file.
//!
//! That button never starts a `WKDownload`, so `on_download` does not see it.
//! WebKit (`PDFPluginBase::save` → `WebPageProxy::savePDFToFileInDownloadsFolder`
//! → `UIDelegate::UIClient::saveDataToFileInDownloadsFolder`) hands the bytes
//! to the private WKUIDelegate method
//! `_webView:saveDataToFile:suggestedFilename:mimeType:originatingURL:`, and
//! does nothing when the delegate does not implement it. wry's delegate does
//! not, so the school view's delegate is wrapped in a shim that implements
//! that one method and forwards everything else to wry's delegate.
//!
//! This is private WebKit API. If WebKit renames the selector, the button goes
//! back to doing nothing; nothing crashes, as the shim only adds a method.
//!
//! WebKit caches which delegate methods exist when the delegate is set, so the
//! shim must answer `respondsToSelector:` for wry's methods too (new windows,
//! JS dialogs, permission prompts, file uploads), and forward their calls.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, AnyProtocol, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_foundation::{NSData, NSString, NSURL};
use objc2_web_kit::{WKUIDelegate, WKWebView};
use std::ffi::c_void;
use std::ptr::null_mut;

/// What the PDF viewer asked to save.
pub struct SavedData {
    pub bytes: Vec<u8>,
    pub suggested_name: String,
    pub mime_type: String,
    /// The PDF's own URL (absolute string), when WebKit provides one.
    pub origin: Option<String>,
}

pub struct ShimIvars {
    /// wry's delegate. WebKit holds delegates weakly, but wry keeps its own
    /// reference too; this one keeps forwarding valid regardless.
    original: Retained<ProtocolObject<dyn WKUIDelegate>>,
    on_save: Box<dyn Fn(SavedData)>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ShimIvars]
    pub struct SchoolUiDelegate;

    unsafe impl NSObjectProtocol for SchoolUiDelegate {}

    // Every WKUIDelegate method is optional: the shim implements none of them
    // itself and forwards them all to wry's delegate.
    unsafe impl WKUIDelegate for SchoolUiDelegate {}

    impl SchoolUiDelegate {
        #[unsafe(method(_webView:saveDataToFile:suggestedFilename:mimeType:originatingURL:))]
        fn save_data_to_file(
            &self,
            _web_view: Option<&AnyObject>,
            data: Option<&NSData>,
            suggested_name: Option<&NSString>,
            mime_type: Option<&NSString>,
            origin: Option<&NSURL>,
        ) {
            let Some(data) = data else {
                return;
            };
            let origin = origin
                .and_then(|url| url.absoluteString())
                .map(|url| url.to_string());
            (self.ivars().on_save)(SavedData {
                bytes: data.to_vec(),
                suggested_name: suggested_name.map(|name| name.to_string()).unwrap_or_default(),
                mime_type: mime_type.map(|mime| mime.to_string()).unwrap_or_default(),
                origin,
            });
        }

        #[unsafe(method(respondsToSelector:))]
        fn responds_to_selector(&self, selector: Sel) -> bool {
            let own: bool = unsafe { msg_send![super(self), respondsToSelector: selector] };
            own || self.original_responds(selector)
        }

        #[unsafe(method(forwardingTargetForSelector:))]
        fn forwarding_target_for_selector(&self, selector: Sel) -> *mut AnyObject {
            if self.original_responds(selector) {
                Retained::as_ptr(&self.ivars().original) as *mut AnyObject
            } else {
                null_mut()
            }
        }

        #[unsafe(method(conformsToProtocol:))]
        fn conforms_to_protocol(&self, protocol: &AnyProtocol) -> bool {
            let own: bool = unsafe { msg_send![super(self), conformsToProtocol: protocol] };
            own || unsafe { msg_send![&*self.ivars().original, conformsToProtocol: protocol] }
        }
    }
);

impl SchoolUiDelegate {
    fn original_responds(&self, selector: Sel) -> bool {
        unsafe { msg_send![&*self.ivars().original, respondsToSelector: selector] }
    }
}

/// Keeps the shim alive: `WKWebView.UIDelegate` is a weak property.
///
/// The handle lives in the controller state next to the resident webview and
/// is only replaced from the main thread (inside `with_webview`). It is
/// otherwise dropped at exit; releasing an NSObject off the main thread is
/// safe, and its ivars hold nothing thread-bound beyond wry's delegate, which
/// wry itself keeps alive.
pub struct DelegateHandle(#[allow(dead_code)] Retained<SchoolUiDelegate>);

// SAFETY: see the type's documentation; the shim is never messaged through
// the handle, only kept alive and eventually released.
unsafe impl Send for DelegateHandle {}

/// Wraps the WKWebView's current UI delegate in the save shim.
///
/// # Safety
/// `web_view` must be a live `WKWebView` (tauri's `PlatformWebview::inner()`),
/// and this must run on the main thread (inside `Webview::with_webview`).
pub unsafe fn install(
    web_view: *mut c_void,
    on_save: Box<dyn Fn(SavedData)>,
) -> Option<DelegateHandle> {
    let mtm = MainThreadMarker::new()?;
    let web_view = unsafe { (web_view as *const WKWebView).as_ref() }?;
    let original = unsafe { web_view.UIDelegate() }?;
    let shim = mtm
        .alloc::<SchoolUiDelegate>()
        .set_ivars(ShimIvars { original, on_save });
    let shim: Retained<SchoolUiDelegate> = unsafe { msg_send![super(shim), init] };
    unsafe { web_view.setUIDelegate(Some(ProtocolObject::from_ref(&*shim))) };
    if cfg!(debug_assertions) {
        // The forwarding must keep wry's new-window handling (`on_new_window`).
        let new_window =
            sel!(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:);
        let save = sel!(_webView:saveDataToFile:suggestedFilename:mimeType:originatingURL:);
        let responds =
            |selector: Sel| -> bool { unsafe { msg_send![&*shim, respondsToSelector: selector] } };
        eprintln!(
            "[school-download] PDF save hook installed: new-window forwarded={}, save={}",
            responds(new_window),
            responds(save)
        );
    }
    Some(DelegateHandle(shim))
}
