//! macOS school-view host: the WebKit delegates that hand Core every fact of
//! a request and carry out what it decides, instead of wry's callbacks,
//! which pass only a URL. Main views wrap wry's delegates in shims that add
//! their own methods and forward everything else; popups get the same shims
//! with nothing underneath.
//!
//! - Navigation: `decidePolicyForNavigationAction` reports URL, method, frame,
//!   navigation type (`.formResubmitted` included) and gesture to Core and
//!   applies Allow / External / Deny / SkipEntry. Main-frame load failures
//!   are reported too: WebKit shows nothing and stays on the old page.
//! - Popups: `createWebViewWithConfiguration:` returns a web view built from
//!   WebKit's own configuration, so the POST body and `window.opener`
//!   survive. It covers its opener inside the web pane, is registered in Core
//!   as the opener's child, and `webViewDidClose` reports `window.close()`.
//! - Page dialogs: `alert()`, `confirm()`, `prompt()` as a sheet for the
//!   view the person is looking at, Cancel for a hidden one. `window.print()`
//!   reaches the private `_webView:printFrame:pdfFirstPageSize:completionHandler:`
//!   and opens the system print panel, foreground only.
//! - Files: the download (↓) button of WebKit's PDF viewer reaches the
//!   private `_webView:saveDataToFile:suggestedFilename:mimeType:originatingURL:`.
//!   A displayable `Content-Disposition: attachment` response (Moodle
//!   submissions, `?forcedownload=1`), which WebKit would commit as an empty
//!   page, is fetched as a `WKDownload` and loaded back in place. Other
//!   attachments stay with wry's download handling in main views and are
//!   saved through the shell in popups. Private API: if WebKit renames a
//!   selector its feature goes back to doing nothing; nothing crashes.
//!
//! WebKit caches which delegate methods exist when a delegate is set, so each
//! shim also answers `respondsToSelector:` for wry's methods and forwards them.

use app_core::school_browser::{
    BrowserId, LoadFailure, NavigationDecision, NavigationKind, NewWindowDecision, WindowFeatures,
};
use block2::{DynBlock, RcBlock};
use objc2::rc::{Retained, Weak};
use objc2::runtime::{
    AnyObject, AnyProtocol, Bool, NSObject, NSObjectProtocol, ProtocolObject, Sel,
};
use objc2::{
    define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, Message,
};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAutoresizingMaskOptions, NSModalResponse, NSPrintInfo,
    NSResponder, NSTextField, NSView,
};
use objc2_foundation::{
    ns_string, NSData, NSError, NSHTTPURLResponse, NSPoint, NSRect, NSSize, NSString, NSURLRequest,
    NSURLResponse, NSURL,
};
use objc2_web_kit::{
    WKBackForwardListItem, WKDownload, WKDownloadDelegate, WKFrameInfo, WKNavigation,
    WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate, WKNavigationResponse,
    WKNavigationResponsePolicy, WKNavigationType, WKUIDelegate, WKWebView, WKWebViewConfiguration,
    WKWindowFeatures,
};
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::null_mut;
use std::rc::Rc;
use url::Url;

/// What the PDF viewer asked to save.
pub struct SavedData {
    pub bytes: Vec<u8>,
    pub suggested_name: String,
    pub mime_type: String,
    /// The PDF's own URL (absolute string), when WebKit provides one.
    pub origin: Option<String>,
}

/// A navigation as WebKit describes it, before the shell adds the page.
pub struct Request {
    pub url: Url,
    pub method: Option<String>,
    pub main_frame: bool,
    pub kind: NavigationKind,
    pub user_gesture: bool,
}

/// The shell side of one school view and the popups its pages open. Every
/// call arrives on the main thread.
pub trait Host {
    /// Whether this is the school view the person is looking at.
    fn foreground(&self) -> bool;
    fn navigation(&self, view: BrowserId, request: Request) -> NavigationDecision;
    fn new_window(
        &self,
        opener: BrowserId,
        request: Request,
        features: WindowFeatures,
    ) -> NewWindowDecision;
    fn popup_loaded(&self, popup: BrowserId, url: Url);
    fn failed(&self, view: BrowserId, url: Option<Url>, reason: LoadFailure, message: String);
    /// The popup's page closed it (`window.close()`).
    fn popup_closed(&self, popup: BrowserId);
    fn save(&self, data: SavedData);
    fn open_external(&self, url: Url);
    /// Where a file a popup downloads is written; `None` refuses it.
    fn download_destination(&self, url: Option<Url>, suggested: &str) -> Option<PathBuf>;
    fn download_finished(&self, path: PathBuf, saved: bool);
    /// `view`'s navigation became a download, shown in place or saved;
    /// `had_page` when the view had committed a page before.
    fn became_download(&self, view: BrowserId, had_page: bool, shown: bool);
}

pub struct UiIvars {
    /// wry's delegate for a main view; none for a popup. WebKit holds
    /// delegates weakly, but wry keeps its own reference too; this one keeps
    /// forwarding valid regardless.
    original: Option<Retained<ProtocolObject<dyn WKUIDelegate>>>,
    host: Rc<dyn Host>,
    view: BrowserId,
}

pub struct NavigationIvars {
    original: Option<Retained<ProtocolObject<dyn WKNavigationDelegate>>>,
    host: Rc<dyn Host>,
    view: BrowserId,
    /// The history entry of the last committed page. A form resubmission
    /// whose entry differs travels through history instead of reloading.
    committed: RefCell<Option<Retained<WKBackForwardListItem>>>,
    /// Files being fetched. `WKDownload.delegate` is weak; this shim is the
    /// delegate and lives as long as the view.
    attachments: RefCell<Vec<Attachment>>,
}

struct Attachment {
    download: Retained<WKDownload>,
    /// Shown in place (a displayable attachment) or saved (a popup's file).
    shown: bool,
    /// The view whose navigation became this download, where a shown file
    /// goes. Not `download.webView()`: for a popup that has not committed a
    /// page yet, WebKit names the opener there.
    target: Weak<WKWebView>,
    /// The file and, for one shown in place, the response it holds.
    file: Option<(PathBuf, Option<Retained<NSURLResponse>>)>,
}

fn responds(object: &AnyObject, selector: Sel) -> bool {
    unsafe { msg_send![object, respondsToSelector: selector] }
}

fn forwarding_target(object: Option<&AnyObject>, selector: Sel) -> *mut AnyObject {
    match object {
        Some(object) if responds(object, selector) => object as *const AnyObject as *mut AnyObject,
        _ => null_mut(),
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

fn request_url(request: &NSURLRequest) -> Option<Url> {
    Url::parse(&request.URL()?.absoluteString()?.to_string()).ok()
}

/// The facts of a navigation action. `main_frame` for a new window, whose
/// target frame does not exist yet.
fn request(
    web_view: &WKWebView,
    action: &WKNavigationAction,
    committed: Option<&WKBackForwardListItem>,
) -> Option<Request> {
    let request = unsafe { action.request() };
    let url = request_url(&request)?;
    let main_frame =
        unsafe { action.targetFrame() }.is_none_or(|frame| unsafe { frame.isMainFrame() });
    let kind = match unsafe { action.navigationType() } {
        WKNavigationType::LinkActivated => NavigationKind::Link,
        WKNavigationType::FormSubmitted => NavigationKind::FormSubmit,
        WKNavigationType::BackForward => NavigationKind::BackForward,
        WKNavigationType::Reload => NavigationKind::Reload,
        // WebKit has already moved the list to the entry it is going to when
        // it asks, so an entry other than the committed one is history.
        WKNavigationType::FormResubmitted => {
            let current = unsafe { web_view.backForwardList().currentItem() };
            let from_history = main_frame
                && match (current.as_deref(), committed) {
                    (Some(current), Some(committed)) => !std::ptr::eq(current, committed),
                    (current, committed) => current.is_some() != committed.is_some(),
                };
            NavigationKind::FormResubmit { from_history }
        }
        WKNavigationType::Other => NavigationKind::Other,
        _ => NavigationKind::Unknown,
    };
    // `_isUserInitiated` is private; without it, WebKit's own link and form
    // types say the person acted.
    let gesture_selector = sel!(_isUserInitiated);
    let user_gesture =
        responds(action, gesture_selector) && unsafe { msg_send![action, _isUserInitiated] };
    Some(Request {
        url,
        method: request.HTTPMethod().map(|method| method.to_string()),
        main_frame,
        kind,
        user_gesture,
    })
}

/// A coarse reason for a failed load. Cancellations (a newer load, a policy
/// decision, a response that became a download or a plug-in) are not failures.
fn failure(error: &NSError) -> LoadFailure {
    let domain = error.domain().to_string();
    let code = error.code();
    match (domain.as_str(), code) {
        ("NSURLErrorDomain", -999) => LoadFailure::Cancelled,
        ("WebKitErrorDomain", 102 | 204) => LoadFailure::Cancelled,
        ("NSURLErrorDomain", -1009 | -1018 | -1020) => LoadFailure::Offline,
        ("NSURLErrorDomain", -1003 | -1006) => LoadFailure::HostNotFound,
        ("NSURLErrorDomain", -1001 | -1004 | -1005) => LoadFailure::Unreachable,
        ("NSURLErrorDomain", -1022) => LoadFailure::Cleartext,
        ("NSURLErrorDomain", -1206..=-1200) => LoadFailure::Insecure,
        _ => LoadFailure::Other,
    }
}

fn failing_url(error: &NSError) -> Option<Url> {
    let info = error.userInfo();
    let value = info.objectForKey(ns_string!("NSErrorFailingURLStringKey"))?;
    Url::parse(&value.downcast_ref::<NSString>()?.to_string()).ok()
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

/// A popup's web view and the shims that keep it reporting. WebKit holds
/// delegates weakly; this registry holds them for as long as the popup is open.
struct Popup {
    id: BrowserId,
    web_view: Retained<WKWebView>,
    _ui: Retained<SchoolUiDelegate>,
    navigation: Retained<SchoolNavigationDelegate>,
}

thread_local! {
    /// Popups exist on the main thread only, as their web views do.
    static POPUPS: RefCell<Vec<Popup>> = const { RefCell::new(Vec::new()) };
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = UiIvars]
    pub struct SchoolUiDelegate;

    unsafe impl NSObjectProtocol for SchoolUiDelegate {}

    // Every WKUIDelegate method is optional: the shim implements windows and
    // page dialogs itself and forwards the rest to wry's delegate.
    unsafe impl WKUIDelegate for SchoolUiDelegate {
        #[unsafe(method_id(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        fn create_web_view(
            &self,
            web_view: &WKWebView,
            configuration: &WKWebViewConfiguration,
            action: &WKNavigationAction,
            features: &WKWindowFeatures,
        ) -> Option<Retained<WKWebView>> {
            self.window_for(web_view, configuration, action, features)
        }

        #[unsafe(method(webViewDidClose:))]
        fn did_close(&self, _web_view: &WKWebView) {
            let view = self.ivars().view;
            if close_popup(view) {
                self.ivars().host.popup_closed(view);
            }
        }

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
            self.ivars().host.save(SavedData {
                bytes: data.to_vec(),
                suggested_name: suggested_name.map(|name| name.to_string()).unwrap_or_default(),
                mime_type: mime_type.map(|mime| mime.to_string()).unwrap_or_default(),
                origin,
            });
        }

        /// `window.print()`: the system print panel as a sheet, for the view
        /// the person is looking at. A hidden view prints nothing.
        #[unsafe(method(_webView:printFrame:pdfFirstPageSize:completionHandler:))]
        fn print_frame(
            &self,
            web_view: &WKWebView,
            _frame: Option<&AnyObject>,
            _size: NSSize,
            handler: &DynBlock<dyn Fn()>,
        ) {
            let window = web_view.window().filter(|window| window.isVisible());
            if let (true, Some(window)) = (self.ivars().host.foreground(), window) {
                let operation =
                    unsafe { web_view.printOperationWithPrintInfo(&NSPrintInfo::sharedPrintInfo()) };
                // WebKit prints blank pages unless the operation's view has
                // the web view's size.
                if let Some(view) = operation.view() {
                    view.setFrame(web_view.bounds());
                }
                unsafe {
                    operation.runOperationModalForWindow_delegate_didRunSelector_contextInfo(
                        &window,
                        None,
                        None,
                        null_mut(),
                    )
                };
            }
            handler.call(());
        }

        #[unsafe(method(respondsToSelector:))]
        fn responds_to_selector(&self, selector: Sel) -> bool {
            let own: bool = unsafe { msg_send![super(self), respondsToSelector: selector] };
            own || self.original().is_some_and(|original| responds(original, selector))
        }

        #[unsafe(method(forwardingTargetForSelector:))]
        fn forwarding_target_for_selector(&self, selector: Sel) -> *mut AnyObject {
            forwarding_target(self.original(), selector)
        }

        #[unsafe(method(conformsToProtocol:))]
        fn conforms_to_protocol(&self, protocol: &AnyProtocol) -> bool {
            let own: bool = unsafe { msg_send![super(self), conformsToProtocol: protocol] };
            own || self
                .original()
                .is_some_and(|original| unsafe { msg_send![original, conformsToProtocol: protocol] })
        }
    }
);

impl SchoolUiDelegate {
    fn new(
        mtm: MainThreadMarker,
        original: Option<Retained<ProtocolObject<dyn WKUIDelegate>>>,
        host: Rc<dyn Host>,
        view: BrowserId,
    ) -> Retained<Self> {
        let shim = mtm.alloc::<Self>().set_ivars(UiIvars {
            original,
            host,
            view,
        });
        unsafe { msg_send![super(shim), init] }
    }

    /// The popup WebKit asks for, if Core gives it one.
    fn window_for(
        &self,
        web_view: &WKWebView,
        configuration: &WKWebViewConfiguration,
        action: &WKNavigationAction,
        features: &WKWindowFeatures,
    ) -> Option<Retained<WKWebView>> {
        let ivars = self.ivars();
        let request = request(web_view, action, None)?;
        let features = WindowFeatures {
            width: unsafe { features.width() }.map(|value| value.doubleValue()),
            height: unsafe { features.height() }.map(|value| value.doubleValue()),
        };
        match ivars.host.new_window(ivars.view, request, features) {
            NewWindowDecision::Popup(id) => Some(open_popup(
                self.mtm(),
                web_view,
                configuration,
                id,
                ivars.host.clone(),
            )),
            NewWindowDecision::External(url) => {
                ivars.host.open_external(url);
                None
            }
            NewWindowDecision::Deny(_) => None,
        }
    }

    fn original(&self) -> Option<&AnyObject> {
        self.ivars().original.as_deref().map(AsRef::as_ref)
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
            suggested_name: &NSString,
            handler: &DynBlock<dyn Fn(*mut NSURL)>,
        ) {
            let ivars = self.ivars();
            let mut attachments = ivars.attachments.borrow_mut();
            let Some(attachment) = attachments
                .iter_mut()
                .find(|attachment| std::ptr::eq(&*attachment.download, download))
            else {
                handler.call((null_mut(),));
                return;
            };
            let file = if attachment.shown {
                Some(
                    std::env::temp_dir()
                        .join(format!("isct-companion-attachment-{}", uuid::Uuid::new_v4())),
                )
            } else {
                let url = response
                    .URL()
                    .and_then(|url| url.absoluteString())
                    .and_then(|url| Url::parse(&url.to_string()).ok());
                ivars
                    .host
                    .download_destination(url, &suggested_name.to_string())
            };
            let destination = file.as_ref().and_then(|file| file.to_str()).map(|path| {
                NSURL::fileURLWithPath_isDirectory(&NSString::from_str(path), false)
            });
            match (file, destination) {
                (Some(file), Some(url)) => {
                    let shown = attachment.shown.then(|| response.retain());
                    attachment.file = Some((file, shown));
                    handler.call((Retained::as_ptr(&url).cast_mut(),));
                }
                _ => handler.call((null_mut(),)),
            }
        }

        #[unsafe(method(downloadDidFinish:))]
        fn download_did_finish(&self, download: &WKDownload) {
            let Some((file, response, target)) = self.take(download) else {
                return;
            };
            let Some(response) = response else {
                self.ivars().host.download_finished(file, true);
                return;
            };
            let bytes = std::fs::read(&file);
            let _ = std::fs::remove_file(&file);
            let (Ok(bytes), Some(web_view), Some(url)) = (bytes, target.load(), response.URL())
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
                "[school-download] download failed: {}",
                error.localizedDescription()
            );
            match self.take(download) {
                Some((file, Some(_), _)) => {
                    let _ = std::fs::remove_file(file);
                }
                Some((file, None, _)) => self.ivars().host.download_finished(file, false),
                None => {}
            }
        }
    }

    impl SchoolNavigationDelegate {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide_policy_for_action(
            &self,
            web_view: &WKWebView,
            action: &WKNavigationAction,
            handler: &DynBlock<dyn Fn(WKNavigationActionPolicy)>,
        ) {
            let ivars = self.ivars();
            // `<a download>` and the like: wry hands these to the shell's
            // download handling; a popup saves them through the host.
            if unsafe { action.shouldPerformDownload() } {
                match &ivars.original {
                    Some(original) => unsafe {
                        original.webView_decidePolicyForNavigationAction_decisionHandler(
                            web_view, action, handler,
                        )
                    },
                    None => handler.call((WKNavigationActionPolicy::Download,)),
                }
                return;
            }
            let committed = ivars.committed.borrow().clone();
            let Some(request) = request(web_view, action, committed.as_deref()) else {
                handler.call((WKNavigationActionPolicy::Cancel,));
                return;
            };
            match ivars.host.navigation(ivars.view, request) {
                NavigationDecision::Allow => handler.call((WKNavigationActionPolicy::Allow,)),
                NavigationDecision::External(url) => {
                    handler.call((WKNavigationActionPolicy::Cancel,));
                    ivars.host.open_external(url);
                }
                NavigationDecision::Deny(_) => handler.call((WKNavigationActionPolicy::Cancel,)),
                NavigationDecision::SkipEntry => {
                    // The entry WebKit was going to; once the cancel has put
                    // the list back, step past it.
                    let target = unsafe { web_view.backForwardList().currentItem() };
                    handler.call((WKNavigationActionPolicy::Cancel,));
                    if let Some(target) = target {
                        let _: () = unsafe {
                            msg_send![
                                self,
                                performSelector: sel!(skipPast:),
                                withObject: &*target,
                                afterDelay: 0.2f64
                            ]
                        };
                    }
                }
            }
        }

        /// Moves one entry beyond `target` in the direction it lies from the
        /// current entry, so the form it would resend is never sent.
        #[unsafe(method(skipPast:))]
        fn skip_past(&self, target: &WKBackForwardListItem) {
            let Some(web_view) = self.web_view() else {
                return;
            };
            let list = unsafe { web_view.backForwardList() };
            let back = unsafe { list.backList() };
            let forward = unsafe { list.forwardList() };
            let offset = (0..back.count())
                .find(|index| std::ptr::eq(&*back.objectAtIndex(*index), target))
                .map(|index| index as isize - back.count() as isize - 1)
                .or_else(|| {
                    (0..forward.count())
                        .find(|index| std::ptr::eq(&*forward.objectAtIndex(*index), target))
                        .map(|index| index as isize + 2)
                });
            if let Some(item) = offset.and_then(|offset| unsafe { list.itemAtIndex(offset) }) {
                unsafe { web_view.goToBackForwardListItem(&item) };
            }
        }

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
            match &self.ivars().original {
                Some(original)
                    if original.respondsToSelector(sel!(webView:decidePolicyForNavigationResponse:decisionHandler:)) =>
                unsafe {
                    original.webView_decidePolicyForNavigationResponse_decisionHandler(
                        web_view, navigation, handler,
                    )
                },
                Some(_) => handler.call((WKNavigationResponsePolicy::Allow,)),
                // A popup saves what WebKit cannot show.
                None if !unsafe { navigation.canShowMIMEType() } => {
                    handler.call((WKNavigationResponsePolicy::Download,))
                }
                None => handler.call((WKNavigationResponsePolicy::Allow,)),
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
            let shown = displayable_attachment(navigation);
            match &self.ivars().original {
                Some(original) if !shown => {
                    if original.respondsToSelector(sel!(webView:navigationResponse:didBecomeDownload:)) {
                        unsafe {
                            original.webView_navigationResponse_didBecomeDownload(
                                web_view, navigation, download,
                            )
                        };
                    }
                }
                _ => self.adopt(download, shown, web_view),
            }
            self.report_download(shown);
        }

        #[unsafe(method(webView:navigationAction:didBecomeDownload:))]
        fn action_did_become_download(
            &self,
            web_view: &WKWebView,
            action: &WKNavigationAction,
            download: &WKDownload,
        ) {
            match &self.ivars().original {
                Some(original) => {
                    if original.respondsToSelector(sel!(webView:navigationAction:didBecomeDownload:)) {
                        unsafe {
                            original.webView_navigationAction_didBecomeDownload(
                                web_view, action, download,
                            )
                        };
                    }
                }
                None => self.adopt(download, false, web_view),
            }
            self.report_download(false);
        }

        #[unsafe(method(webView:didCommitNavigation:))]
        fn did_commit(&self, web_view: &WKWebView, navigation: Option<&WKNavigation>) {
            *self.ivars().committed.borrow_mut() = unsafe { web_view.backForwardList().currentItem() };
            if let Some(original) = self.forwards(sel!(webView:didCommitNavigation:)) {
                let _: () = unsafe { msg_send![original, webView: web_view, didCommitNavigation: navigation] };
            }
        }

        #[unsafe(method(webView:didFinishNavigation:))]
        fn did_finish(&self, web_view: &WKWebView, navigation: Option<&WKNavigation>) {
            let ivars = self.ivars();
            match self.forwards(sel!(webView:didFinishNavigation:)) {
                Some(original) => {
                    let _: () = unsafe { msg_send![original, webView: web_view, didFinishNavigation: navigation] };
                }
                // A main view reports loads through wry; a popup here.
                None if ivars.original.is_none() => {
                    if let Some(url) = unsafe { web_view.URL() }
                        .and_then(|url| url.absoluteString())
                        .and_then(|url| Url::parse(&url.to_string()).ok())
                    {
                        ivars.host.popup_loaded(ivars.view, url);
                    }
                }
                None => {}
            }
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn did_fail_provisional(
            &self,
            web_view: &WKWebView,
            navigation: Option<&WKNavigation>,
            error: &NSError,
        ) {
            self.report_failure(error);
            if let Some(original) = self.forwards(sel!(webView:didFailProvisionalNavigation:withError:)) {
                let _: () = unsafe {
                    msg_send![original, webView: web_view, didFailProvisionalNavigation: navigation, withError: error]
                };
            }
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn did_fail(&self, web_view: &WKWebView, navigation: Option<&WKNavigation>, error: &NSError) {
            self.report_failure(error);
            if let Some(original) = self.forwards(sel!(webView:didFailNavigation:withError:)) {
                let _: () = unsafe {
                    msg_send![original, webView: web_view, didFailNavigation: navigation, withError: error]
                };
            }
        }

        #[unsafe(method(respondsToSelector:))]
        fn responds_to_selector(&self, selector: Sel) -> bool {
            let own: bool = unsafe { msg_send![super(self), respondsToSelector: selector] };
            own || self.original().is_some_and(|original| responds(original, selector))
        }

        #[unsafe(method(forwardingTargetForSelector:))]
        fn forwarding_target_for_selector(&self, selector: Sel) -> *mut AnyObject {
            forwarding_target(self.original(), selector)
        }

        #[unsafe(method(conformsToProtocol:))]
        fn conforms_to_protocol(&self, protocol: &AnyProtocol) -> bool {
            let own: bool = unsafe { msg_send![super(self), conformsToProtocol: protocol] };
            own || self
                .original()
                .is_some_and(|original| unsafe { msg_send![original, conformsToProtocol: protocol] })
        }
    }
);

impl SchoolNavigationDelegate {
    fn new(
        mtm: MainThreadMarker,
        original: Option<Retained<ProtocolObject<dyn WKNavigationDelegate>>>,
        host: Rc<dyn Host>,
        view: BrowserId,
        web_view: &WKWebView,
    ) -> Retained<Self> {
        let shim = mtm.alloc::<Self>().set_ivars(NavigationIvars {
            original,
            host,
            view,
            committed: RefCell::new(unsafe { web_view.backForwardList().currentItem() }),
            attachments: RefCell::default(),
        });
        unsafe { msg_send![super(shim), init] }
    }

    fn original(&self) -> Option<&AnyObject> {
        self.ivars().original.as_deref().map(AsRef::as_ref)
    }

    /// wry's delegate, when it implements `selector`.
    fn forwards(&self, selector: Sel) -> Option<&AnyObject> {
        self.original()
            .filter(|original| responds(original, selector))
    }

    /// The view this shim serves: the popup in the registry, or the view
    /// whose delegate it is.
    fn web_view(&self) -> Option<Retained<WKWebView>> {
        let view = self.ivars().view;
        POPUPS
            .with_borrow(|popups| {
                popups
                    .iter()
                    .find(|popup| popup.id == view)
                    .map(|popup| popup.web_view.clone())
            })
            .or_else(|| {
                MAIN_VIEWS.with_borrow(|views| {
                    views
                        .iter()
                        .find(|(id, _)| *id == view)
                        .and_then(|(_, web_view)| web_view.load())
                })
            })
    }

    fn report_failure(&self, error: &NSError) {
        let reason = failure(error);
        if reason == LoadFailure::Cancelled {
            return;
        }
        let ivars = self.ivars();
        ivars.host.failed(
            ivars.view,
            failing_url(error),
            reason,
            error.localizedDescription().to_string(),
        );
    }

    fn adopt(&self, download: &WKDownload, shown: bool, target: &WKWebView) {
        unsafe { download.setDelegate(Some(ProtocolObject::from_ref(self))) };
        self.ivars().attachments.borrow_mut().push(Attachment {
            download: download.retain(),
            shown,
            target: Weak::new(target),
            file: None,
        });
    }

    /// Tells Core this view's navigation became a download; Core closes a
    /// popup it leaves with nothing to show.
    fn report_download(&self, shown: bool) {
        let ivars = self.ivars();
        let had_page = ivars.committed.borrow().is_some();
        ivars.host.became_download(ivars.view, had_page, shown);
    }

    /// Forgets a finished or failed download, returning its file, the
    /// response of one shown in place, and the view it belongs to. A closed
    /// popup's shim is released once its last download is done.
    #[allow(clippy::type_complexity)]
    fn take(
        &self,
        download: &WKDownload,
    ) -> Option<(PathBuf, Option<Retained<NSURLResponse>>, Weak<WKWebView>)> {
        let (attachment, drained) = {
            let mut attachments = self.ivars().attachments.borrow_mut();
            let index = attachments
                .iter()
                .position(|attachment| std::ptr::eq(&*attachment.download, download))?;
            (attachments.remove(index), attachments.is_empty())
        };
        if drained {
            DRAINING.with_borrow_mut(|draining| {
                draining.retain(|shim| !std::ptr::eq(&**shim, self));
            });
        }
        let (file, response) = attachment.file?;
        Some((file, response, attachment.target))
    }
}

thread_local! {
    /// Navigation shims of closed popups whose downloads are still running:
    /// `WKDownload.delegate` is weak, so the shim must outlive its popup
    /// until the file is written.
    static DRAINING: RefCell<Vec<Retained<SchoolNavigationDelegate>>> = const { RefCell::new(Vec::new()) };

    /// Main views by id, for the history step a resubmission skips. Weak:
    /// tauri owns them.
    static MAIN_VIEWS: RefCell<Vec<(BrowserId, Weak<WKWebView>)>> = const { RefCell::new(Vec::new()) };
}

/// Builds the popup WebKit asked for from its own configuration, covering
/// the opener inside the web pane and following its size.
fn open_popup(
    mtm: MainThreadMarker,
    opener: &WKWebView,
    configuration: &WKWebViewConfiguration,
    id: BrowserId,
    host: Rc<dyn Host>,
) -> Retained<WKWebView> {
    let web_view = unsafe {
        WKWebView::initWithFrame_configuration(mtm.alloc(), opener.bounds(), configuration)
    };
    web_view.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    let ui = SchoolUiDelegate::new(mtm, None, host.clone(), id);
    let navigation = SchoolNavigationDelegate::new(mtm, None, host, id, &web_view);
    unsafe {
        web_view.setUIDelegate(Some(ProtocolObject::from_ref(&*ui)));
        web_view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*navigation)));
    }
    opener.addSubview(&web_view);
    if let Some(window) = opener.window() {
        let responder: &NSResponder = &web_view;
        window.makeFirstResponder(Some(responder));
    }
    POPUPS.with_borrow_mut(|popups| {
        popups.push(Popup {
            id,
            web_view: web_view.clone(),
            _ui: ui,
            navigation,
        })
    });
    web_view
}

/// Takes a popup off the screen and out of the registry. False when it was
/// not open.
pub fn close_popup(id: BrowserId) -> bool {
    let popup = POPUPS.with_borrow_mut(|popups| {
        let index = popups.iter().position(|popup| popup.id == id)?;
        Some(popups.remove(index))
    });
    let Some(popup) = popup else {
        return false;
    };
    unsafe {
        popup.web_view.stopLoading();
    }
    popup.web_view.removeFromSuperview();
    if !popup.navigation.ivars().attachments.borrow().is_empty() {
        DRAINING.with_borrow_mut(|draining| draining.push(popup.navigation.clone()));
    }
    true
}

/// A command for a popup; false when it is not open.
pub fn popup_command(id: BrowserId, command: Command) -> bool {
    let Some(web_view) = POPUPS.with_borrow(|popups| {
        popups
            .iter()
            .find(|popup| popup.id == id)
            .map(|popup| popup.web_view.clone())
    }) else {
        return false;
    };
    run(&web_view, command);
    true
}

/// What the shell sends a popup, or a main view through its WKWebView.
pub enum Command {
    Navigate(Url),
    Reload,
    GoBack,
    GoForward,
    Evaluate(String),
}

fn run(web_view: &WKWebView, command: Command) {
    unsafe {
        match command {
            Command::Navigate(url) => {
                if let Some(url) = NSURL::URLWithString(&NSString::from_str(url.as_str())) {
                    web_view.loadRequest(&NSURLRequest::requestWithURL(&url));
                }
            }
            Command::Reload => {
                web_view.reload();
            }
            Command::GoBack => {
                web_view.goBack();
            }
            Command::GoForward => {
                web_view.goForward();
            }
            Command::Evaluate(script) => {
                web_view.evaluateJavaScript_completionHandler(&NSString::from_str(&script), None);
            }
        }
    }
}

/// Runs `command` on a main view's WKWebView.
///
/// # Safety
/// `web_view` must be a live `WKWebView` (tauri's `PlatformWebview::inner()`),
/// and this must run on the main thread (inside `Webview::with_webview`).
pub unsafe fn command(web_view: *mut c_void, command: Command) {
    if let Some(web_view) = unsafe { (web_view as *const WKWebView).as_ref() } {
        run(web_view, command);
    }
}

/// Keeps a main view's shims alive: WebKit's delegate properties are weak.
///
/// The handle lives in the shell's state next to the resident webview. It is
/// otherwise dropped at exit; releasing an NSObject off the main thread is
/// safe, and its ivars hold nothing thread-bound beyond wry's delegates, which
/// wry itself keeps alive, the host, and downloads still in flight.
pub struct DelegateHandle {
    _ui: Option<Retained<SchoolUiDelegate>>,
    _navigation: Option<Retained<SchoolNavigationDelegate>>,
}

// SAFETY: see the type's documentation; the shims are never messaged through
// the handle, only kept alive and eventually released.
unsafe impl Send for DelegateHandle {}

/// Wraps a main view's WKWebView delegates in the shims.
///
/// # Safety
/// `web_view` must be a live `WKWebView` (tauri's `PlatformWebview::inner()`),
/// and this must run on the main thread (inside `Webview::with_webview`).
pub unsafe fn install(
    web_view: *mut c_void,
    view: BrowserId,
    host: Rc<dyn Host>,
) -> Option<DelegateHandle> {
    let mtm = MainThreadMarker::new()?;
    let web_view = unsafe { (web_view as *const WKWebView).as_ref() }?;
    refuse_first_responder_while_hidden(web_view);
    MAIN_VIEWS.with_borrow_mut(|views| {
        views.retain(|(id, web_view)| id.site != view.site && web_view.load().is_some());
        views.push((view, Weak::new(web_view)));
    });
    let ui = unsafe { web_view.UIDelegate() }.map(|original| {
        let shim = SchoolUiDelegate::new(mtm, Some(original), host.clone(), view);
        unsafe { web_view.setUIDelegate(Some(ProtocolObject::from_ref(&*shim))) };
        shim
    });
    let navigation = unsafe { web_view.navigationDelegate() }.map(|original| {
        let shim = SchoolNavigationDelegate::new(mtm, Some(original), host, view, web_view);
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
            "[school-web] WebKit host installed: popups={}, print={}, pdf-save={}, confirm={}, navigation={}, failures={}, page-load forwarded={}",
            check(ui_object, sel!(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:)),
            check(ui_object, sel!(_webView:printFrame:pdfFirstPageSize:completionHandler:)),
            check(ui_object, sel!(_webView:saveDataToFile:suggestedFilename:mimeType:originatingURL:)),
            check(ui_object, sel!(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:)),
            check(navigation_object, sel!(webView:decidePolicyForNavigationAction:decisionHandler:)),
            check(navigation_object, sel!(webView:didFailProvisionalNavigation:withError:)),
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
