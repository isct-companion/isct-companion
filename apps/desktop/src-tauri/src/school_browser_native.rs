//! Native school-view transport and presentation. Core owns every domain transition.
use crate::{
    lifecycle::SchoolSite,
    school_web::{apply_bounds, create_webview, NativeWebview},
};
use isct_core::{runtime_tasks::RuntimeTasks, school_browser::*};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, Weak,
    },
};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;
use tokio::sync::mpsc;
use url::Url;

pub use isct_core::school_browser::{
    SchoolWebResult, KYOMU_POPUP as KYOMU_POPUP_SCRIPT, SCHOOL_LINKS as SCHOOL_LINKS_SCRIPT,
};
pub use isct_core::school_service::IngestOutcome;
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
#[derive(Clone)]
pub(crate) struct ViewCallbacks {
    pub loaded: Arc<dyn Fn(&str) + Send + Sync>,
    pub title: Arc<dyn Fn(&str) + Send + Sync>,
    pub navigation: Arc<dyn Fn(&Url) -> bool + Send + Sync>,
    pub new_window: Arc<dyn Fn(Url) + Send + Sync>,
    pub download: Arc<dyn Fn(tauri::webview::DownloadEvent<'_>) -> bool + Send + Sync>,
}
struct View {
    id: BrowserId,
    native: NativeWebview,
    page: Arc<AtomicU64>,
    url: Arc<Mutex<Option<Url>>>,
}
#[derive(Default)]
struct Presentation {
    views: HashMap<SchoolSite, View>,
    visible: Option<SchoolSite>,
    bounds: WebBounds,
}
enum Operation {
    Ensure(BrowserId, Url),
    Dispatch(BrowserCommand),
    Dispose(BrowserId),
    SameView(BrowserId, Url),
}
/// This queue serializes native dispatch only. It has no login/pull/session policy.
pub struct NativeBrowser {
    app: Mutex<Option<AppHandle>>,
    listener: Mutex<Option<Weak<dyn SchoolBrowserListener>>>,
    presentation: Mutex<Presentation>,
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
    fn listener(&self) -> Option<Arc<dyn SchoolBrowserListener>> {
        self.listener.lock().ok()?.as_ref()?.upgrade()
    }
    fn app(&self) -> Result<AppHandle, BrowserError> {
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
            Operation::Dispatch(command) => {
                let id = command.browser;
                let cause = Some(command.action);
                let result = self.with_view(id, |view| match command.command {
                    BrowserAction::Navigate(url) => view.navigate(url).map_err(|e| e.to_string()),
                    BrowserAction::Reload => view.reload().map_err(|e| e.to_string()),
                    BrowserAction::Eval(script) => view.eval(&script).map_err(|e| e.to_string()),
                });
                (id, cause, result)
            }
            Operation::SameView(id, url) => (
                id,
                None,
                self.with_view(id, |view| view.navigate(url).map_err(|e| e.to_string())),
            ),
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
                        message: error.to_string(),
                    }),
                _ => listener.closed(id),
            }
        }
    }
    fn discard(&self, id: BrowserId) {
        let view = self.presentation.lock().ok().and_then(|mut state| {
            if state.views.get(&id.site).is_some_and(|v| v.id == id) {
                state.views.remove(&id.site)
            } else {
                None
            }
        });
        if let Some(view) = view {
            let _ = view.native.close();
        }
    }
    fn next_page(&self, id: BrowserId) -> Option<PageId> {
        let sequence = self.presentation.lock().ok().and_then(|p| {
            p.views
                .get(&id.site)
                .filter(|v| v.id == id)
                .map(|v| v.page.fetch_add(1, Ordering::AcqRel) + 1)
        })?;
        Some(PageId {
            browser: id,
            sequence,
        })
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
        let nav_weak = weak.clone();
        let nav_sequence = sequence.clone();
        let popup_weak = weak.clone();
        let popup_sequence = sequence.clone();
        let download_weak = weak.clone();
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
            navigation: Arc::new(move |url| {
                nav_weak
                    .upgrade()
                    .and_then(|n| n.listener())
                    .is_some_and(|listener| {
                        listener.decide_navigation(
                            PageId {
                                browser: id,
                                sequence: nav_sequence.load(Ordering::Acquire),
                            },
                            url.clone(),
                            true,
                        ) == NavigationDecision::Allow
                    })
            }),
            new_window: Arc::new(move |url| {
                let Some(native) = popup_weak.upgrade() else {
                    return;
                };
                let Some(listener) = native.listener() else {
                    return;
                };
                match listener.new_window(
                    PageId {
                        browser: id,
                        sequence: popup_sequence.load(Ordering::Acquire),
                    },
                    url,
                ) {
                    NewWindowDecision::SameView(url) => {
                        let _ = native.send(Operation::SameView(id, url));
                    }
                    // Core already emitted `school-web-open` for this decision.
                    NewWindowDecision::OpenSchool { .. } => {}
                    NewWindowDecision::OpenExternal(url) => {
                        if let Ok(app) = native.app() {
                            let _ = app.opener().open_url(url.as_str(), None::<&str>);
                        }
                    }
                    NewWindowDecision::Ignore => {}
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
        let view = create_webview(
            &app,
            id.site,
            url.as_str(),
            WebBounds::default(),
            false,
            callbacks,
        )
        .map_err(BrowserError::Dispatch)?;
        #[cfg(target_os = "macos")]
        self.downloads.install(&app, id.site, &view);
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
        for (site, view) in views {
            if visible == Some(site) {
                apply_bounds(&view, bounds)?;
                view.show().map_err(|e| e.to_string())?;
                let _ = view.set_focus();
            } else {
                view.hide().map_err(|e| e.to_string())?;
            }
        }
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
        Ok(p.views
            .get(&id.site)
            .filter(|v| v.id == id)
            .and_then(|v| v.url.lock().ok().and_then(|url| url.clone())))
    }
    fn dispose(&self, id: BrowserId) -> Result<(), BrowserError> {
        self.send(Operation::Dispose(id))
    }
    fn shutdown(&self) -> BrowserFuture<'_, ()> {
        Box::pin(async move {
            self.tasks.close();
            self.tasks.wait().await;
            let views = self
                .presentation
                .lock()
                .map_err(|_| BrowserError::Stopped)?
                .views
                .drain()
                .map(|(_, view)| view.native)
                .collect::<Vec<_>>();
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
    pub fn is_visible(&self) -> bool {
        self.native
            .presentation
            .lock()
            .is_ok_and(|p| p.visible.is_some())
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
        target: Option<String>,
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
        let result = self.engine.open(site, target).await;
        if result.is_err() {
            let _ = self.native.visible(None, None);
            let _ = self.engine.user_browsing(None);
        }
        result.map_err(|e| e.to_string())
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
        if let Some(view) = view {
            view.set_zoom(scale.clamp(0.5, 2.))
                .map_err(|e| e.to_string())?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
