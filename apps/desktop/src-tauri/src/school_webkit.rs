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
//!   The same shim answers the page's `alert()`, `confirm()` and `prompt()`,
//!   which WebKit otherwise dismisses unseen (wry's delegate implements none
//!   of them): a sheet on the window for the view the person is looking at,
//!   Cancel for a hidden view.
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

use block2::{DynBlock, RcBlock};
use objc2::rc::{Retained, Weak};
use objc2::runtime::{
    AnyObject, AnyProtocol, Bool, NSObject, NSObjectProtocol, ProtocolObject, Sel,
};
use objc2::{
    define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, Message,
};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSModalResponse, NSResponder, NSTextField, NSView,
};
use objc2_foundation::{
    ns_string, NSData, NSError, NSHTTPURLResponse, NSPoint, NSRect, NSSize, NSString,
    NSURLRequest, NSURLResponse, NSURL,
};
use objc2_web_kit::{
    WKDownload, WKDownloadDelegate, WKFrameInfo, WKNavigationDelegate, WKNavigationResponse,
    WKNavigationResponsePolicy, WKUIDelegate, WKWebView,
};
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::null_mut;
use std::rc::Rc;

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
    /// Whether this is the school view the person is looking at, as the
    /// shell's presentation decided it.
    foreground: Box<dyn Fn() -> bool>,
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

/// A page dialog's answer, given to WebKit exactly once: `Some` is OK (with
/// the entered text for `prompt()`), `None` is Cancel. A dialog that ends
/// without an answer, never shown or torn down with its window, answers
/// Cancel when its last reference goes.
struct Reply(Cell<Option<SendAnswer>>);

type SendAnswer = Box<dyn FnOnce(Option<&NSString>)>;

impl Reply {
    fn new(send: impl FnOnce(Option<&NSString>) + 'static) -> Rc<Self> {
        Rc::new(Self(Cell::new(Some(Box::new(send)))))
    }

    fn send(&self, answer: Option<&NSString>) {
        if let Some(send) = self.0.take() {
            send(answer);
        }
    }
}

impl Drop for Reply {
    fn drop(&mut self) {
        self.send(None);
    }
}

enum Dialog<'a> {
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

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = UiIvars]
    pub struct SchoolUiDelegate;

    unsafe impl NSObjectProtocol for SchoolUiDelegate {}

    // Every WKUIDelegate method is optional: the shim implements the page
    // dialogs itself and forwards the rest to wry's delegate.
    unsafe impl WKUIDelegate for SchoolUiDelegate {
        #[unsafe(method(webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn run_alert(
            &self,
            web_view: &WKWebView,
            message: &NSString,
            frame: &WKFrameInfo,
            handler: &DynBlock<dyn Fn()>,
        ) {
            let handler = handler.copy();
            let reply = Reply::new(move |_| handler.call(()));
            self.present(web_view, message, frame, Dialog::Alert, reply);
        }

        #[unsafe(method(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn run_confirm(
            &self,
            web_view: &WKWebView,
            message: &NSString,
            frame: &WKFrameInfo,
            handler: &DynBlock<dyn Fn(Bool)>,
        ) {
            let handler = handler.copy();
            let reply = Reply::new(move |answer| handler.call((Bool::new(answer.is_some()),)));
            self.present(web_view, message, frame, Dialog::Confirm, reply);
        }

        #[unsafe(method(webView:runJavaScriptTextInputPanelWithPrompt:defaultText:initiatedByFrame:completionHandler:))]
        fn run_prompt(
            &self,
            web_view: &WKWebView,
            prompt: &NSString,
            default_text: Option<&NSString>,
            frame: &WKFrameInfo,
            handler: &DynBlock<dyn Fn(*mut NSString)>,
        ) {
            let handler = handler.copy();
            let reply = Reply::new(move |answer| {
                handler.call((answer.map_or(null_mut(), |text| text as *const _ as *mut _),))
            });
            self.present(web_view, prompt, frame, Dialog::Prompt(default_text), reply);
        }
    }

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

    /// Shows a page dialog as a sheet on the window hosting the view, only
    /// for the school view the person is looking at. A hidden view (background
    /// work) raises no UI: dropping `reply` answers Cancel, as WebKit does
    /// without a delegate.
    fn present(
        &self,
        web_view: &WKWebView,
        message: &NSString,
        frame: &WKFrameInfo,
        dialog: Dialog<'_>,
        reply: Rc<Reply>,
    ) {
        if !(self.ivars().foreground)() {
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
                log::error!("[school-download] attachment could not be shown");
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
            log::error!(
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
    foreground: Box<dyn Fn() -> bool>,
) -> Option<DelegateHandle> {
    let mtm = MainThreadMarker::new()?;
    let web_view = unsafe { (web_view as *const WKWebView).as_ref() }?;
    refuse_first_responder_while_hidden(web_view);
    let ui = unsafe { web_view.UIDelegate() }.map(|original| {
        let shim = mtm
            .alloc::<SchoolUiDelegate>()
            .set_ivars(UiIvars {
                original,
                on_save,
                foreground,
            });
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
        log::info!(
            "[school-download] WebKit hooks installed: new-window forwarded={}, pdf-save={}, confirm={}, attachment={}, page-load forwarded={}",
            check(ui_object, sel!(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:)),
            check(ui_object, sel!(_webView:saveDataToFile:suggestedFilename:mimeType:originatingURL:)),
            check(ui_object, sel!(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:)),
            check(navigation_object, sel!(webView:navigationResponse:didBecomeDownload:)),
            check(navigation_object, sel!(webView:didFinishNavigation:)),
        );
    }
    (ui.is_some() || navigation.is_some()).then_some(DelegateHandle {
        _ui: ui,
        _navigation: navigation,
    })
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

/// Hidden school views load pages whose scripts focus a field on load
/// (ASP.NET `WebForm_AutoFocus`, sign-in forms). WebKit then makes that view
/// the window's first responder, and the visible app stops receiving keys
/// until it is clicked. wry's view class therefore refuses first-responder
/// status while a view is hidden; a shown view keeps WebKit's answer. It is
/// installed once on the class, so the hidden LMS downloader view is covered
/// too.
fn refuse_first_responder_while_hidden(web_view: &WKWebView) {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        // wry registers its class under a versioned module path, and the
        // title observer adds a KVO subclass on top; walk to the real class.
        let mut class = web_view.class();
        while class.name().to_bytes().starts_with(b"NSKVONotifying_") {
            let Some(superclass) = class.superclass() else {
                return;
            };
            class = superclass;
        }
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
    });
}
