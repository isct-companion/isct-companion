//! Google Drive for every shell: the `drive.appdata` REST calls and the
//! PKCE authorization that desktop and iOS share. A shell supplies only an
//! access token through [`DriveAuthorization`] (and, for PKCE, the browser).

use super::document::{
    self, BoxFuture, CloudWorkspaceFile, DocumentError, WorkspaceTransport, CLOUD_FILE_NAME,
    MAX_DOCUMENT_BYTES,
};
use super::CloudProvider;
use crate::net::read_response_limited;
use crate::oauth::code_exchange::{self, CodeExchange, OAuthClient};
#[cfg(any(target_os = "ios", target_os = "macos", target_os = "windows"))]
use crate::oauth::{code_exchange::Options, OAuthError};
use crate::ports::SecretVault;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::Url;

/// The vault account every shell keeps the Drive token under.
pub const VAULT_ACCOUNT: &str = "google-drive";
/// The non-sensitive scope for the app's hidden folder; nothing else is asked.
pub const SCOPE: &str = "https://www.googleapis.com/auth/drive.appdata";

/// Supplies a current access token for Drive requests. `refresh` asks for a
/// new one after Drive rejected the last (401).
pub trait DriveAuthorization: Send + Sync {
    fn access_token<'a>(&'a self, refresh: bool) -> BoxFuture<'a, String>;
    /// Forgets this device's authorization. Nothing is revoked at Google:
    /// revocation ends the grant of every client in the project, so turning
    /// Drive off here would sign the person's other devices out as well.
    fn disconnect(&self) -> Result<(), DocumentError>;
}

fn transport(error: impl ToString) -> DocumentError {
    DocumentError::Transport(error.to_string())
}

#[derive(Debug, Deserialize)]
struct DriveFile {
    id: String,
}

#[derive(Debug, Deserialize)]
struct DriveFiles {
    files: Option<Vec<DriveFile>>,
}

/// The workspace envelope and the encrypted credential file in Drive's
/// application-data folder.
pub struct GoogleDrive<A> {
    client: reqwest::Client,
    auth: A,
    /// Duplicate workspace files the last read merged in. Once a write that
    /// carries their union lands, they are deleted, so the folder keeps one.
    merged_duplicates: std::sync::Mutex<Vec<String>>,
}

impl<A: DriveAuthorization> GoogleDrive<A> {
    pub fn new(auth: A) -> Result<Self, DocumentError> {
        let client = crate::net::client_builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(45))
            .build()
            .map_err(|_| transport("Cloud HTTP client initialization failed."))?;
        Ok(Self {
            client,
            auth,
            merged_duplicates: std::sync::Mutex::default(),
        })
    }

    pub fn authorization(&self) -> &A {
        &self.auth
    }

    async fn send(
        &self,
        method: reqwest::Method,
        url: Url,
        content_type: Option<&str>,
        body: Option<Vec<u8>>,
        etag: Option<&str>,
    ) -> Result<reqwest::Response, DocumentError> {
        if body
            .as_ref()
            .is_some_and(|body| body.len() > MAX_BODY_BYTES)
        {
            return Err(transport("Cloud request body is too large."));
        }
        // One retry with a fresh token: a native token source (Android) can
        // hand out a token that Drive has already revoked.
        for refresh in [false, true] {
            let mut request = self
                .client
                .request(method.clone(), url.clone())
                .bearer_auth(self.auth.access_token(refresh).await?);
            if let Some(etag) = etag {
                request = request.header(reqwest::header::IF_MATCH, etag);
            }
            if let Some(content_type) = content_type {
                request = request.header(reqwest::header::CONTENT_TYPE, content_type);
            }
            if let Some(body) = &body {
                request = request.body(body.clone());
            }
            let response = request
                .send()
                .await
                .map_err(|_| transport("Cloud request failed."))?;
            if response.status().is_redirection() {
                return Err(transport("Cloud redirects are not followed."));
            }
            if response.status() != reqwest::StatusCode::UNAUTHORIZED {
                return Ok(response);
            }
        }
        Err(transport(
            "Cloud authorization expired; reconnect is required.",
        ))
    }

    async fn file_ids(&self, name: &str) -> Result<Vec<String>, DocumentError> {
        let mut url = endpoint("https://www.googleapis.com/drive/v3/files")?;
        url.query_pairs_mut()
            .append_pair("spaces", "appDataFolder")
            .append_pair("q", &format!("name = '{name}' and trashed = false"))
            .append_pair("fields", "files(id)")
            .append_pair("pageSize", "100");
        let response = self
            .send(reqwest::Method::GET, url, None, None, None)
            .await?;
        if !response.status().is_success() {
            return Err(transport(format!(
                "Google Drive lookup failed ({}).",
                response.status().as_u16()
            )));
        }
        let files: DriveFiles = serde_json::from_slice(
            &read_response_limited(response, 1024 * 1024)
                .await
                .map_err(transport)?,
        )
        .map_err(|_| transport("Google Drive lookup response was invalid."))?;
        let mut ids = files
            .files
            .unwrap_or_default()
            .into_iter()
            .map(|file| file.id)
            .collect::<Vec<_>>();
        ids.sort();
        Ok(ids)
    }

    async fn download(&self, id: &str) -> Result<Vec<u8>, DocumentError> {
        let response = self
            .send(
                reqwest::Method::GET,
                endpoint(&format!(
                    "https://www.googleapis.com/drive/v3/files/{id}?alt=media"
                ))?,
                None,
                None,
                None,
            )
            .await?;
        if !response.status().is_success() {
            return Err(transport("Google Drive workspace read failed."));
        }
        read_response_limited(response, MAX_BODY_BYTES)
            .await
            .map_err(transport)
    }

    /// Reads a file by name; `None` when the folder has none.
    pub async fn read_file(&self, name: &str) -> Result<Option<Vec<u8>>, DocumentError> {
        match self.file_ids(name).await?.first() {
            Some(id) => self.download(id).await.map(Some),
            None => Ok(None),
        }
    }

    /// Replaces a file by name, creating it in the app folder if missing.
    pub async fn write_file(&self, name: &str, bytes: Vec<u8>) -> Result<(), DocumentError> {
        let response = if let Some(id) = self.file_ids(name).await?.first() {
            let mut url = endpoint(&format!(
                "https://www.googleapis.com/upload/drive/v3/files/{id}"
            ))?;
            url.query_pairs_mut().append_pair("uploadType", "media");
            self.send(
                reqwest::Method::PATCH,
                url,
                Some("application/json"),
                Some(bytes),
                None,
            )
            .await?
        } else {
            let boundary = format!(
                "isct-companion-{}",
                code_exchange::random_urlsafe::<18>().map_err(transport)?
            );
            let metadata = serde_json::json!({ "name": name, "parents": ["appDataFolder"] });
            let mut body = format!(
                "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n--{boundary}\r\nContent-Type: application/json\r\n\r\n"
            )
            .into_bytes();
            body.extend_from_slice(&bytes);
            body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
            let mut url = endpoint("https://www.googleapis.com/upload/drive/v3/files")?;
            url.query_pairs_mut().append_pair("uploadType", "multipart");
            self.send(
                reqwest::Method::POST,
                url,
                Some(&format!("multipart/related; boundary={boundary}")),
                Some(body),
                None,
            )
            .await?
        };
        if !response.status().is_success() {
            return Err(transport(format!(
                "Cloud file write failed ({}).",
                response.status().as_u16()
            )));
        }
        Ok(())
    }

    async fn read_workspace(&self) -> Result<Option<CloudWorkspaceFile>, DocumentError> {
        // Drive names are not unique. Two first-time offline devices can
        // create siblings; choose a stable canonical id and retain both
        // documents until their union has been committed there.
        let mut current: Option<CloudWorkspaceFile> = None;
        let mut duplicates = Vec::new();
        for id in self.file_ids(CLOUD_FILE_NAME).await? {
            let file = document::parse_cloud_workspace(&self.download(&id).await?)?;
            match current.as_mut() {
                Some(current) => {
                    current.siblings.push(file.document);
                    duplicates.push(id);
                }
                None => current = Some(file),
            }
        }
        if let Ok(mut merged) = self.merged_duplicates.lock() {
            *merged = duplicates;
        }
        Ok(current)
    }

    async fn write_workspace(
        &self,
        file: &CloudWorkspaceFile,
        expected: Option<u64>,
    ) -> Result<(), DocumentError> {
        let bytes = serde_json::to_vec(file).map_err(transport)?;
        if bytes.len() > MAX_BODY_BYTES {
            return Err(DocumentError::TooLarge);
        }
        let Some(id) = self.file_ids(CLOUD_FILE_NAME).await?.into_iter().next() else {
            return match expected {
                None => self.write_file(CLOUD_FILE_NAME, bytes).await,
                Some(_) => Err(DocumentError::Conflict),
            };
        };
        // v2 exposes the file ETag; the upload below is conditional on it, so
        // a write that landed after this read fails with 412 and is merged
        // again. Media downloads do not accept it: Drive answers If-Match on
        // `alt=media` with 412 conditionNotMet even when nothing changed, so
        // the content is read plainly.
        let meta = self
            .send(
                reqwest::Method::GET,
                endpoint(&format!(
                    "https://www.googleapis.com/drive/v2/files/{id}?fields=etag"
                ))?,
                None,
                None,
                None,
            )
            .await?;
        if !meta.status().is_success() {
            return Err(transport("Google Drive metadata read failed."));
        }
        let meta: serde_json::Value = serde_json::from_slice(
            &read_response_limited(meta, 65536)
                .await
                .map_err(transport)?,
        )
        .map_err(transport)?;
        let etag = meta["etag"]
            .as_str()
            .ok_or_else(|| transport("Google Drive did not return a version tag."))?;
        let current = document::parse_cloud_workspace(&self.download(&id).await?)?;
        if Some(current.revision) != expected
            || file.previous.first().is_some_and(|previous| {
                !document::workspace_contents_equal(&current.document, &previous.document)
            })
        {
            log::warn!(
                "Google Drive changed before write: expected {:?}, observed {}",
                expected,
                current.revision
            );
            return Err(DocumentError::Conflict);
        }
        let response = self
            .send(
                reqwest::Method::PUT,
                endpoint(&format!(
                    "https://www.googleapis.com/upload/drive/v2/files/{id}?uploadType=media"
                ))?,
                Some("application/json"),
                Some(bytes),
                Some(etag),
            )
            .await?;
        if response.status() == reqwest::StatusCode::PRECONDITION_FAILED {
            log::warn!("Google Drive rejected conditional write");
            return Err(DocumentError::Conflict);
        }
        if !response.status().is_success() {
            return Err(transport(format!(
                "Cloud file write failed ({}).",
                response.status()
            )));
        }
        self.delete_merged_duplicates(&id).await;
        Ok(())
    }

    /// A duplicate is only ever created, never written again: every writer
    /// targets the first file id. So one this write merged holds nothing the
    /// written file lacks. A failed delete leaves it for the next merge.
    async fn delete_merged_duplicates(&self, written: &str) {
        let duplicates = self
            .merged_duplicates
            .lock()
            .map(|mut merged| std::mem::take(&mut *merged))
            .unwrap_or_default();
        for id in duplicates.iter().filter(|id| id.as_str() != written) {
            let deleted = match endpoint(&format!("https://www.googleapis.com/drive/v3/files/{id}"))
            {
                Ok(url) => self
                    .send(reqwest::Method::DELETE, url, None, None, None)
                    .await
                    .is_ok_and(|response| response.status().is_success()),
                Err(_) => false,
            };
            if !deleted {
                crate::diagnostics::info(
                    "cloud-sync",
                    "A duplicate Drive file stays until the next merge",
                );
            }
        }
    }
}

const MAX_BODY_BYTES: usize = MAX_DOCUMENT_BYTES;

fn endpoint(raw: &str) -> Result<Url, DocumentError> {
    Url::parse(raw).map_err(|_| transport("Google Drive endpoint is invalid."))
}

fn only_drive(provider: CloudProvider) -> Result<(), DocumentError> {
    if provider == CloudProvider::GoogleDrive {
        Ok(())
    } else {
        Err(transport("This store only serves Google Drive."))
    }
}

impl<A: DriveAuthorization> WorkspaceTransport for GoogleDrive<A> {
    fn read<'a>(&'a self, provider: CloudProvider) -> BoxFuture<'a, Option<CloudWorkspaceFile>> {
        Box::pin(async move {
            only_drive(provider)?;
            self.read_workspace().await
        })
    }

    fn write<'a>(
        &'a self,
        provider: CloudProvider,
        file: &'a CloudWorkspaceFile,
        expected_revision: Option<u64>,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            only_drive(provider)?;
            self.write_workspace(file, expected_revision).await
        })
    }

    fn disconnect(&self, provider: CloudProvider) -> Result<(), DocumentError> {
        only_drive(provider)?;
        self.auth.disconnect()
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct StoredToken {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: u64,
    client_id: String,
}

/// The Drive grant uses the same vault account and document on every PKCE host.
pub struct Pkce<V> {
    exchange: CodeExchange,
    vault: V,
    oauth: OAuthClient,
    #[cfg(target_os = "ios")]
    interactive: tokio::sync::Mutex<()>,
}

impl<V: SecretVault> Pkce<V> {
    pub fn new(vault: V, oauth: OAuthClient) -> Result<Self, DocumentError> {
        Ok(Self {
            exchange: CodeExchange::new().map_err(transport)?,
            vault,
            oauth,
            #[cfg(target_os = "ios")]
            interactive: tokio::sync::Mutex::new(()),
        })
    }

    pub fn vault(&self) -> &V {
        &self.vault
    }

    #[cfg(target_os = "ios")]
    pub async fn connect(
        &self,
        browser: &dyn crate::oauth::NativeBrowserAuth,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<(), crate::cloud_sync::CloudSyncError> {
        let _interactive = self
            .interactive
            .try_lock()
            .map_err(|_| sign_in_error(OAuthError::Busy))?;
        let redirect = self.oauth.ios_redirect().map_err(sign_in_error)?;
        let scheme = redirect.scheme().to_owned();
        let pending =
            code_exchange::begin(&self.oauth.id, redirect, DRIVE_OPTIONS).map_err(sign_in_error)?;
        let authorize = async {
            let raw = browser
                .authenticate(&pending.url, crate::oauth::BrowserCallback::Scheme(&scheme))
                .await?;
            let callback = Url::parse(&raw).map_err(|_| OAuthError::Callback)?;
            self.exchange
                .exchange(&self.oauth, &pending, &callback)
                .await
        };
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(OAuthError::Cancelled),
            result = tokio::time::timeout(Duration::from_secs(180), authorize) => result.unwrap_or(Err(OAuthError::Timeout)),
        };
        if matches!(result, Err(OAuthError::Cancelled | OAuthError::Timeout)) {
            browser.cancel().await;
        }
        self.save_grant(&self.oauth.id, result.map_err(sign_in_error)?)
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub async fn connect(
        &self,
        override_id: Option<&str>,
        browser: &dyn crate::oauth::NativeBrowserAuth,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<(), crate::cloud_sync::CloudSyncError> {
        let client_id = override_id.unwrap_or(&self.oauth.id);
        let listener = crate::oauth::loopback::Loopback::bind(42813)
            .await
            .map_err(sign_in_error)?;
        let pending = code_exchange::begin(client_id, listener.redirect().clone(), DRIVE_OPTIONS)
            .map_err(sign_in_error)?;
        let callback = listener
            .authenticate(browser, &pending, cancel)
            .await
            .map_err(sign_in_error)?;
        let token = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(OAuthError::Cancelled),
            result = self.exchange.exchange(&self.oauth, &pending, &callback) => result,
        }
        .map_err(sign_in_error)?;
        self.save_grant(client_id, token)
    }

    #[cfg(any(target_os = "ios", target_os = "macos", target_os = "windows"))]
    fn save_grant(
        &self,
        client_id: &str,
        token: code_exchange::Tokens,
    ) -> Result<(), crate::cloud_sync::CloudSyncError> {
        self.save(&StoredToken {
            access_token: token.access_token,
            refresh_token: token.refresh_token,
            expires_at: expires_at(token.expires_in),
            client_id: client_id.into(),
        })
        .map_err(|error| crate::cloud_sync::CloudSyncError::Transport(error.to_string()))
    }

    /// Whether the vault holds a usable saved authorization.
    pub fn authorized(&self) -> Result<bool, DocumentError> {
        Ok(self.stored()?.is_some_and(|token| {
            !token.access_token.is_empty() && code_exchange::valid_client_id(&token.client_id)
        }))
    }

    fn stored(&self) -> Result<Option<StoredToken>, DocumentError> {
        let Some(raw) = self.vault.load(VAULT_ACCOUNT).map_err(transport)? else {
            return Ok(None);
        };
        Ok(serde_json::from_str(&raw).ok())
    }

    fn save(&self, token: &StoredToken) -> Result<(), DocumentError> {
        let raw =
            serde_json::to_string(token).map_err(|_| transport("Token serialization failed."))?;
        self.vault.store(VAULT_ACCOUNT, &raw).map_err(transport)
    }
}

impl<V: SecretVault> DriveAuthorization for Pkce<V> {
    fn access_token<'a>(&'a self, refresh: bool) -> BoxFuture<'a, String> {
        Box::pin(async move {
            let mut token = self
                .stored()?
                .filter(|token| {
                    !token.access_token.is_empty()
                        && code_exchange::valid_client_id(&token.client_id)
                })
                .ok_or_else(|| transport("Cloud authorization is required."))?;
            if !refresh && token.expires_at > now() {
                return Ok(token.access_token);
            }
            let refresh_token = token
                .refresh_token
                .clone()
                .ok_or_else(|| transport("Cloud authorization expired; reconnect is required."))?;
            let refreshed = self
                .exchange
                .refresh(&self.oauth, &token.client_id, &refresh_token)
                .await
                .map_err(|error| {
                    transport(format!(
                        "{error} Saved authorization belongs to OAuth client {}. Reconnect if that client is no longer configured.",
                        token.client_id
                    ))
                })?;
            token.access_token = refreshed.access_token;
            if let Some(value) = refreshed.refresh_token {
                token.refresh_token = Some(value);
            }
            token.expires_at = expires_at(refreshed.expires_in);
            self.save(&token)?;
            Ok(token.access_token)
        })
    }

    fn disconnect(&self) -> Result<(), DocumentError> {
        self.vault.delete(VAULT_ACCOUNT).map_err(transport)
    }
}

#[cfg(any(target_os = "ios", target_os = "macos", target_os = "windows"))]
const DRIVE_OPTIONS: Options<'static> = Options {
    scope: SCOPE,
    prompt: "consent",
    offline: true,
    hosted_domain: None,
    nonce: None,
};

#[cfg(any(target_os = "ios", target_os = "macos", target_os = "windows"))]
fn sign_in_error(error: OAuthError) -> crate::cloud_sync::CloudSyncError {
    match error {
        OAuthError::Cancelled => crate::cloud_sync::CloudSyncError::Cancelled,
        other => crate::cloud_sync::CloudSyncError::Transport(other.to_string()),
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn expires_at(seconds: Option<u64>) -> u64 {
    let seconds = seconds
        .filter(|seconds| (60..=2_592_000).contains(seconds))
        .unwrap_or(3_600);
    now() + seconds.saturating_sub(60)
}

#[cfg(test)]
#[path = "../tests/cloud_sync__google_drive.rs"]
mod tests;
