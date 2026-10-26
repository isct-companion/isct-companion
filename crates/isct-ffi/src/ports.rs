//! Native callbacks implement OS mechanisms, never domain state machines.
use crate::{
    json::{project, project_input},
    types::*,
    SdkError,
};
use isct_core::cloud_sync::document::{
    BoxFuture, DocumentCloud, DocumentError, ObjectStore, Providers, StoredObject, TaggedWorkspace,
};
use isct_core::cloud_sync::google_drive::{
    DriveAuthorization, GoogleDrive, OAuthClient, Pkce, SCOPE, VAULT_ACCOUNT,
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
    /// Only Apple devices synchronize Keychain items between devices.
    fn shared(&self, account: &str) -> bool {
        cfg!(target_os = "ios") && core::vault_item(account).is_ok_and(|item| item.synchronized)
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
/// The school WebView's cookie store: course files, LMS and 教務Web requests
/// read the sign-in from it and write server cookies back to it.
#[uniffi::export(rust, foreign)]
#[async_trait::async_trait]
pub trait NativeDownloadSession: Send + Sync {
    /// The Cookie header the school store sends to `url`; empty when none.
    async fn cookie_header(&self, url: String) -> Result<String, SdkError>;
    /// Stores Set-Cookie values from a Core response to `url` (already vetted
    /// by Core: host-only or for the response's own site).
    async fn store_set_cookies(&self, url: String, set_cookie: Vec<String>)
        -> Result<(), SdkError>;
}
pub(crate) struct NativeCookies(pub Arc<dyn NativeDownloadSession>);
fn cookie_error(error: SdkError) -> isct_core::net::NetError {
    use isct_core::net::NetError as E;
    match error {
        SdkError::Authentication { .. } => E::SignInRequired,
        SdkError::InvalidInput { detail } => E::Invalid(detail),
        SdkError::Timeout { .. } => E::Deadline,
        SdkError::Cancelled | SdkError::Closed => E::Cancelled,
        // Cookie access failures must not ask the user to sign in again.
        other => E::Connect(format!("cookie store: {other}")),
    }
}

#[cfg(test)]
#[path = "tests/ports__download_tests.rs"]
mod download_tests;

impl isct_core::net::SessionCookies for NativeCookies {
    fn cookie_header<'a>(
        &'a self,
        url: &'a Url,
    ) -> isct_core::net::BoxFuture<'a, Result<String, isct_core::net::NetError>> {
        Box::pin(async move {
            let header = self
                .0
                .cookie_header(url.to_string())
                .await
                .map_err(cookie_error)?;
            if header.trim().is_empty() {
                Err(isct_core::net::NetError::SignInRequired)
            } else {
                Ok(header)
            }
        })
    }

    fn store_set_cookies<'a>(
        &'a self,
        url: &'a Url,
        set_cookie: &'a [String],
    ) -> isct_core::net::BoxFuture<'a, Result<(), isct_core::net::NetError>> {
        Box::pin(async move {
            let storable = isct_core::net::cookies::storable(url, set_cookie);
            if storable.is_empty() {
                return Ok(());
            }
            self.0
                .store_set_cookies(url.to_string(), storable)
                .await
                .map_err(cookie_error)
        })
    }
}
/// One stored workspace object and the provider's opaque change tag for it.
#[derive(Clone, Debug, uniffi::Record)]
pub struct CloudObject {
    pub bytes: Vec<u8>,
    pub tag: String,
}
/// Moves the cloud workspace object's bytes. Core parses the envelope,
/// merges, keeps history and decides which revision a write may replace.
#[uniffi::export(rust, foreign)]
#[async_trait::async_trait]
pub trait NativeCloud: Send + Sync {
    /// The name cloud version lists show for this device, if the OS gives one.
    fn device_name(&self) -> Option<String>;
    fn disconnect(&self, provider: CloudProvider) -> Result<(), SdkError>;
    async fn read(&self, provider: CloudProvider) -> Result<Option<CloudObject>, SdkError>;
    /// `expected` is the tag of the object last read, or none when nothing
    /// existed. Fail with `SdkError::Conflict` when the stored object differs.
    async fn write(
        &self,
        provider: CloudProvider,
        bytes: Vec<u8>,
        expected: Option<String>,
    ) -> Result<(), SdkError>;
}
pub(crate) struct Cloud(Arc<dyn NativeCloud>);
fn cloud_error(e: SdkError) -> DocumentError {
    match e {
        SdkError::Conflict { .. } => DocumentError::Conflict,
        other => DocumentError::Transport(other.to_string()),
    }
}
impl ObjectStore for Cloud {
    fn read<'a>(
        &'a self,
        p: isct_core::cloud_sync::CloudProvider,
    ) -> BoxFuture<'a, Option<StoredObject>> {
        Box::pin(async move {
            Ok(self
                .0
                .read(project(p).map_err(cloud_error)?)
                .await
                .map_err(cloud_error)?
                .map(|object| StoredObject {
                    bytes: object.bytes,
                    tag: object.tag,
                }))
        })
    }
    fn write<'a>(
        &'a self,
        p: isct_core::cloud_sync::CloudProvider,
        bytes: Vec<u8>,
        expected: Option<String>,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.0
                .write(project(p).map_err(cloud_error)?, bytes, expected)
                .await
                .map_err(cloud_error)
        })
    }
    fn disconnect(&self, p: isct_core::cloud_sync::CloudProvider) -> Result<(), DocumentError> {
        self.0
            .disconnect(project(p).map_err(cloud_error)?)
            .map_err(cloud_error)
    }
}
/// The Google OAuth client an Apple shell signs in with: its registered iOS
/// client and the redirect that client accepts.
#[derive(Clone, Debug, uniffi::Record)]
pub struct GoogleClient {
    pub client_id: String,
    pub redirect_uri: String,
}
/// Presents Google sign-in for Drive in a web authentication session (iOS).
/// Core starts and checks the PKCE authorization and keeps the token in the
/// vault; the shell only shows the page.
#[uniffi::export(rust, foreign)]
#[async_trait::async_trait]
pub trait NativeWebAuthentication: Send + Sync {
    fn google_client(&self) -> GoogleClient;
    /// Opens `url` and returns the URL the session ended at, whose scheme is
    /// `callback_scheme`. Fail with `SdkError::Cancelled` if the user closed it.
    async fn authenticate(&self, url: String, callback_scheme: String) -> Result<String, SdkError>;
}
/// Hands out Drive access tokens from the OS account system (Android's
/// Google Identity authorization), which keeps the grant itself.
#[uniffi::export(rust, foreign)]
#[async_trait::async_trait]
pub trait NativeDriveTokens: Send + Sync {
    /// A current token for `scope`. With `interactive` false, fail instead of
    /// showing any UI.
    async fn access_token(&self, scope: String, interactive: bool) -> Result<String, SdkError>;
    /// Forgets this app's Drive grant on this device.
    fn sign_out(&self) -> Result<(), SdkError>;
}
/// How this shell authorizes Drive; Core owns the REST calls either way.
pub(crate) enum DriveAuth {
    Pkce {
        pkce: Pkce<Vault>,
        web: Arc<dyn NativeWebAuthentication>,
    },
    Tokens {
        tokens: Arc<dyn NativeDriveTokens>,
        /// Records that the user connected, so a restart knows Drive is
        /// authorized without asking the OS (which may show UI).
        vault: Vault,
    },
}
const DRIVE_CONNECTED: &str = "connected";
fn drive_error(e: SdkError) -> DocumentError {
    DocumentError::Transport(e.to_string())
}
impl DriveAuth {
    pub(crate) fn new(
        vault: Arc<dyn NativeVault>,
        web: Option<Arc<dyn NativeWebAuthentication>>,
        tokens: Option<Arc<dyn NativeDriveTokens>>,
    ) -> Result<Option<Self>, SdkError> {
        Ok(match (web, tokens) {
            (Some(web), _) => {
                let client = web.google_client();
                Some(Self::Pkce {
                    pkce: Pkce::new(
                        Vault(vault),
                        OAuthClient {
                            id: client.client_id,
                            secret: None,
                        },
                    )
                    .map_err(|e| SdkError::Internal {
                        detail: e.to_string(),
                    })?,
                    web,
                })
            }
            (None, Some(tokens)) => Some(Self::Tokens {
                tokens,
                vault: Vault(vault),
            }),
            (None, None) => None,
        })
    }
    pub(crate) fn authorized(&self) -> Result<bool, SdkError> {
        let saved = match self {
            Self::Pkce { pkce, .. } => pkce.authorized(),
            Self::Tokens { vault, .. } => core::SecretVault::load(vault, VAULT_ACCOUNT)
                .map(|saved| saved.as_deref() == Some(DRIVE_CONNECTED))
                .map_err(|e| DocumentError::Transport(e.to_string())),
        };
        saved.map_err(|e| SdkError::Storage {
            detail: e.to_string(),
        })
    }
    /// The interactive sign-in behind turning Drive on.
    pub(crate) async fn connect(&self) -> Result<(), SdkError> {
        match self {
            Self::Pkce { pkce, web } => {
                let client = web.google_client();
                let pending = pkce
                    .begin(&client.client_id, &client.redirect_uri)
                    .map_err(|e| SdkError::InvalidInput {
                        detail: e.to_string(),
                    })?;
                let scheme = Url::parse(&client.redirect_uri)
                    .map_err(|e| SdkError::InvalidInput {
                        detail: e.to_string(),
                    })?
                    .scheme()
                    .to_owned();
                let callback = web.authenticate(pending.url.clone(), scheme).await?;
                pkce.finish(&client.client_id, &pending, &callback)
                    .await
                    .map_err(|e| SdkError::Authentication {
                        detail: e.to_string(),
                    })
            }
            Self::Tokens { tokens, vault } => {
                tokens.access_token(SCOPE.into(), true).await?;
                core::SecretVault::store(vault, VAULT_ACCOUNT, DRIVE_CONNECTED).map_err(|e| {
                    SdkError::Storage {
                        detail: e.to_string(),
                    }
                })
            }
        }
    }
}
impl DriveAuthorization for DriveAuth {
    fn access_token<'a>(&'a self, refresh: bool) -> BoxFuture<'a, String> {
        match self {
            Self::Pkce { pkce, .. } => pkce.access_token(refresh),
            // The OS refreshes its own tokens; a background request never shows UI.
            Self::Tokens { tokens, .. } => Box::pin(async move {
                tokens
                    .access_token(SCOPE.into(), false)
                    .await
                    .map_err(drive_error)
            }),
        }
    }
    fn disconnect(&self) -> Result<(), DocumentError> {
        match self {
            Self::Pkce { pkce, .. } => pkce.disconnect(),
            Self::Tokens { tokens, vault } => {
                core::SecretVault::delete(vault, VAULT_ACCOUNT)
                    .map_err(|e| DocumentError::Transport(e.to_string()))?;
                tokens.sign_out().map_err(drive_error)
            }
        }
    }
}
/// Lets the session keep the `DriveAuth` it signs in with while Drive's
/// store, inside the transport, uses the same one.
pub(crate) struct SharedDrive(pub Arc<DriveAuth>);
impl DriveAuthorization for SharedDrive {
    fn access_token<'a>(&'a self, refresh: bool) -> BoxFuture<'a, String> {
        self.0.access_token(refresh)
    }
    fn disconnect(&self) -> Result<(), DocumentError> {
        self.0.disconnect()
    }
}
/// The queue's transport over the stores this shell has: CloudKit through
/// `NativeCloud`, Drive through Core's REST client.
pub(crate) fn cloud_stores(
    native: Option<Arc<dyn NativeCloud>>,
    drive: Option<Arc<DriveAuth>>,
) -> Result<DocumentCloud<Providers>, SdkError> {
    let name = native.as_ref().and_then(|native| native.device_name());
    let mut stores = Providers::default();
    if let Some(native) = native {
        stores = stores.with(
            isct_core::cloud_sync::CloudProvider::Icloud,
            TaggedWorkspace::new(Cloud(native)),
        );
    }
    if let Some(drive) = drive {
        stores = stores.with(
            isct_core::cloud_sync::CloudProvider::GoogleDrive,
            GoogleDrive::new(SharedDrive(drive)).map_err(|e| SdkError::Internal {
                detail: e.to_string(),
            })?,
        );
    }
    Ok(DocumentCloud::new(stores, name))
}
/// Stands in when the shell has no cloud store: the journal still owns
/// imports and recovery points, and no provider can be enabled.
pub(crate) struct NoCloud;
impl isct_core::cloud_sync::CloudTransport for NoCloud {
    fn disconnect(
        &self,
        _: isct_core::cloud_sync::CloudProvider,
    ) -> Result<(), isct_core::cloud_sync::CloudSyncError> {
        Ok(())
    }
    fn read_workspace<'a>(
        &'a self,
        _: isct_core::cloud_sync::CloudProvider,
    ) -> isct_core::cloud_sync::CloudFuture<'a, Option<serde_json::Value>> {
        Box::pin(async { Err(no_cloud()) })
    }
    fn merge_and_push<'a>(
        &'a self,
        _: isct_core::cloud_sync::CloudProvider,
        _: &'a serde_json::Value,
        _: &'a str,
    ) -> isct_core::cloud_sync::CloudFuture<'a, serde_json::Value> {
        Box::pin(async { Err(no_cloud()) })
    }
    fn versions<'a>(
        &'a self,
        _: isct_core::cloud_sync::CloudProvider,
    ) -> isct_core::cloud_sync::CloudFuture<'a, Vec<isct_core::cloud_sync::CloudVersion>> {
        Box::pin(async { Err(no_cloud()) })
    }
    fn version_document<'a>(
        &'a self,
        _: isct_core::cloud_sync::CloudProvider,
        _: u64,
    ) -> isct_core::cloud_sync::CloudFuture<'a, serde_json::Value> {
        Box::pin(async { Err(no_cloud()) })
    }
}
fn no_cloud() -> isct_core::cloud_sync::CloudSyncError {
    isct_core::cloud_sync::CloudSyncError::Transport("No cloud store is installed.".into())
}

/// Keychain location of a `NativeVault` account, shared with the desktop app.
#[derive(Clone, Debug, uniffi::Record)]
pub struct VaultItem {
    pub service: String,
    pub key: String,
    /// Store as an iCloud Keychain / backed-up item rather than device-only.
    pub synchronized: bool,
}

/// The CloudKit record a `NativeCloud` on Apple devices reads and writes,
/// shared with the Mac app.
#[derive(Clone, Debug, uniffi::Record)]
pub struct CloudKitRecord {
    pub container: String,
    pub record_type: String,
    pub record_name: String,
    pub asset_key: String,
}

#[uniffi::export]
pub fn cloudkit_record() -> CloudKitRecord {
    use isct_core::cloud_sync::document as d;
    CloudKitRecord {
        container: d::CLOUDKIT_CONTAINER.into(),
        record_type: d::CLOUDKIT_RECORD_TYPE.into(),
        record_name: d::CLOUD_FILE_NAME.into(),
        asset_key: d::CLOUDKIT_ASSET_KEY.into(),
    }
}

/// The page a school site opens at, for "back to this site's start".
#[uniffi::export]
pub fn school_start_url(site: crate::types::SchoolSite) -> Result<String, SdkError> {
    let site: isct_core::lifecycle::SchoolSite = project_input(site)?;
    Ok(site.start_url().to_string())
}

/// Whether a cookie or website-data domain is a school one, for the shell's
/// `NativeWebSignOut`.
#[uniffi::export]
pub fn is_school_cookie_domain(domain: String) -> bool {
    isct_core::school_browser::is_school_cookie_domain(&domain)
}

#[uniffi::export]
pub fn vault_item(account: String) -> Result<VaultItem, SdkError> {
    let item = core::vault_item(&account).map_err(|e| SdkError::InvalidInput {
        detail: e.to_string(),
    })?;
    Ok(VaultItem {
        service: item.service.into(),
        key: item.key,
        synchronized: item.synchronized,
    })
}

/// One authenticator account read from a setup link or a transfer QR code.
#[derive(Clone, Debug, uniffi::Record)]
pub struct OtpImportAccount {
    pub label: String,
    pub issuer: String,
    /// Sensitive: pass only the chosen account's value to `connectSchool`.
    pub setup: String,
}

#[derive(Clone, Debug, thiserror::Error, uniffi::Error)]
pub enum OtpImportError {
    #[error("This is not a supported authenticator QR code.")]
    Invalid,
    /// A multi-page Google Authenticator export; export the school account alone.
    #[error("Export only your school account, then scan its QR code again.")]
    MultiplePages,
    #[error("This account uses an unsupported verification code format.")]
    Unsupported,
}

/// Reads a decoded QR payload or pasted text: an `otpauth://totp` link or a
/// Google Authenticator transfer (`otpauth-migration://`). QR pixels and the
/// camera stay in the shell.
#[uniffi::export]
pub fn import_otp(payload: String) -> Result<Vec<OtpImportAccount>, OtpImportError> {
    use isct_core::school_auth::OtpImportError as E;
    isct_core::school_auth::import_otp(&payload)
        .map(|accounts| {
            accounts
                .into_iter()
                .map(|a| OtpImportAccount {
                    label: a.label,
                    issuer: a.issuer,
                    setup: a.setup,
                })
                .collect()
        })
        .map_err(|e| match e {
            E::Invalid => OtpImportError::Invalid,
            E::MultiplePages => OtpImportError::MultiplePages,
            E::Unsupported => OtpImportError::Unsupported,
        })
}

#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum MatrixConfidence {
    High,
    Medium,
    Low,
}

/// How a scanned Matrix cell is shown for review, the same on every shell.
#[uniffi::export]
pub fn matrix_confidence(text: String, confidence: f64) -> MatrixConfidence {
    use isct_core::matrix_scan::MatrixConfidence as C;
    match isct_core::matrix_scan::matrix_confidence(&text, confidence) {
        C::High => MatrixConfidence::High,
        C::Medium => MatrixConfidence::Medium,
        C::Low => MatrixConfidence::Low,
    }
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
