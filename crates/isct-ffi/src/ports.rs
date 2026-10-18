//! Native callbacks implement OS mechanisms, never domain state machines.
use crate::{
    json::{project, project_input},
    types::*,
    JsonValue, SdkError,
};
use isct_core::{ports as core, school_browser as browser};
use std::sync::{Arc, Mutex, Weak};
use url::Url;

#[uniffi::export(rust, foreign)]
pub trait NativeVault: Send + Sync {
    fn load(&self, account: String) -> Result<Option<String>, SdkError>;
    fn store(&self, account: String, secret: String) -> Result<(), SdkError>;
    fn delete(&self, account: String) -> Result<(), SdkError>;
}
fn vault_error(error: SdkError) -> core::SecretError {
    match error {
        SdkError::Callback { detail } => core::SecretError::Callback(detail),
        other => core::SecretError::Access(other.to_string()),
    }
}
pub(crate) struct Vault(pub Arc<dyn NativeVault>);
impl core::SecretVault for Vault {
    fn load(&self, account: &str) -> Result<Option<String>, core::SecretError> {
        self.0.load(account.into()).map_err(vault_error)
    }
    fn store(&self, account: &str, secret: &str) -> Result<(), core::SecretError> {
        self.0
            .store(account.into(), secret.into())
            .map_err(vault_error)
    }
    fn delete(&self, account: &str) -> Result<(), core::SecretError> {
        self.0.delete(account.into()).map_err(vault_error)
    }
}

#[derive(Clone, Debug, uniffi::Enum)]
pub enum BrowserAction {
    Navigate { url: String },
    Reload,
    Evaluate { script: String },
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct BrowserCommand {
    pub browser: BrowserId,
    pub action_id: u64,
    pub action: BrowserAction,
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum NavigationDecision {
    Allow,
    Cancel,
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum NewWindowDecision {
    SameView { url: String },
    OpenSchool { site: SchoolSite, url: String },
    OpenExternal { url: String },
    Ignore,
}

/// Callbacks enqueue native UI work and return promptly. BrowserEvents correlates
/// the completion. A callback must never synchronously wait for the UI thread.
#[uniffi::export(rust, foreign)]
pub trait NativeBrowser: Send + Sync {
    fn attach(&self, events: Arc<BrowserEvents>) -> Result<(), SdkError>;
    fn ensure(&self, browser: BrowserId, initial_url: String) -> Result<(), SdkError>;
    fn dispatch(&self, command: BrowserCommand) -> Result<(), SdkError>;
    fn current_url(&self, browser: BrowserId) -> Result<Option<String>, SdkError>;
    fn dispose(&self, browser: BrowserId) -> Result<(), SdkError>;
    fn shutdown(&self) -> Result<(), SdkError>;
}
#[derive(uniffi::Object)]
pub struct BrowserEvents {
    listener: Mutex<Option<Weak<dyn browser::SchoolBrowserListener>>>,
}
impl BrowserEvents {
    fn listener(&self) -> Result<Arc<dyn browser::SchoolBrowserListener>, SdkError> {
        self.listener
            .lock()
            .map_err(|_| SdkError::Closed)?
            .as_ref()
            .and_then(Weak::upgrade)
            .ok_or(SdkError::Closed)
    }
}
#[uniffi::export]
impl BrowserEvents {
    pub fn page_loaded(
        &self,
        page: PageId,
        cause: Option<u64>,
        url: String,
    ) -> Result<(), SdkError> {
        self.listener()?.page_loaded(browser::PageLoaded {
            page: project_input(page)?,
            cause: cause.map(browser::ActionId),
            url: parse_url(&url)?,
        });
        Ok(())
    }
    pub fn load_failed(
        &self,
        page: PageId,
        cause: Option<u64>,
        url: Option<String>,
        detail: String,
    ) -> Result<(), SdkError> {
        self.listener()?.load_failed(browser::LoadFailed {
            page: project_input(page)?,
            cause: cause.map(browser::ActionId),
            url: url.map(|v| parse_url(&v)).transpose()?,
            message: detail,
        });
        Ok(())
    }
    pub fn title_changed(&self, page: PageId, title: String) -> Result<(), SdkError> {
        self.listener()?.title_changed(project_input(page)?, title);
        Ok(())
    }
    pub fn navigation(
        &self,
        page: PageId,
        url: String,
        main_frame: bool,
    ) -> Result<NavigationDecision, SdkError> {
        Ok(
            match self.listener()?.decide_navigation(
                project_input(page)?,
                parse_url(&url)?,
                main_frame,
            ) {
                browser::NavigationDecision::Allow => NavigationDecision::Allow,
                browser::NavigationDecision::Cancel => NavigationDecision::Cancel,
            },
        )
    }
    pub fn new_window(&self, page: PageId, url: String) -> Result<NewWindowDecision, SdkError> {
        Ok(
            match self
                .listener()?
                .new_window(project_input(page)?, parse_url(&url)?)
            {
                browser::NewWindowDecision::SameView(url) => {
                    NewWindowDecision::SameView { url: url.into() }
                }
                browser::NewWindowDecision::OpenSchool { site, url } => {
                    NewWindowDecision::OpenSchool {
                        site: project(site)?,
                        url: url.into(),
                    }
                }
                browser::NewWindowDecision::OpenExternal(url) => {
                    NewWindowDecision::OpenExternal { url: url.into() }
                }
                browser::NewWindowDecision::Ignore => NewWindowDecision::Ignore,
            },
        )
    }
    pub fn closed(&self, browser: BrowserId) -> Result<(), SdkError> {
        self.listener()?.closed(project_input(browser)?);
        Ok(())
    }
}
pub(crate) struct Browser {
    native: Arc<dyn NativeBrowser>,
    events: Arc<BrowserEvents>,
    pub(crate) shutdown_error: Arc<Mutex<Option<SdkError>>>,
}
impl Browser {
    pub fn new(native: Arc<dyn NativeBrowser>) -> Self {
        Self {
            native,
            shutdown_error: Arc::new(Mutex::new(None)),
            events: Arc::new(BrowserEvents {
                listener: Mutex::new(None),
            }),
        }
    }
}
fn dispatch_error(e: SdkError) -> browser::BrowserError {
    browser::BrowserError::Dispatch(e.to_string())
}
impl browser::SchoolBrowser for Browser {
    fn attach(
        &self,
        listener: Weak<dyn browser::SchoolBrowserListener>,
    ) -> Result<(), browser::BrowserError> {
        *self
            .events
            .listener
            .lock()
            .map_err(|_| browser::BrowserError::Stopped)? = Some(listener);
        self.native
            .attach(self.events.clone())
            .map_err(dispatch_error)
    }
    fn ensure(&self, id: browser::BrowserId, initial: Url) -> Result<(), browser::BrowserError> {
        self.native
            .ensure(project(id).map_err(dispatch_error)?, initial.into())
            .map_err(dispatch_error)
    }
    fn dispatch(&self, command: browser::BrowserCommand) -> Result<(), browser::BrowserError> {
        let action = match command.command {
            browser::BrowserAction::Navigate(url) => BrowserAction::Navigate { url: url.into() },
            browser::BrowserAction::Reload => BrowserAction::Reload,
            browser::BrowserAction::Eval(script) => BrowserAction::Evaluate { script },
        };
        self.native
            .dispatch(BrowserCommand {
                browser: project(command.browser).map_err(dispatch_error)?,
                action_id: command.action.0,
                action,
            })
            .map_err(dispatch_error)
    }
    fn current_url(&self, id: browser::BrowserId) -> Result<Option<Url>, browser::BrowserError> {
        self.native
            .current_url(project(id).map_err(dispatch_error)?)
            .map_err(dispatch_error)?
            .map(|v| parse_url(&v).map_err(dispatch_error))
            .transpose()
    }
    fn dispose(&self, id: browser::BrowserId) -> Result<(), browser::BrowserError> {
        self.native
            .dispose(project(id).map_err(dispatch_error)?)
            .map_err(dispatch_error)
    }
    fn shutdown(&self) -> browser::BrowserFuture<'_, ()> {
        Box::pin(async move {
            *self
                .events
                .listener
                .lock()
                .map_err(|_| browser::BrowserError::Stopped)? = None;
            let result = self.native.shutdown();
            if let Err(error) = &result {
                *self
                    .shutdown_error
                    .lock()
                    .map_err(|_| browser::BrowserError::Stopped)? = Some(error.clone());
            }
            result.map_err(dispatch_error)
        })
    }
}
pub(crate) fn parse_url(text: &str) -> Result<Url, SdkError> {
    Url::parse(text).map_err(|e| SdkError::InvalidInput {
        detail: e.to_string(),
    })
}

#[uniffi::export(rust, foreign)]
#[async_trait::async_trait]
pub trait NativeNotifications: Send + Sync {
    async fn deliver(&self, item: NotificationItem) -> Result<(), SdkError>;
}
pub(crate) struct Notifications(pub Arc<dyn NativeNotifications>);
impl isct_core::schedule::NotificationSink for Notifications {
    fn deliver<'a>(
        &'a self,
        item: isct_core::schedule::NotificationItem,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<(), isct_core::schedule::NotificationSinkError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            self.0
                .deliver(project(item).map_err(|e| {
                    isct_core::schedule::NotificationSinkError::Delivery(e.to_string())
                })?)
                .await
                .map_err(|e| isct_core::schedule::NotificationSinkError::Delivery(e.to_string()))
        })
    }
}
#[uniffi::export(rust, foreign)]
#[async_trait::async_trait]
pub trait NativeDownloadSession: Send + Sync {
    async fn cookie_header(&self, url: String) -> Result<String, SdkError>;
}
pub(crate) struct DownloadSession(pub Arc<dyn NativeDownloadSession>);
fn download_session_error(error: SdkError) -> isct_core::downloads::FetchError {
    use isct_core::downloads::FetchError as E;
    match error {
        SdkError::Authentication { .. } => E::Login,
        SdkError::InvalidInput { .. } => E::Invalid,
        SdkError::Timeout { .. } => E::Timeout,
        SdkError::Cancelled | SdkError::Closed => E::Cancelled,
        // Cookie access failures must not ask the user to sign in again.
        _ => E::Network,
    }
}

#[cfg(test)]
#[path = "tests/ports__download_tests.rs"]
mod download_tests;

impl isct_core::downloads::DownloadSession for DownloadSession {
    fn cookie_header<'a>(
        &'a self,
        url: &'a Url,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<String, isct_core::downloads::FetchError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            self.0
                .cookie_header(url.to_string())
                .await
                .map_err(download_session_error)
        })
    }
}
#[uniffi::export(rust, foreign)]
#[async_trait::async_trait]
pub trait NativeCloud: Send + Sync {
    fn disconnect(&self, provider: CloudProvider) -> Result<(), SdkError>;
    async fn read_workspace(&self, provider: CloudProvider) -> Result<Option<JsonValue>, SdkError>;
    async fn merge_and_push(
        &self,
        provider: CloudProvider,
        document: JsonValue,
        device: String,
    ) -> Result<JsonValue, SdkError>;
    async fn versions(&self, provider: CloudProvider) -> Result<Vec<CloudVersion>, SdkError>;
    async fn version_document(
        &self,
        provider: CloudProvider,
        revision: u64,
    ) -> Result<JsonValue, SdkError>;
}
pub(crate) struct Cloud(pub Arc<dyn NativeCloud>);
fn cloud_error(e: SdkError) -> isct_core::cloud_sync::CloudSyncError {
    isct_core::cloud_sync::CloudSyncError::Transport(e.to_string())
}
impl isct_core::cloud_sync::CloudTransport for Cloud {
    fn disconnect(
        &self,
        p: isct_core::cloud_sync::CloudProvider,
    ) -> Result<(), isct_core::cloud_sync::CloudSyncError> {
        self.0
            .disconnect(project(p).map_err(cloud_error)?)
            .map_err(cloud_error)
    }
    fn read_workspace<'a>(
        &'a self,
        p: isct_core::cloud_sync::CloudProvider,
    ) -> isct_core::cloud_sync::CloudFuture<'a, Option<serde_json::Value>> {
        Box::pin(async move {
            Ok(self
                .0
                .read_workspace(project(p).map_err(cloud_error)?)
                .await
                .map_err(cloud_error)?
                .map(|v| v.0))
        })
    }
    fn merge_and_push<'a>(
        &'a self,
        p: isct_core::cloud_sync::CloudProvider,
        d: &'a serde_json::Value,
        device: &'a str,
    ) -> isct_core::cloud_sync::CloudFuture<'a, serde_json::Value> {
        Box::pin(async move {
            Ok(self
                .0
                .merge_and_push(
                    project(p).map_err(cloud_error)?,
                    JsonValue(d.clone()),
                    device.into(),
                )
                .await
                .map_err(cloud_error)?
                .0)
        })
    }
    fn versions<'a>(
        &'a self,
        p: isct_core::cloud_sync::CloudProvider,
    ) -> isct_core::cloud_sync::CloudFuture<'a, Vec<isct_core::cloud_sync::CloudVersion>> {
        Box::pin(async move {
            project_input(
                self.0
                    .versions(project(p).map_err(cloud_error)?)
                    .await
                    .map_err(cloud_error)?,
            )
            .map_err(cloud_error)
        })
    }
    fn version_document<'a>(
        &'a self,
        p: isct_core::cloud_sync::CloudProvider,
        r: u64,
    ) -> isct_core::cloud_sync::CloudFuture<'a, serde_json::Value> {
        Box::pin(async move {
            Ok(self
                .0
                .version_document(project(p).map_err(cloud_error)?, r)
                .await
                .map_err(cloud_error)?
                .0)
        })
    }
}

/// Map search text for a room's building, as on Today's class cards.
#[uniffi::export]
pub fn room_map_query(room: String) -> Option<String> {
    isct_core::schedule::room_map_query(&room)
}

/// Install these shared scripts at document start before navigating a school view.
#[uniffi::export]
pub fn school_browser_initialization() -> Result<Vec<String>, SdkError> {
    let scripts = vec![
        isct_core::school_browser::KYOMU_POPUP.to_owned(),
        isct_core::school_browser::SCHOOL_LINKS.to_owned(),
    ];
    #[cfg(target_os = "android")]
    {
        let json = serde_json::to_string(&scripts).map_err(|e| SdkError::Internal {
            detail: e.to_string(),
        })?;
        Ok(vec![
            isct_core::school_browser::ANDROID_SUPPORT.replace("__ISCT_SUPPORT_SCRIPTS__", &json)
        ])
    }
    #[cfg(not(target_os = "android"))]
    {
        Ok(scripts)
    }
}
