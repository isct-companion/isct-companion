//! macOS: WebKit hooks wry does not offer, installed on each school view by
//! wrapping wry's delegates in shims that add one method each and forward
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
//!   for a `Content-Disposition: attachment` response WebKit then commits an
//!   empty page. The shim reports each main-frame response to the shell, which
//!   may turn it into a download (`WKNavigationResponsePolicyDownload`); wry's
//!   own delegate then receives the download through `didBecomeDownload`.
//!
//! WebKit caches which delegate methods exist when a delegate is set, so each
//! shim also answers `respondsToSelector:` for wry's methods and forwards them.

use block2::DynBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, AnyProtocol, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_foundation::{NSData, NSHTTPURLResponse, NSString, NSURL};
use objc2_web_kit::{
    WKNavigationDelegate, WKNavigationResponse, WKNavigationResponsePolicy, WKUIDelegate, WKWebView,
};
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

/// A main-frame response about to be displayed.
pub struct MainFrameResponse {
    pub url: String,
    pub disposition: Option<String>,
    pub mime_type: Option<String>,
    pub suggested_name: String,
}

pub struct UiIvars {
    /// wry's delegate. WebKit holds delegates weakly, but wry keeps its own
    /// reference too; this one keeps forwarding valid regardless.
    original: Retained<ProtocolObject<dyn WKUIDelegate>>,
    on_save: Box<dyn Fn(SavedData)>,
}

pub struct NavigationIvars {
    original: Retained<ProtocolObject<dyn WKNavigationDelegate>>,
    /// True turns the response into a download.
    on_response: Box<dyn Fn(MainFrameResponse) -> bool>,
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

    impl SchoolNavigationDelegate {
        #[unsafe(method(webView:decidePolicyForNavigationResponse:decisionHandler:))]
        fn decide_policy_for_response(
            &self,
            web_view: &WKWebView,
            navigation: &WKNavigationResponse,
            handler: &DynBlock<dyn Fn(WKNavigationResponsePolicy)>,
        ) {
            if unsafe { navigation.isForMainFrame() } {
                let response = unsafe { navigation.response() };
                let disposition = response
                    .downcast_ref::<NSHTTPURLResponse>()
                    .and_then(|http| {
                        http.valueForHTTPHeaderField(&NSString::from_str("Content-Disposition"))
                    })
                    .map(|value| value.to_string());
                let facts = MainFrameResponse {
                    url: response
                        .URL()
                        .and_then(|url| url.absoluteString())
                        .map(|url| url.to_string())
                        .unwrap_or_default(),
                    disposition,
                    mime_type: response.MIMEType().map(|mime| mime.to_string()),
                    suggested_name: response
                        .suggestedFilename()
                        .map(|name| name.to_string())
                        .unwrap_or_default(),
                };
                if (self.ivars().on_response)(facts) {
                    handler.call((WKNavigationResponsePolicy::Download,));
                    return;
                }
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
}

/// Keeps the shims alive: WebKit's delegate properties are weak.
///
/// The handle lives in the controller state next to the resident webview and
/// is only replaced from the main thread (inside `with_webview`). It is
/// otherwise dropped at exit; releasing an NSObject off the main thread is
/// safe, and its ivars hold nothing thread-bound beyond wry's delegates, which
/// wry itself keeps alive.
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
    on_response: Box<dyn Fn(MainFrameResponse) -> bool>,
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
                on_response,
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
            "[school-download] WebKit hooks installed: new-window forwarded={}, pdf-save={}, response={}, download forwarded={}",
            check(ui_object, sel!(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:)),
            check(ui_object, sel!(_webView:saveDataToFile:suggestedFilename:mimeType:originatingURL:)),
            check(navigation_object, sel!(webView:decidePolicyForNavigationResponse:decisionHandler:)),
            check(navigation_object, sel!(webView:navigationResponse:didBecomeDownload:)),
        );
    }
    (ui.is_some() || navigation.is_some()).then_some(DelegateHandle {
        _ui: ui,
        _navigation: navigation,
    })
}
