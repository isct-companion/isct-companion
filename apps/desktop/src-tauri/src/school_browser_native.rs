//! Native school-view transport and presentation. Core owns every domain transition.
use crate::{
    lifecycle::SchoolSite,
    school_web::{apply_bounds, create_webview, NativeWebview},
};
use app_core::{runtime_tasks::RuntimeTasks, school_browser::*};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, Weak,
    },
};
use tauri::AppHandle;
#[cfg(target_os = "macos")]
use tauri_plugin_opener::OpenerExt;
use tokio::sync::mpsc;
use url::Url;

pub use app_core::school_browser::SchoolWebResult;
#[cfg(target_os = "macos")]
pub(crate) const SCHOOL_DATA_STORE: [u8; 16] = [
    0x63, 0x61, 0x6d, 0x70, 0x75, 0x73, 0x2d, 0x6c, 0x69, 0x66, 0x65, 0x2d, 0x77, 0x65, 0x62, 0x31,
];
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolWebPolicy {
    pub restore_across_restarts: bool,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct WebBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Default for WebBounds {
    fn default() -> Self {
        Self {
            x: 72.,
            y: 60.,
            width: 900.,
            height: 620.,
        }
    }
}
impl WebBounds {
    pub(crate) fn sanitized(self) -> Self {
        let clamp = |n: f64| {
            if n.is_finite() {
                n.clamp(0., 20_000.)
            } else {
                0.
            }
        };
        Self {
            x: clamp(self.x),
            y: clamp(self.y),
            width: clamp(self.width).max(1.),
            height: clamp(self.height).max(1.),
        }
    }
}
/// What wry reports for a main view. Navigation and windows go through the
/// platform host instead (`school_webkit.rs`; `school_windows.rs` for now).
#[derive(Clone)]
pub(crate) struct ViewCallbacks {
    pub loaded: Arc<dyn Fn(&str) + Send + Sync>,
    pub title: Arc<dyn Fn(&str) + Send + Sync>,
    pub download: Arc<dyn Fn(tauri::webview::DownloadEvent<'_>) -> bool + Send + Sync>,
}
struct View {
    id: BrowserId,
    native: NativeWebview,
    page: Arc<AtomicU64>,
    url: Arc<Mutex<Option<Url>>>,
    /// The WebKit host's shims for this view.
    #[cfg(target_os = "macos")]
    _host: Option<crate::school_webkit::DelegateHandle>,
}
/// A popup Core registered as a child of a view. The window itself lives
/// with the platform host.
pub(crate) struct PopupRecord {
    pub(crate) page: AtomicU64,
    pub(crate) url: Mutex<Option<Url>>,
    #[cfg(windows)]
    pub(crate) window: Option<tauri::WebviewWindow>,
}
#[derive(Default)]
pub(crate) struct Presentation {
    views: HashMap<SchoolSite, View>,
    pub(crate) popups: HashMap<BrowserId, PopupRecord>,
    visible: Option<SchoolSite>,
    bounds: WebBounds,
}
enum Operation {
    Ensure(BrowserId, Url),
    Dispatch(BrowserCommand),
    Dispose(BrowserId),
}
/// This queue serializes native dispatch only. It has no login/pull/session policy.
pub struct NativeBrowser {
    app: Mutex<Option<AppHandle>>,
    listener: Mutex<Option<Weak<dyn SchoolBrowserListener>>>,
    pub(crate) presentation: Mutex<Presentation>,
    sender: mpsc::UnboundedSender<Operation>,
    tasks: RuntimeTasks,
    downloads: crate::school_downloads::SchoolDownloads,
}
impl NativeBrowser {
    pub fn new(app: AppHandle) -> Result<Arc<Self>, BrowserError> {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let this = Arc::new(Self {
            app: Mutex::new(Some(app)),
            listener: Mutex::new(None),
            presentation: Mutex::new(Presentation::default()),
            sender,
            tasks: RuntimeTasks::default(),
            downloads: Default::default(),
        });
        let weak = Arc::downgrade(&this);
        let tasks = this.tasks.clone();
        let started = this.tasks.spawn(async move {
            loop {
                let operation = tokio::select! {
                    biased;
                    _ = tasks.closed() => break,
                    operation = receiver.recv() => match operation {
                        Some(operation) => operation,
                        None => break,
                    },
                };
                let Some(native) = weak.upgrade() else { break };
                // Platform implementations marshal WebView operations to their UI thread.
                let _ = tokio::task::spawn_blocking(move || native.perform(operation)).await;
            }
        });
        // Without the operation loop every command would be queued and never run.
        if !started {
            return Err(BrowserError::Unavailable(
                "The school view queue could not start.".into(),
            ));
        }
        Ok(this)
    }
    pub(crate) fn listener(&self) -> Option<Arc<dyn SchoolBrowserListener>> {
        self.listener.lock().ok()?.as_ref()?.upgrade()
    }
    pub(crate) fn app(&self) -> Result<AppHandle, BrowserError> {
        self.app
            .lock()
            .map_err(|_| BrowserError::Stopped)?
            .clone()
            .ok_or(BrowserError::Stopped)
    }
    fn send(&self, op: Operation) -> Result<(), BrowserError> {
        if self.tasks.is_closed() {
            return Err(BrowserError::Stopped);
        }
        self.sender.send(op).map_err(|_| BrowserError::Stopped)
    }
    fn perform(self: Arc<Self>, op: Operation) {
        let (id, cause, result) = match op {
            Operation::Ensure(id, url) => {
                if let Err(error) = self.create(id, url) {
                    log::error!(
                        "[school-web] {} could not be created: {error}",
                        id.site.webview_label()
                    );
                    // Core forgets this generation and creates a new view next time.
                    self.discard(id);
                    if let Some(listener) = self.listener() {
                        listener.closed(id)
                    }
                }
                return;
            }
            Operation::Dispatch(command) if self.is_popup(command.browser) => {
                let id = command.browser;
                if let Err(error) = self.popup_dispatch(command) {
                    log::warn!("[school-web] popup command not sent: {error}");
                    if let Some(listener) = self.listener() {
                        listener.closed(id);
                    }
                }
                return;
            }
            Operation::Dispatch(command) => {
                let id = command.browser;
                let cause = Some(command.action);
                let stop = matches!(command.command, BrowserAction::Stop);
                let result = self.with_view(id, |view| match command.command {
                    BrowserAction::Navigate(url) => view.navigate(url).map_err(|e| e.to_string()),
                    BrowserAction::Reload => view.reload().map_err(|e| e.to_string()),
                    BrowserAction::GoBack => crate::school_web::go(&view, true),
                    BrowserAction::GoForward => crate::school_web::go(&view, false),
                    BrowserAction::Eval(script) => view.eval(&script).map_err(|e| e.to_string()),
                    // `window.stop()` in the script ends the loads: Tauri's
                    // webview has no stop call of its own.
                    BrowserAction::Stop => view.eval(SCHOOL_STOP).map_err(|e| e.to_string()),
                });
                if stop && result.is_ok() {
                    // Behind the script on the main thread, so the page
                    // callbacks WebKit delivered before it reach Core first.
                    let native = self.clone();
                    let action = command.action;
                    let confirmed = self.app().and_then(|app| {
                        app.run_on_main_thread(move || {
                            if let (Some(page), Some(listener)) =
                                (native.current_page(id), native.listener())
                            {
                                listener.stopped(page, action);
                            }
                        })
                        .map_err(|e| BrowserError::Dispatch(e.to_string()))
                    });
                    if let Err(error) = confirmed {
                        log::warn!("[school-web] stop not confirmed: {error}");
                    }
                }
                (id, cause, result)
            }
            Operation::Dispose(id) if self.is_popup(id) => {
                self.close_popup(id);
                if let Some(listener) = self.listener() {
                    listener.closed(id)
                }
                return;
            }
            Operation::Dispose(id) => {
                self.discard(id);
                if let Some(listener) = self.listener() {
                    listener.closed(id)
                }
                return;
            }
        };
        if let Err(error) = result {
            let Some(listener) = self.listener() else {
                return;
            };
            // A command for a view that no longer exists cannot fail a page of it.
            match self.next_page(id) {
                Some(page) if !matches!(error, BrowserError::Unavailable(_)) => listener
                    .load_failed(LoadFailed {
                        page,
                        cause,
                        url: None,
                        reason: LoadFailure::Other,
                        message: error.to_string(),
                    }),
                _ => listener.closed(id),
            }
        }
    }
    /// Closes a view and every popup over it.
    fn discard(&self, id: BrowserId) {
        let (view, popups) = self
            .presentation
            .lock()
            .ok()
            .map(|mut state| {
                if !state.views.get(&id.site).is_some_and(|v| v.id == id) {
                    return (None, Vec::new());
                }
                let popups: Vec<_> = state
                    .popups
                    .keys()
                    .filter(|popup| popup.site == id.site)
                    .copied()
                    .collect();
                (state.views.remove(&id.site), popups)
            })
            .unwrap_or_default();
        for popup in popups {
            self.close_popup(popup);
        }
        if let Some(view) = view {
            #[cfg(target_os = "macos")]
            self.end_file_panels(move |owner| owner == id);
            let _ = view.native.close();
        }
    }
    /// Ends the file panels of the views and popups `owner` names, each
    /// answering Cancel.
    #[cfg(target_os = "macos")]
    fn end_file_panels(&self, owner: impl Fn(BrowserId) -> bool + Send + 'static) {
        if let Ok(app) = self.app() {
            let _ = app.run_on_main_thread(move || crate::school_webkit::end_file_panels(owner));
        }
    }
    /// Puts the popups back over their views. They are siblings of the
    /// views, so they follow a view's frame and visibility only through
    /// this. Queued behind the bounds and visibility just sent to the views,
    /// so it reads their new state.
    #[cfg(target_os = "macos")]
    pub(crate) fn arrange_popups(&self) {
        if let Ok(app) = self.app() {
            let _ = app.run_on_main_thread(crate::school_webkit::arrange_popups);
        }
    }
    fn is_popup(&self, id: BrowserId) -> bool {
        self.presentation
            .lock()
            .is_ok_and(|p| p.popups.contains_key(&id))
    }
    /// The page counter of a view or popup, read or advanced.
    fn page(&self, id: BrowserId, advance: bool) -> Option<PageId> {
        let step = |page: &AtomicU64| {
            if advance {
                page.fetch_add(1, Ordering::AcqRel) + 1
            } else {
                page.load(Ordering::Acquire)
            }
        };
        let sequence = self.presentation.lock().ok().and_then(|p| {
            match p.views.get(&id.site).filter(|v| v.id == id) {
                Some(view) => Some(step(&view.page)),
                None => p.popups.get(&id).map(|popup| step(&popup.page)),
            }
        })?;
        Some(PageId {
            browser: id,
            sequence,
        })
    }
    pub(crate) fn current_page(&self, id: BrowserId) -> Option<PageId> {
        self.page(id, false)
    }
    pub(crate) fn next_page(&self, id: BrowserId) -> Option<PageId> {
        self.page(id, true)
    }
    /// Records a popup Core just registered, before its first report.
    #[cfg(target_os = "macos")]
    pub(crate) fn popup_opened(&self, id: BrowserId) {
        if let Ok(mut p) = self.presentation.lock() {
            p.popups.insert(
                id,
                PopupRecord {
                    page: AtomicU64::new(0),
                    url: Mutex::new(None),
                },
            );
        }
    }
    /// A popup's page loaded: the next page of that popup.
    pub(crate) fn popup_loaded(&self, id: BrowserId, url: Url) {
        if let Ok(p) = self.presentation.lock() {
            if let Some(popup) = p.popups.get(&id) {
                if let Ok(mut current) = popup.url.lock() {
                    *current = Some(url.clone());
                }
            }
        }
        if let (Some(page), Some(listener)) = (self.next_page(id), self.listener()) {
            listener.page_loaded(PageLoaded {
                page,
                cause: None,
                url,
            });
        }
    }
    /// The popup's page or window closed it.
    pub(crate) fn popup_gone(&self, id: BrowserId) {
        let known = self
            .presentation
            .lock()
            .is_ok_and(|mut p| p.popups.remove(&id).is_some());
        if let (true, Some(listener)) = (known, self.listener()) {
            listener.closed(id);
        }
    }
    fn close_popup(&self, id: BrowserId) {
        let record = self
            .presentation
            .lock()
            .ok()
            .and_then(|mut p| p.popups.remove(&id));
        let Some(record) = record else {
            return;
        };
        #[cfg(target_os = "macos")]
        {
            let _ = record;
            if let Ok(app) = self.app() {
                let _ = app.run_on_main_thread(move || {
                    crate::school_webkit::close_popup(id);
                });
            }
        }
        #[cfg(windows)]
        if let Some(window) = record.window {
            let _ = window.destroy();
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        let _ = record;
    }
    fn popup_dispatch(&self, command: BrowserCommand) -> Result<(), BrowserError> {
        let id = command.browser;
        #[cfg(target_os = "macos")]
        {
            use crate::school_webkit::Command;
            let action = match command.command {
                BrowserAction::Navigate(url) => Command::Navigate(url),
                BrowserAction::Reload => Command::Reload,
                BrowserAction::GoBack => Command::GoBack,
                BrowserAction::GoForward => Command::GoForward,
                BrowserAction::Eval(script) => Command::Evaluate(script),
                // Core stops only the views its hidden work runs in.
                BrowserAction::Stop => return Ok(()),
            };
            let native = self.app()?;
            native
                .run_on_main_thread(move || {
                    crate::school_webkit::popup_command(id, action);
                })
                .map_err(|e| BrowserError::Dispatch(e.to_string()))
        }
        #[cfg(windows)]
        {
            let window = self
                .presentation
                .lock()
                .map_err(|_| BrowserError::Stopped)?
                .popups
                .get(&id)
                .and_then(|popup| popup.window.clone())
                .ok_or_else(|| BrowserError::Unavailable("The popup is gone.".into()))?;
            crate::school_windows::popup_command(&window, command.command)
                .map_err(BrowserError::Dispatch)
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let _ = command;
            Err(BrowserError::Unavailable(format!(
                "{id:?}: popups need a platform host."
            )))
        }
    }
    fn with_view(
        &self,
        id: BrowserId,
        work: impl FnOnce(NativeWebview) -> Result<(), String>,
    ) -> Result<(), BrowserError> {
        let view = self
            .presentation
            .lock()
            .map_err(|_| BrowserError::Stopped)?
            .views
            .get(&id.site)
            .filter(|v| v.id == id)
            .map(|v| v.native.clone())
            .ok_or_else(|| BrowserError::Unavailable("School view is unavailable.".into()))?;
        work(view).map_err(BrowserError::Dispatch)
    }
    fn create(self: &Arc<Self>, id: BrowserId, url: Url) -> Result<(), BrowserError> {
        if self
            .presentation
            .lock()
            .map_err(|_| BrowserError::Stopped)?
            .views
            .get(&id.site)
            .is_some_and(|view| view.id == id)
        {
            return Ok(());
        }
        let previous = self
            .presentation
            .lock()
            .map_err(|_| BrowserError::Stopped)?
            .views
            .remove(&id.site);
        if let Some(previous) = previous {
            previous
                .native
                .close()
                .map_err(|e| BrowserError::Dispatch(e.to_string()))?;
        }
        let app = self.app()?;
        let sequence = Arc::new(AtomicU64::new(0));
        let current = Arc::new(Mutex::new(Some(url.clone())));
        let weak = Arc::downgrade(self);
        let loaded_sequence = sequence.clone();
        let loaded_url = current.clone();
        let title_weak = weak.clone();
        let title_sequence = sequence.clone();
        let download_weak = weak.clone();
        #[cfg(windows)]
        let interim_weak = weak.clone();
        let callbacks = ViewCallbacks {
            loaded: Arc::new(move |raw| {
                let Ok(url) = Url::parse(raw) else { return };
                let page = PageId {
                    browser: id,
                    sequence: loaded_sequence.fetch_add(1, Ordering::AcqRel) + 1,
                };
                if let Ok(mut value) = loaded_url.lock() {
                    *value = Some(url.clone())
                }
                if let Some(listener) = weak.upgrade().and_then(|n| n.listener()) {
                    // WebKit does not expose a request ID here. Never label a late
                    // callback with the most recently dispatched action.
                    listener.page_loaded(PageLoaded {
                        page,
                        cause: None,
                        url,
                    });
                }
            }),
            title: Arc::new(move |title| {
                if let Some(listener) = title_weak.upgrade().and_then(|n| n.listener()) {
                    listener.title_changed(
                        PageId {
                            browser: id,
                            sequence: title_sequence.load(Ordering::Acquire),
                        },
                        title.to_string(),
                    )
                }
            }),
            download: Arc::new(move |event| {
                download_weak.upgrade().is_some_and(|native| {
                    native
                        .app()
                        .is_ok_and(|app| native.downloads.download(&app, id.site, event))
                })
            }),
        };
        #[cfg(windows)]
        let interim = crate::school_windows::Interim {
            native: interim_weak,
            view: id,
        };
        let view = create_webview(
            &app,
            id.site,
            url.as_str(),
            WebBounds::default(),
            false,
            callbacks,
            #[cfg(windows)]
            interim,
        )
        .map_err(BrowserError::Dispatch)?;
        #[cfg(target_os = "macos")]
        let host = self.install_host(id, &view);
        let (visible, bounds) = {
            let mut state = self
                .presentation
                .lock()
                .map_err(|_| BrowserError::Stopped)?;
            state.views.insert(
                id.site,
                View {
                    id,
                    native: view.clone(),
                    page: sequence,
                    url: current,
                    #[cfg(target_os = "macos")]
                    _host: host,
                },
            );
            (state.visible == Some(id.site), state.bounds)
        };
        if visible {
            apply_bounds(&view, bounds).map_err(BrowserError::Dispatch)?;
            view.show()
                .map_err(|e| BrowserError::Dispatch(e.to_string()))?;
            let _ = view.set_focus();
        } else {
            view.hide()
                .map_err(|e| BrowserError::Dispatch(e.to_string()))?;
        }
        Ok(())
    }

    /// Installs the WebKit host on a new main view. Its popups share the host.
    #[cfg(target_os = "macos")]
    fn install_host(
        self: &Arc<Self>,
        id: BrowserId,
        view: &NativeWebview,
    ) -> Option<crate::school_webkit::DelegateHandle> {
        let (sender, receiver) = std::sync::mpsc::channel();
        let host = WebKitHost {
            native: Arc::downgrade(self),
            site: id.site,
        };
        let installed = view.with_webview(move |platform| {
            // SAFETY: `inner()` is this view's live WKWebView, and
            // `with_webview` invokes this closure on the main thread.
            let handle = unsafe {
                crate::school_webkit::install(platform.inner(), id, std::rc::Rc::new(host))
            };
            let _ = sender.send(handle);
        });
        if let Err(error) = installed {
            log::error!(
                "[school-web] {:?} WebKit host unavailable: {error}",
                id.site
            );
            return None;
        }
        // `create` runs off the main thread, so waiting here cannot block it.
        receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .ok()
            .flatten()
    }

    fn apply_visibility(&self) -> Result<(), String> {
        let (views, visible, bounds) = {
            let p = self
                .presentation
                .lock()
                .map_err(|_| "School presentation unavailable.")?;
            (
                p.views
                    .iter()
                    .map(|(s, v)| (*s, v.native.clone()))
                    .collect::<Vec<_>>(),
                p.visible,
                p.bounds,
            )
        };
        // A file panel answers a page the person can see.
        #[cfg(target_os = "macos")]
        self.end_file_panels(move |owner| Some(owner.site) != visible);
        for (site, view) in views {
            if visible == Some(site) {
                apply_bounds(&view, bounds)?;
                view.show().map_err(|e| e.to_string())?;
                let _ = view.set_focus();
            } else {
                view.hide().map_err(|e| e.to_string())?;
            }
        }
        #[cfg(target_os = "macos")]
        self.arrange_popups();
        Ok(())
    }
    fn visible(&self, site: Option<SchoolSite>, bounds: Option<WebBounds>) -> Result<(), String> {
        {
            let mut p = self
                .presentation
                .lock()
                .map_err(|_| "School presentation unavailable.")?;
            p.visible = site;
            if let Some(bounds) = bounds {
                p.bounds = bounds.sanitized();
            }
        }
        self.downloads.set_visible(site);
        self.apply_visibility()
    }
}
impl SchoolBrowser for NativeBrowser {
    /// macOS reports every navigation fact and shows popups in the pane.
    /// The Windows interim reports neither: popups are separate windows.
    fn capabilities(&self) -> HostCapabilities {
        HostCapabilities {
            form_resubmission: cfg!(target_os = "macos"),
            inline_popups: cfg!(target_os = "macos"),
        }
    }
    fn attach(&self, listener: Weak<dyn SchoolBrowserListener>) -> Result<(), BrowserError> {
        *self.listener.lock().map_err(|_| BrowserError::Stopped)? = Some(listener);
        Ok(())
    }
    fn ensure(&self, id: BrowserId, url: Url) -> Result<(), BrowserError> {
        self.send(Operation::Ensure(id, url))
    }
    fn dispatch(&self, command: BrowserCommand) -> Result<(), BrowserError> {
        self.send(Operation::Dispatch(command))
    }
    fn current_url(&self, id: BrowserId) -> Result<Option<Url>, BrowserError> {
        let p = self
            .presentation
            .lock()
            .map_err(|_| BrowserError::Stopped)?;
        Ok(match p.views.get(&id.site).filter(|v| v.id == id) {
            Some(view) => view.url.lock().ok().and_then(|url| url.clone()),
            None => p
                .popups
                .get(&id)
                .and_then(|popup| popup.url.lock().ok().and_then(|url| url.clone())),
        })
    }
    fn dispose(&self, id: BrowserId) -> Result<(), BrowserError> {
        self.send(Operation::Dispose(id))
    }
    fn shutdown(&self) -> BrowserFuture<'_, ()> {
        Box::pin(async move {
            self.tasks.close();
            self.tasks.wait().await;
            let (views, popups) = {
                let mut p = self
                    .presentation
                    .lock()
                    .map_err(|_| BrowserError::Stopped)?;
                (
                    p.views
                        .drain()
                        .map(|(_, view)| view.native)
                        .collect::<Vec<_>>(),
                    p.popups.keys().copied().collect::<Vec<_>>(),
                )
            };
            for popup in popups {
                self.close_popup(popup);
            }
            #[cfg(target_os = "macos")]
            self.end_file_panels(|_| true);
            tokio::task::spawn_blocking(move || {
                for view in views {
                    let _ = view.close();
                }
            })
            .await
            .map_err(|error| BrowserError::Dispatch(error.to_string()))?;
            self.downloads.shutdown().await;
            if let Ok(mut app) = self.app.lock() {
                app.take();
            }
            if let Ok(mut listener) = self.listener.lock() {
                listener.take();
            }
            Ok(())
        })
    }
}

/// Command-facing presentation adapter. Session and pull state lives in Core.
#[derive(Clone)]
pub struct SchoolWebController {
    native: Arc<NativeBrowser>,
    pub(crate) engine: Arc<SchoolBrowserEngine>,
}
impl SchoolWebController {
    pub(crate) fn new(native: Arc<NativeBrowser>, engine: Arc<SchoolBrowserEngine>) -> Self {
        Self { native, engine }
    }
    pub fn configure(&self, app: &AppHandle, policy: SchoolWebPolicy) -> Result<bool, String> {
        use tauri::Manager;
        self.engine
            .configure_restore(
                policy.restore_across_restarts,
                app.path()
                    .app_data_dir()
                    .map_err(|e| e.to_string())?
                    .join("school-web-restore.json"),
            )
            .map_err(|e| e.to_string())
    }
    pub fn set_bounds(&self, bounds: WebBounds) -> Result<bool, String> {
        let bounds = bounds.sanitized();
        let view = {
            let mut state = self
                .native
                .presentation
                .lock()
                .map_err(|_| "School presentation unavailable.")?;
            state.bounds = bounds;
            state
                .visible
                .and_then(|site| state.views.get(&site).map(|view| view.native.clone()))
        };
        if let Some(view) = view {
            apply_bounds(&view, bounds)?;
            #[cfg(target_os = "macos")]
            self.native.arrange_popups();
        }
        Ok(true)
    }
    pub fn hide_all(&self) -> Result<bool, String> {
        self.native.visible(None, None)?;
        self.engine.user_browsing(None).map_err(|e| e.to_string())?;
        Ok(true)
    }
    pub async fn open(
        &self,
        site: SchoolSite,
        css: String,
        js: String,
        bounds: Option<WebBounds>,
        surface: &str,
        entry: SchoolWebEntry,
    ) -> Result<SchoolWebResult, String> {
        self.native.visible(Some(site), bounds)?;
        self.engine
            .set_customization(
                site,
                SchoolPageCustomization {
                    css,
                    javascript: js,
                },
            )
            .map_err(|e| e.to_string())?;
        let result = self.engine.open(site, surface, entry).await;
        if result.is_err() {
            let _ = self.native.visible(None, None);
            let _ = self.engine.user_browsing(None);
        }
        result.map_err(|e| e.to_string())
    }
    pub fn close_popup(&self, site: SchoolSite) -> Result<bool, String> {
        self.engine.close_popup(site).map_err(|e| e.to_string())
    }
    pub fn reveal_download(&self, app: &AppHandle, path: &str) -> Result<bool, String> {
        self.native.downloads.reveal(app, path)
    }
    pub fn set_zoom(&self, site: SchoolSite, scale: f64) -> Result<bool, String> {
        if !scale.is_finite() {
            return Err("School view scale must be finite.".into());
        }
        let view = self
            .native
            .presentation
            .lock()
            .map_err(|_| "School presentation unavailable.")?
            .views
            .get(&site)
            .map(|v| v.native.clone());
        let Some(view) = view else {
            return Ok(false);
        };
        let scale = scale.clamp(0.5, 2.);
        view.set_zoom(scale).map_err(|e| e.to_string())?;
        #[cfg(target_os = "macos")]
        self.native
            .app()
            .and_then(|app| {
                app.run_on_main_thread(move || crate::school_webkit::zoom_popups(site, scale))
                    .map_err(|e| BrowserError::Dispatch(e.to_string()))
            })
            .map_err(|e| e.to_string())?;
        Ok(true)
    }
}

/// The shell side of the WebKit host for one site's view and its popups.
#[cfg(target_os = "macos")]
struct WebKitHost {
    native: Weak<NativeBrowser>,
    site: SchoolSite,
}

#[cfg(target_os = "macos")]
impl crate::school_webkit::Host for WebKitHost {
    fn foreground(&self) -> bool {
        self.native.upgrade().is_some_and(|native| {
            native
                .presentation
                .lock()
                .is_ok_and(|p| p.visible == Some(self.site))
        })
    }

    fn navigation(
        &self,
        view: BrowserId,
        request: crate::school_webkit::Request,
    ) -> NavigationDecision {
        let Some(native) = self.native.upgrade() else {
            return NavigationDecision::Deny(DenyReason::Closed);
        };
        let (Some(page), Some(listener)) = (native.current_page(view), native.listener()) else {
            return NavigationDecision::Deny(DenyReason::Closed);
        };
        listener.decide_navigation(NavigationRequest {
            page,
            url: request.url,
            method: request.method,
            main_frame: request.main_frame,
            kind: request.kind,
            user_gesture: request.user_gesture,
        })
    }

    fn new_window(
        &self,
        opener: BrowserId,
        request: crate::school_webkit::Request,
        features: WindowFeatures,
    ) -> NewWindowDecision {
        let Some(native) = self.native.upgrade() else {
            return NewWindowDecision::Deny(DenyReason::Closed);
        };
        let (Some(page), Some(listener)) = (native.current_page(opener), native.listener()) else {
            return NewWindowDecision::Deny(DenyReason::Closed);
        };
        // WebKit does not expose the target window's name; it keeps reusing
        // named windows itself.
        let decision = listener.new_window(NewWindowRequest {
            opener: page,
            url: request.url,
            method: request.method,
            kind: request.kind,
            user_gesture: request.user_gesture,
            name: None,
            features,
        });
        if let NewWindowDecision::Popup(id) = decision {
            native.popup_opened(id);
        }
        decision
    }

    fn popup_loaded(&self, popup: BrowserId, url: Url) {
        if let Some(native) = self.native.upgrade() {
            native.popup_loaded(popup, url);
        }
    }

    fn history(&self, view: BrowserId, back: bool, forward: bool) {
        if let Some(listener) = self.native.upgrade().and_then(|native| native.listener()) {
            listener.history_changed(view, History { back, forward });
        }
    }

    fn failed(&self, view: BrowserId, url: Option<Url>, reason: LoadFailure, message: String) {
        let Some(native) = self.native.upgrade() else {
            return;
        };
        if let (Some(page), Some(listener)) = (native.next_page(view), native.listener()) {
            listener.load_failed(LoadFailed {
                page,
                cause: None,
                url,
                reason,
                message,
            });
        }
    }

    fn popup_closed(&self, popup: BrowserId) {
        if let Some(native) = self.native.upgrade() {
            native.popup_gone(popup);
        }
    }

    fn save(&self, data: crate::school_webkit::SavedData) {
        if let Some(native) = self.native.upgrade() {
            if let Ok(app) = native.app() {
                native.downloads.save_viewer_data(&app, self.site, data);
            }
        }
    }

    fn open_external(&self, url: Url) {
        if let Some(app) = self.native.upgrade().and_then(|native| native.app().ok()) {
            let _ = app.opener().open_url(url.as_str(), None::<&str>);
        }
    }

    fn download_destination(
        &self,
        url: Option<Url>,
        suggested: &str,
    ) -> Option<std::path::PathBuf> {
        let native = self.native.upgrade()?;
        let app = native.app().ok()?;
        native
            .downloads
            .popup_destination(&app, self.site, url.as_ref(), suggested)
    }

    fn became_download(&self, view: BrowserId, had_page: bool, shown: bool) {
        let Some(native) = self.native.upgrade() else {
            return;
        };
        if let (Some(page), Some(listener)) = (native.current_page(view), native.listener()) {
            listener.became_download(BecameDownload {
                page,
                had_page,
                disposition: if shown {
                    DownloadDisposition::Shown
                } else {
                    DownloadDisposition::Saved
                },
            });
        }
    }

    fn download_finished(&self, path: std::path::PathBuf, saved: bool) {
        if let Some(native) = self.native.upgrade() {
            if let Ok(app) = native.app() {
                native
                    .downloads
                    .popup_finished(&app, self.site, path, saved);
            }
        }
    }
}
