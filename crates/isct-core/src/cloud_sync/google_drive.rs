//! Google Drive for every shell: the `drive.appdata` REST calls and the
//! PKCE authorization that desktop and iOS share. A shell supplies only an
//! access token through [`DriveAuthorization`] (and, for PKCE, the browser).

use super::document::{
    self, BoxFuture, CloudWorkspaceFile, DocumentError, WorkspaceTransport, CLOUD_FILE_NAME,
    MAX_DOCUMENT_BYTES,
};
use super::CloudProvider;
use crate::ports::SecretVault;
use base64::Engine;
use ring::rand::SecureRandom;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::Url;

/// The vault account every shell keeps the Drive token under.
pub const VAULT_ACCOUNT: &str = "google-drive";
const AUTHORIZE_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
/// The non-sensitive scope for the app's hidden folder; nothing else is asked.
pub const SCOPE: &str = "https://www.googleapis.com/auth/drive.appdata";

/// Supplies a current access token for Drive requests. `refresh` asks for a
/// new one after Drive rejected the last (401).
pub trait DriveAuthorization: Send + Sync {
    fn access_token<'a>(&'a self, refresh: bool) -> BoxFuture<'a, String>;
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
}

impl<A: DriveAuthorization> GoogleDrive<A> {
    pub fn new(auth: A) -> Result<Self, DocumentError> {
        let client = crate::net::client_builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(45))
            .build()
            .map_err(|_| transport("Cloud HTTP client initialization failed."))?;
        Ok(Self { client, auth })
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
        let files: DriveFiles = serde_json::from_slice(&read_limited(response, 1024 * 1024).await?)
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

    async fn download(&self, id: &str, etag: Option<&str>) -> Result<Vec<u8>, DocumentError> {
        // v2 honors the ETag of its own metadata for a conditional read.
        let version = if etag.is_some() { "v2" } else { "v3" };
        let response = self
            .send(
                reqwest::Method::GET,
                endpoint(&format!(
                    "https://www.googleapis.com/drive/{version}/files/{id}?alt=media"
                ))?,
                None,
                None,
                etag,
            )
            .await?;
        if response.status() == reqwest::StatusCode::PRECONDITION_FAILED {
            log::warn!("Google Drive changed during conditional read");
            return Err(DocumentError::Conflict);
        }
        if !response.status().is_success() {
            return Err(transport("Google Drive workspace read failed."));
        }
        read_limited(response, MAX_BODY_BYTES).await
    }

    /// Reads a file by name; `None` when the folder has none.
    pub async fn read_file(&self, name: &str) -> Result<Option<Vec<u8>>, DocumentError> {
        match self.file_ids(name).await?.first() {
            Some(id) => self.download(id, None).await.map(Some),
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
            let boundary = format!("isct-companion-{}", random_urlsafe(18)?);
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
        for id in self.file_ids(CLOUD_FILE_NAME).await? {
            let file = document::parse_cloud_workspace(&self.download(&id, None).await?)?;
            match current.as_mut() {
                Some(current) => current.siblings.push(file.document),
                None => current = Some(file),
            }
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
        // v2 exposes the file ETag explicitly; it is honored by conditional
        // media reads and writes of the same resource.
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
        let meta: serde_json::Value =
            serde_json::from_slice(&read_limited(meta, 65536).await?).map_err(transport)?;
        let etag = meta["etag"]
            .as_str()
            .ok_or_else(|| transport("Google Drive did not return a version tag."))?;
        let current = document::parse_cloud_workspace(&self.download(&id, Some(etag)).await?)?;
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
        Ok(())
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

async fn read_limited(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, DocumentError> {
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err(transport("Remote response is too large."));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| transport("Remote response read failed."))?
    {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(transport("Remote response is too large."));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn random_urlsafe(bytes: usize) -> Result<String, DocumentError> {
    let mut value = vec![0u8; bytes];
    ring::rand::SystemRandom::new()
        .fill(&mut value)
        .map_err(|_| transport("Secure random bytes are unavailable."))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value))
}

/// A registered Google OAuth client. Google requires a Desktop client's
/// secret in token requests even with PKCE and documents it as not
/// confidential for installed apps; iOS clients have none.
#[derive(Clone, Debug)]
pub struct OAuthClient {
    pub id: String,
    pub secret: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct StoredToken {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: u64,
    client_id: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

/// A started authorization: open `url`, then pass the redirect it returns to
/// [`Pkce::finish`] with this value.
#[derive(Clone, Debug)]
pub struct PendingAuthorization {
    pub url: String,
    redirect: String,
    verifier: String,
    state: String,
}

/// Authorization code with PKCE for installed apps, the token kept in the
/// shell's vault under [`VAULT_ACCOUNT`]. Desktop opens the system browser
/// to a loopback redirect; iOS presents a web authentication session.
pub struct Pkce<V> {
    client: reqwest::Client,
    vault: V,
    oauth: OAuthClient,
}

impl<V: SecretVault> Pkce<V> {
    pub fn new(vault: V, oauth: OAuthClient) -> Result<Self, DocumentError> {
        let client = crate::net::client_builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(45))
            .build()
            .map_err(|_| transport("Cloud HTTP client initialization failed."))?;
        Ok(Self {
            client,
            vault,
            oauth,
        })
    }

    pub fn vault(&self) -> &V {
        &self.vault
    }

    /// The authorization URL for `redirect` and the values its callback must match.
    pub fn begin(
        &self,
        client_id: &str,
        redirect: &str,
    ) -> Result<PendingAuthorization, DocumentError> {
        if !valid_client_id(client_id) {
            return Err(transport("OAuth client ID is invalid."));
        }
        let verifier = random_urlsafe(48)?;
        let state = random_urlsafe(32)?;
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        let mut url = endpoint(AUTHORIZE_ENDPOINT)?;
        url.query_pairs_mut()
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", redirect)
            .append_pair("response_type", "code")
            .append_pair("scope", SCOPE)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state)
            .append_pair("access_type", "offline")
            .append_pair("prompt", "consent");
        Ok(PendingAuthorization {
            url: url.into(),
            redirect: redirect.into(),
            verifier,
            state,
        })
    }

    /// Checks the redirect the browser returned, exchanges its code and
    /// stores the token. `client_id` is the one `begin` was given.
    pub async fn finish(
        &self,
        client_id: &str,
        pending: &PendingAuthorization,
        callback: &str,
    ) -> Result<(), DocumentError> {
        let code = callback_code(pending, callback)?;
        let mut form = vec![
            ("client_id", client_id),
            ("code", code.as_str()),
            ("code_verifier", pending.verifier.as_str()),
            ("redirect_uri", pending.redirect.as_str()),
            ("grant_type", "authorization_code"),
        ];
        if let Some(secret) = self.secret_for(client_id) {
            form.push(("client_secret", secret));
        }
        let token = self
            .token_request(&form, "OAuth token exchange was rejected.")
            .await?;
        self.save(&StoredToken {
            access_token: token.access_token,
            refresh_token: token.refresh_token,
            expires_at: expires_at(token.expires_in),
            client_id: client_id.into(),
        })
    }

    /// Whether the vault holds a usable saved authorization.
    pub fn authorized(&self) -> Result<bool, DocumentError> {
        Ok(self.stored()?.is_some_and(|token| {
            !token.access_token.is_empty() && valid_client_id(&token.client_id)
        }))
    }

    fn secret_for(&self, client_id: &str) -> Option<&str> {
        // Only sent with the client it belongs to, not a development override.
        (client_id == self.oauth.id)
            .then_some(self.oauth.secret.as_deref())
            .flatten()
            .filter(|secret| !secret.is_empty())
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

    async fn token_request(
        &self,
        form: &[(&str, &str)],
        rejected: &str,
    ) -> Result<TokenResponse, DocumentError> {
        let response = self
            .client
            .post(TOKEN_ENDPOINT)
            .form(form)
            .send()
            .await
            .map_err(|_| transport("OAuth token request failed."))?;
        if response.status().is_redirection() || !response.status().is_success() {
            return Err(transport(rejection(rejected, response).await));
        }
        let token: TokenResponse =
            serde_json::from_slice(&read_limited(response, 1024 * 1024).await?)
                .map_err(|_| transport("OAuth token response was invalid."))?;
        if token.access_token.is_empty() {
            return Err(transport("OAuth token response was invalid."));
        }
        Ok(token)
    }
}

impl<V: SecretVault> DriveAuthorization for Pkce<V> {
    fn access_token<'a>(&'a self, refresh: bool) -> BoxFuture<'a, String> {
        Box::pin(async move {
            let mut token = self
                .stored()?
                .filter(|token| !token.access_token.is_empty() && valid_client_id(&token.client_id))
                .ok_or_else(|| transport("Cloud authorization is required."))?;
            if !refresh && token.expires_at > now() {
                return Ok(token.access_token);
            }
            let refresh_token = token
                .refresh_token
                .clone()
                .ok_or_else(|| transport("Cloud authorization expired; reconnect is required."))?;
            let mut form = vec![
                ("client_id", token.client_id.as_str()),
                ("refresh_token", refresh_token.as_str()),
                ("grant_type", "refresh_token"),
            ];
            if let Some(secret) = self.secret_for(&token.client_id) {
                form.push(("client_secret", secret));
            }
            let refreshed = self
                .token_request(&form, "Cloud token refresh was rejected.")
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

/// The authorization code of a redirect, after checking it answers `pending`.
fn callback_code(pending: &PendingAuthorization, callback: &str) -> Result<String, DocumentError> {
    let callback = Url::parse(callback).map_err(|_| transport("OAuth callback is invalid."))?;
    let expected =
        Url::parse(&pending.redirect).map_err(|_| transport("OAuth redirect is invalid."))?;
    if callback.scheme() != expected.scheme()
        || callback.host_str() != expected.host_str()
        || callback.port() != expected.port()
        || callback.path() != expected.path()
    {
        return Err(transport("OAuth callback target is invalid."));
    }
    let params: BTreeMap<_, _> = callback.query_pairs().into_owned().collect();
    if params.get("state") != Some(&pending.state) {
        return Err(transport("OAuth state did not match."));
    }
    if params.contains_key("error") {
        return Err(transport("OAuth authorization was declined."));
    }
    params
        .get("code")
        .filter(|value| !value.is_empty())
        .cloned()
        .ok_or_else(|| transport("OAuth authorization returned no code."))
}

fn valid_client_id(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
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

/// Rejection message with the provider's own error code, e.g.
/// `invalid_client`, so a misconfigured client is diagnosable.
async fn rejection(prefix: &str, response: reqwest::Response) -> String {
    #[derive(Deserialize)]
    struct OAuthError {
        error: Option<String>,
        error_description: Option<String>,
    }
    let status = response.status().as_u16();
    let detail = read_limited(response, 64 * 1024)
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice::<OAuthError>(&bytes).ok())
        .and_then(|body| match (body.error, body.error_description) {
            (Some(code), Some(text)) => Some(format!("{code}: {text}")),
            (Some(code), None) => Some(code),
            (None, text) => text,
        })
        .map(|text| {
            text.chars()
                .filter(|c| !c.is_control())
                .take(200)
                .collect::<String>()
        });
    match detail {
        Some(text) => format!("{prefix} ({status} {text})"),
        None => format!("{prefix} ({status})"),
    }
}

#[cfg(test)]
#[path = "../tests/cloud_sync__google_drive.rs"]
mod tests;
