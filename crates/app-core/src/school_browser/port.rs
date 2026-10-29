use super::DenyReason;
use crate::lifecycle::SchoolSite;
use serde::{Deserialize, Serialize};
use std::{future::Future, pin::Pin, sync::Weak};
use url::Url;

/// A view generation. A popup carries its opener's site and a generation of
/// its own.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserId {
    pub site: SchoolSite,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ActionId(pub u64);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageId {
    pub browser: BrowserId,
    pub sequence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BrowserAction {
    Navigate(Url),
    /// The engine's own reload. Only sent to hosts that report form
    /// resubmission (`HostCapabilities::form_resubmission`), so a reload
    /// that would resend a form reaches the rule table.
    Reload,
    /// The engine's own history; a resubmission on the way is a
    /// `NavigationKind::FormResubmit { from_history: true }`.
    GoBack,
    GoForward,
    Eval(String),
    /// Ends an abandoned attempt: the view stops loading, runs
    /// [`SCHOOL_STOP`](crate::school_browser::SCHOOL_STOP), and then answers
    /// with [`SchoolBrowserListener::stopped`].
    Stop,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrowserCommand {
    pub browser: BrowserId,
    pub action: ActionId,
    pub command: BrowserAction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageLoaded {
    pub page: PageId,
    pub cause: Option<ActionId>,
    pub url: Url,
}

/// Whether a view's or popup's own history can step back or forward from
/// the page it shows. A host that never reports it leaves both unknown, and
/// Core treats unknown as possible.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct History {
    pub back: bool,
    pub forward: bool,
}

/// Why a main-frame load failed, coarse enough for every engine to map its
/// own error codes onto.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LoadFailure {
    /// Superseded or cancelled by policy; never shown.
    Cancelled,
    /// The device has no network.
    Offline,
    /// The name does not resolve (often a campus-only host off VPN).
    HostNotFound,
    /// No answer in time, or the connection was refused.
    Unreachable,
    /// The secure connection failed: certificate, TLS, or App Transport
    /// Security refusing plain HTTP.
    Insecure,
    /// Plain HTTP the rule table kept out of the view.
    Cleartext,
    Other,
}

/// A main-frame load in a view that did not finish. Hosts report every one
/// but cancellations of their own; Core decides what is shown.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadFailed {
    pub page: PageId,
    pub cause: Option<ActionId>,
    pub url: Option<Url>,
    pub reason: LoadFailure,
    /// The engine's own description, for logs only.
    pub message: String,
}

/// How the web engine came to start a navigation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationKind {
    Link,
    FormSubmit,
    BackForward,
    Reload,
    /// The engine is about to send an earlier form again: reloading a POST
    /// result (`from_history: false`) or returning to one through history.
    FormResubmit {
        from_history: bool,
    },
    Other,
    /// The host's engine does not say.
    Unknown,
}

/// Everything the host knows about a frame navigation before it starts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NavigationRequest {
    /// The view asking, with the last page it reported.
    pub page: PageId,
    pub url: Url,
    /// `GET`, `POST`, …; `None` when the engine does not say.
    pub method: Option<String>,
    pub main_frame: bool,
    pub kind: NavigationKind,
    pub user_gesture: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WindowFeatures {
    pub width: Option<f64>,
    pub height: Option<f64>,
}

/// A window a page asks for (`window.open`, `target=_blank`, a form posted to
/// a named window). The host has the real request and the opener; Core only
/// decides whether the window exists.
#[derive(Clone, Debug, PartialEq)]
pub struct NewWindowRequest {
    /// The opener view, with the last page it reported.
    pub opener: PageId,
    /// The first URL; `about:blank` for `window.open('')`.
    pub url: Url,
    pub method: Option<String>,
    pub kind: NavigationKind,
    pub user_gesture: bool,
    /// The target window name (`_blank` or a page-chosen name), when given.
    pub name: Option<String>,
    pub features: WindowFeatures,
}

/// What became of a navigation the engine turned into a download.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DownloadDisposition {
    /// A file the engine can display (a PDF sent as an attachment), fetched
    /// and then shown in the view it was meant for.
    Shown,
    /// A file saved to the device.
    Saved,
}

/// A view's or popup's navigation became a download.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BecameDownload {
    /// The view or popup whose navigation it was, with its last page.
    pub page: PageId,
    /// Whether it had committed a page before; a popup opened for the file
    /// has not.
    pub had_page: bool,
    pub disposition: DownloadDisposition,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NavigationDecision {
    Allow,
    /// Cancel; the system browser opens the URL.
    External(Url),
    /// Cancel.
    Deny(DenyReason),
    /// Cancel this back/forward resubmission, then step past the entry that
    /// would have resent its form, in the same direction.
    SkipEntry,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NewWindowDecision {
    /// Create the window from the engine's own request and configuration
    /// (so the POST body and `window.opener` survive) and report everything
    /// that happens in it under this id. It is a child of the opener.
    Popup(BrowserId),
    /// Refuse the window; the system browser opens the URL.
    External(Url),
    Deny(DenyReason),
}

/// What a host can report, so Core can stand in for what it cannot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostCapabilities {
    /// The host reports `NavigationKind::FormResubmit`. Without it Core
    /// reloads with a GET of the current URL instead of the engine's reload.
    pub form_resubmission: bool,
    /// Popups appear inside the web pane over their opener, so the app's
    /// toolbar acts on the frontmost popup. Without it they are separate
    /// windows with their own controls.
    pub inline_popups: bool,
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum BrowserError {
    #[error("{0}")]
    Unavailable(String),
    #[error("{0}")]
    Dispatch(String),
    #[error("The school browser has stopped.")]
    Stopped,
}

pub type BrowserFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, BrowserError>> + Send + 'a>>;

/// Native views dispatch commands on their required UI thread. Successful
/// return means accepted for dispatch; completion arrives through the listener.
pub trait SchoolBrowser: Send + Sync {
    fn capabilities(&self) -> HostCapabilities;
    fn attach(&self, listener: Weak<dyn SchoolBrowserListener>) -> Result<(), BrowserError>;
    fn ensure(&self, browser: BrowserId, initial: Url) -> Result<(), BrowserError>;
    fn dispatch(&self, command: BrowserCommand) -> Result<(), BrowserError>;
    fn current_url(&self, browser: BrowserId) -> Result<Option<Url>, BrowserError>;
    fn dispose(&self, browser: BrowserId) -> Result<(), BrowserError>;
    fn shutdown(&self) -> BrowserFuture<'_, ()>;
}

pub trait SchoolBrowserListener: Send + Sync {
    fn page_loaded(&self, event: PageLoaded);
    fn load_failed(&self, event: LoadFailed);
    fn title_changed(&self, page: PageId, title: String);
    /// The engine's history of a view or popup may have changed: a page
    /// committed or finished, or a popup was created (none yet).
    fn history_changed(&self, browser: BrowserId, history: History);
    fn decide_navigation(&self, request: NavigationRequest) -> NavigationDecision;
    fn new_window(&self, request: NewWindowRequest) -> NewWindowDecision;
    /// A navigation became a download. Core closes a popup that is left
    /// with nothing to show (`dispose`).
    fn became_download(&self, event: BecameDownload);
    /// A view or popup is gone: closed by its page (`window.close`), by the
    /// person, by `dispose`, or never created.
    fn closed(&self, browser: BrowserId);
    /// The view carried out `action`'s stop. `page` is the last page it had
    /// reported then: any later report of that page or an earlier one is late.
    fn stopped(&self, page: PageId, action: ActionId);
}
