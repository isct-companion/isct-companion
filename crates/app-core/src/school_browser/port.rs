use crate::lifecycle::SchoolSite;
use serde::{Deserialize, Serialize};
use std::{future::Future, pin::Pin, sync::Weak};
use url::Url;

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
    Reload,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadFailed {
    pub page: PageId,
    pub cause: Option<ActionId>,
    pub url: Option<Url>,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationDecision {
    Allow,
    Cancel,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NewWindowDecision {
    SameView(Url),
    OpenSchool { site: SchoolSite, url: Url },
    OpenExternal(Url),
    Ignore,
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
    fn decide_navigation(&self, page: PageId, url: Url, main_frame: bool) -> NavigationDecision;
    fn new_window(&self, page: PageId, url: Url) -> NewWindowDecision;
    fn closed(&self, browser: BrowserId);
    /// The view carried out `action`'s stop. `page` is the last page it had
    /// reported then: any later report of that page or an earlier one is late.
    fn stopped(&self, page: PageId, action: ActionId);
}
