//! macOS: WebKit hooks wry does not offer, installed on each school view by
//! wrapping wry's delegates in shims that add their own methods and forward
//! everything else.
//!
//! - UI delegate: the download (↓) button of WebKit's built-in PDF viewer
//!   never starts a `WKDownload`, so `on_download` does not see it. WebKit
//!   (`PDFPluginBase::save` → `UIDelegate::UIClient::saveDataToFileInDownloadsFolder`)
//!   hands the bytes to the private method
//!   `_webView:saveDataToFile:suggestedFilename:mimeType:originatingURL:` and
//!   does nothing when the delegate lacks it. Private API: if WebKit renames
//!   the selector the button goes back to doing nothing; nothing crashes.
//! - Navigation delegate: wry allows every response WebKit can display, but
//!   WebKit commits a displayable `Content-Disposition: attachment` response
//!   (Moodle submissions, `?forcedownload=1`) as an empty page. The shim
//!   fetches such a response as a `WKDownload` into a temporary file instead
//!   and loads the bytes back into the same view under the original URL, so
//!   WebKit shows the file itself (PDF viewer, image, text). Attachments
//!   WebKit cannot display stay with wry, which downloads them.
//!
//! WebKit caches which delegate methods exist when a delegate is set, so each
//! shim also answers `respondsToSelector:` for wry's methods and forwards them.

use block2::DynBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, AnyProtocol, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{
    define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly,
    Message,
};
use objc2_foundation::{
    ns_string, NSData, NSError, NSHTTPURLResponse, NSString, NSURLRequest, NSURLResponse, NSURL,
};
use objc2_web_kit::{
    WKDownload, WKDownloadDelegate, WKNavigationDelegate, WKNavigationResponse,
    WKNavigationResponsePolicy, WKUIDelegate, WKWebView,
};
use std::cell::RefCell;
use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::null_mut;

/// What the PDF viewer asked to save.
pub struct SavedData {
    pub bytes: Vec<u8>,
    pub suggested_name: String,
    pub mime_type: String,
    /// The PDF's own URL (absolute string), when WebKit provides one.
    pub origin: Option<String>,
}

pub struct UiIvars {
    /// wry's delegate. WebKit holds delegates weakly, but wry keeps its own
    /// reference too; this one keeps forwarding valid regardless.
    original: Retained<ProtocolObject<dyn WKUIDelegate>>,
    on_save: Box<dyn Fn(SavedData)>,
}

pub struct NavigationIvars {
    original: Retained<ProtocolObject<dyn WKNavigationDelegate>>,
    /// Attachments being fetched to show in place. `WKDownload.delegate` is
    /// weak; this shim is the delegate and lives as long as the view.
    attachments: RefCell<Vec<Attachment>>,
}

struct Attachment {
    download: Retained<WKDownload>,
    /// The temporary file and the response it holds, once WebKit asked.
    file: Option<(PathBuf, Retained<NSURLResponse>)>,
}

fn responds(object: &AnyObject, selector: Sel) -> bool {
    unsafe { msg_send![object, respondsToSelector: selector] }
}

fn forwarding_target(object: &AnyObject, selector: Sel) -> *mut AnyObject {
    if responds(object, selector) {
        object as *const AnyObject as *mut AnyObject
    } else {
        null_mut()
    }
}

/// A main-frame attachment WebKit can display, which it would otherwise
/// commit as an empty page.
fn displayable_attachment(navigation: &WKNavigationResponse) -> bool {
    if !unsafe { navigation.isForMainFrame() && navigation.canShowMIMEType() } {
        return false;
    }
    unsafe { navigation.response() }
        .downcast_ref::<NSHTTPURLResponse>()
        .and_then(|http| http.valueForHTTPHeaderField(ns_string!("Content-Disposition")))
        .is_some_and(|value| {
            value
                .to_string()
                .split(';')
                .next()
                .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("attachment"))
        })
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = UiIvars]
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
            own || responds(self.original(), selector)
        }

        #[unsafe(method(forwardingTargetForSelector:))]
        fn forwarding_target_for_selector(&self, selector: Sel) -> *mut AnyObject {
            forwarding_target(self.original(), selector)
        }

        #[unsafe(method(conformsToProtocol:))]
        fn conforms_to_protocol(&self, protocol: &AnyProtocol) -> bool {
            let own: bool = unsafe { msg_send![super(self), conformsToProtocol: protocol] };
            own || unsafe { msg_send![self.original(), conformsToProtocol: protocol] }
        }
    }
);

impl SchoolUiDelegate {
    fn original(&self) -> &AnyObject {
        self.ivars().original.as_ref()
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = NavigationIvars]
    pub struct SchoolNavigationDelegate;

    unsafe impl NSObjectProtocol for SchoolNavigationDelegate {}

    unsafe impl WKNavigationDelegate for SchoolNavigationDelegate {}

    unsafe impl WKDownloadDelegate for SchoolNavigationDelegate {
        #[unsafe(method(download:decideDestinationUsingResponse:suggestedFilename:completionHandler:))]
        fn decide_destination(
            &self,
            download: &WKDownload,
            response: &NSURLResponse,
            _suggested_name: &NSString,
            handler: &DynBlock<dyn Fn(*mut NSURL)>,
        ) {
            let file = std::env::temp_dir()
                .join(format!("isct-companion-attachment-{}", uuid::Uuid::new_v4()));
            let destination = file
                .to_str()
                .map(|path| NSURL::fileURLWithPath_isDirectory(&NSString::from_str(path), false));
            let destination = destination.filter(|_| {
                let mut attachments = self.ivars().attachments.borrow_mut();
                let Some(attachment) = attachments
                    .iter_mut()
                    .find(|attachment| std::ptr::eq(&*attachment.download, download))
                else {
                    return false;
                };
                attachment.file = Some((file, response.retain()));
                true
            });
            match destination {
                Some(url) => handler.call((Retained::as_ptr(&url).cast_mut(),)),
                None => handler.call((null_mut(),)),
            }
        }

        #[unsafe(method(downloadDidFinish:))]
        fn download_did_finish(&self, download: &WKDownload) {
            let Some((file, response)) = self.take(download) else {
                return;
            };
            let bytes = std::fs::read(&file);
            let _ = std::fs::remove_file(&file);
            let (Ok(bytes), Some(web_view), Some(url)) =
                (bytes, unsafe { download.webView() }, response.URL())
            else {
                eprintln!("[school-download] attachment could not be shown");
                return;
            };
            // The same response minus its headers, so WebKit displays it.
            let shown = NSURLResponse::initWithURL_MIMEType_expectedContentLength_textEncodingName(
                NSURLResponse::alloc(),
                &url,
                response.MIMEType().as_deref(),
                isize::try_from(bytes.len()).unwrap_or(-1),
                response.textEncodingName().as_deref(),
            );
            unsafe {
                web_view.loadSimulatedRequest_response_responseData(
                    &NSURLRequest::requestWithURL(&url),
                    &shown,
                    &NSData::from_vec(bytes),
                )
            };
        }

        #[unsafe(method(download:didFailWithError:resumeData:))]
        fn download_did_fail(
            &self,
            download: &WKDownload,
            error: &NSError,
            _resume_data: Option<&NSData>,
        ) {
            eprintln!(
                "[school-download] attachment failed: {}",
                error.localizedDescription()
            );
            if let Some((file, _)) = self.take(download) {
                let _ = std::fs::remove_file(file);
            }
        }
    }

    impl SchoolNavigationDelegate {
        #[unsafe(method(webView:decidePolicyForNavigationResponse:decisionHandler:))]
        fn decide_policy_for_response(
            &self,
            web_view: &WKWebView,
            navigation: &WKNavigationResponse,
            handler: &DynBlock<dyn Fn(WKNavigationResponsePolicy)>,
        ) {
            if displayable_attachment(navigation) {
                handler.call((WKNavigationResponsePolicy::Download,));
                return;
            }
            let original = &self.ivars().original;
            if original.respondsToSelector(sel!(webView:decidePolicyForNavigationResponse:decisionHandler:)) {
                unsafe {
                    original.webView_decidePolicyForNavigationResponse_decisionHandler(
                        web_view, navigation, handler,
                    )
                };
            } else {
                handler.call((WKNavigationResponsePolicy::Allow,));
            }
        }

        // wry only turns responses WebKit cannot display into downloads, so a
        // displayable attachment here is always one the shim started.
        #[unsafe(method(webView:navigationResponse:didBecomeDownload:))]
        fn response_did_become_download(
            &self,
            web_view: &WKWebView,
            navigation: &WKNavigationResponse,
            download: &WKDownload,
        ) {
            if displayable_attachment(navigation) {
                unsafe { download.setDelegate(Some(ProtocolObject::from_ref(self))) };
                self.ivars().attachments.borrow_mut().push(Attachment {
                    download: download.retain(),
                    file: None,
                });
                return;
            }
            let original = &self.ivars().original;
            if original.respondsToSelector(sel!(webView:navigationResponse:didBecomeDownload:)) {
                unsafe {
                    original.webView_navigationResponse_didBecomeDownload(
                        web_view, navigation, download,
                    )
                };
            }
        }

        #[unsafe(method(respondsToSelector:))]
        fn responds_to_selector(&self, selector: Sel) -> bool {
            let own: bool = unsafe { msg_send![super(self), respondsToSelector: selector] };
            own || responds(self.original(), selector)
        }

        #[unsafe(method(forwardingTargetForSelector:))]
        fn forwarding_target_for_selector(&self, selector: Sel) -> *mut AnyObject {
            forwarding_target(self.original(), selector)
        }

        #[unsafe(method(conformsToProtocol:))]
        fn conforms_to_protocol(&self, protocol: &AnyProtocol) -> bool {
            let own: bool = unsafe { msg_send![super(self), conformsToProtocol: protocol] };
            own || unsafe { msg_send![self.original(), conformsToProtocol: protocol] }
        }
    }
);

impl SchoolNavigationDelegate {
    fn original(&self) -> &AnyObject {
        self.ivars().original.as_ref()
    }

    /// Forgets a finished or failed attachment, returning its temporary file.
    fn take(&self, download: &WKDownload) -> Option<(PathBuf, Retained<NSURLResponse>)> {
        let mut attachments = self.ivars().attachments.borrow_mut();
        let index = attachments
            .iter()
            .position(|attachment| std::ptr::eq(&*attachment.download, download))?;
        attachments.remove(index).file
    }
}

/// Keeps the shims alive: WebKit's delegate properties are weak.
///
/// The handle lives in the controller state next to the resident webview and
/// is only replaced from the main thread (inside `with_webview`). It is
/// otherwise dropped at exit; releasing an NSObject off the main thread is
/// safe, and its ivars hold nothing thread-bound beyond wry's delegates, which
/// wry itself keeps alive, and attachment downloads still in flight.
pub struct DelegateHandle {
    _ui: Option<Retained<SchoolUiDelegate>>,
    _navigation: Option<Retained<SchoolNavigationDelegate>>,
}

// SAFETY: see the type's documentation; the shims are never messaged through
// the handle, only kept alive and eventually released.
unsafe impl Send for DelegateHandle {}

/// Wraps the WKWebView's current UI and navigation delegates in the shims.
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
    let ui = unsafe { web_view.UIDelegate() }.map(|original| {
        let shim = mtm
            .alloc::<SchoolUiDelegate>()
            .set_ivars(UiIvars { original, on_save });
        let shim: Retained<SchoolUiDelegate> = unsafe { msg_send![super(shim), init] };
        unsafe { web_view.setUIDelegate(Some(ProtocolObject::from_ref(&*shim))) };
        shim
    });
    let navigation = unsafe { web_view.navigationDelegate() }.map(|original| {
        let shim = mtm
            .alloc::<SchoolNavigationDelegate>()
            .set_ivars(NavigationIvars {
                original,
                attachments: RefCell::default(),
            });
        let shim: Retained<SchoolNavigationDelegate> = unsafe { msg_send![super(shim), init] };
        unsafe { web_view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*shim))) };
        shim
    });
    if cfg!(debug_assertions) {
        let check = |object: Option<&AnyObject>, selector: Sel| {
            object.is_some_and(|o| responds(o, selector))
        };
        let ui_object = ui.as_deref().map(|shim| shim.as_ref());
        let navigation_object = navigation.as_deref().map(|shim| shim.as_ref());
        eprintln!(
            "[school-download] WebKit hooks installed: new-window forwarded={}, pdf-save={}, attachment={}, page-load forwarded={}",
            check(ui_object, sel!(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:)),
            check(ui_object, sel!(_webView:saveDataToFile:suggestedFilename:mimeType:originatingURL:)),
            check(navigation_object, sel!(webView:navigationResponse:didBecomeDownload:)),
            check(navigation_object, sel!(webView:didFinishNavigation:)),
        );
    }
    (ui.is_some() || navigation.is_some()).then_some(DelegateHandle {
        _ui: ui,
        _navigation: navigation,
    })
}
