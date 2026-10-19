use base64::Engine;
use isct_core::cloud_credentials;
use isct_core::cloud_sync::document::{
    self, CloudWorkspaceFile, DocumentError, WorkspaceTransport,
};
pub use isct_core::cloud_sync::CloudProvider;
#[cfg(test)]
use isct_core::ports::SecretError;
use isct_core::ports::SecretVault;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Read;
use std::io::Write;
use std::net::TcpListener;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::Url;

const MAX_BODY_BYTES: usize = 25 * 1024 * 1024;
const CLOUD_FILE_NAME: &str = "IsctCompanion.json";

#[derive(Debug, Deserialize)]
struct GoogleDriveFile {
    id: String,
}

#[derive(Debug, Deserialize)]
struct GoogleDriveFiles {
    files: Option<Vec<GoogleDriveFile>>,
}

pub fn validate_redirect_uri(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|_| "OAuth redirect URI is invalid.".to_string())?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.port().is_none()
        || url.path() != "/oauth/callback"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("OAuth redirect must be an exact loopback callback.".into());
    }
    Ok(url)
}

pub fn validate_pkce_state(verifier: &str, state: &str) -> Result<(), String> {
    let url_safe = |value: &str| {
        value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'))
    };
    if !(43..=128).contains(&verifier.len()) || !url_safe(verifier) {
        return Err("PKCE verifier is invalid.".into());
    }
    if state.len() < 32 || state.len() > 128 || !url_safe(state) {
        return Err("OAuth state is invalid.".into());
    }
    Ok(())
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

/// Google requires the client secret of a Desktop OAuth client in token
/// requests, even with PKCE; Google documents it as not confidential for
/// installed apps. `scripts/desktop/tauri.mjs` supplies both values at build time
/// from config/oauth-clients.yaml, and the secret is only sent with the
/// client it belongs to (not with a development override).
const GOOGLE_DESKTOP_CLIENT_ID: Option<&str> = option_env!("ISCT_GOOGLE_DESKTOP_CLIENT_ID");
const GOOGLE_DESKTOP_CLIENT_SECRET: Option<&str> = option_env!("ISCT_GOOGLE_DESKTOP_CLIENT_SECRET");

fn client_secret_for(provider: CloudProvider, client_id: &str) -> Option<&'static str> {
    if provider != CloudProvider::GoogleDrive || GOOGLE_DESKTOP_CLIENT_ID != Some(client_id) {
        return None;
    }
    GOOGLE_DESKTOP_CLIENT_SECRET.filter(|secret| !secret.is_empty())
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

fn provider_account(provider: CloudProvider) -> &'static str {
    match provider {
        CloudProvider::GoogleDrive => "google-drive",
        CloudProvider::Onedrive => "onedrive",
        CloudProvider::Icloud => "icloud",
    }
}

fn providers() -> [CloudProvider; 3] {
    [
        CloudProvider::GoogleDrive,
        CloudProvider::Onedrive,
        CloudProvider::Icloud,
    ]
}

fn oauth_endpoints(provider: CloudProvider) -> (&'static str, &'static str, &'static str) {
    match provider {
        CloudProvider::GoogleDrive => (
            "https://accounts.google.com/o/oauth2/v2/auth",
            "https://oauth2.googleapis.com/token",
            "https://www.googleapis.com/auth/drive.appdata",
        ),
        CloudProvider::Onedrive => (
            "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
            "https://login.microsoftonline.com/common/oauth2/v2.0/token",
            "Files.ReadWrite.AppFolder offline_access",
        ),
        CloudProvider::Icloud => ("", "", ""),
    }
}

fn random_urlsafe(bytes: usize) -> String {
    let mut value = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut value);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value)
}

async fn read_limited(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err("Remote response is too large.".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Remote response read failed.".to_string())?
    {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err("Remote response is too large.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn valid_client_id(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}

fn bounded_expiry(value: Option<u64>) -> u64 {
    value
        .filter(|seconds| (60..=2_592_000).contains(seconds))
        .unwrap_or(3_600)
}

pub struct DesktopCloudService<V: SecretVault> {
    client: reqwest::Client,
    vault: V,
}

impl<V: SecretVault> DesktopCloudService<V> {
    pub fn new(vault: V) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(45))
            .build()
            .map_err(|_| "Cloud HTTP client initialization failed.".to_string())?;
        Ok(Self { client, vault })
    }

    /// The PKCE authorization request for a redirect; returns the URL to open
    /// and the verifier and state that the callback must match.
    fn authorization_request(
        provider: CloudProvider,
        client_id: &str,
        redirect: &Url,
    ) -> Result<(Url, String, String), String> {
        if !valid_client_id(client_id) {
            return Err("OAuth client ID is invalid.".into());
        }
        let verifier = random_urlsafe(48);
        let state = random_urlsafe(32);
        validate_pkce_state(&verifier, &state)?;
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        let (authorize_endpoint, _, scope) = oauth_endpoints(provider);
        let mut authorize =
            Url::parse(authorize_endpoint).map_err(|_| "OAuth endpoint is invalid.")?;
        authorize
            .query_pairs_mut()
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", redirect.as_str())
            .append_pair("response_type", "code")
            .append_pair("scope", scope)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state);
        if provider == CloudProvider::GoogleDrive {
            authorize
                .query_pairs_mut()
                .append_pair("access_type", "offline")
                .append_pair("prompt", "consent");
        }
        Ok((authorize, verifier, state))
    }

    /// Desktop sign-in: the system browser returns to a loopback listener.
    pub async fn connect<F>(
        &self,
        provider: CloudProvider,
        client_id: &str,
        redirect_uri: &str,
        open_browser: F,
    ) -> Result<bool, String>
    where
        F: FnOnce(&str) -> Result<(), String>,
    {
        let redirect = validate_redirect_uri(redirect_uri)?;
        let (authorize, verifier, state) =
            Self::authorization_request(provider, client_id, &redirect)?;
        let listener = TcpListener::bind(("127.0.0.1", redirect.port().unwrap()))
            .map_err(|_| "OAuth loopback port is unavailable.".to_string())?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "OAuth callback listener failed.".to_string())?;
        open_browser(authorize.as_str())?;
        let callback = tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
            let deadline = SystemTime::now() + Duration::from_secs(180);
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut bytes = [0u8; 8192];
                        let size = stream.read(&mut bytes).map_err(|_| "OAuth callback read failed.".to_string())?;
                        let request = String::from_utf8_lossy(&bytes[..size]);
                        let mut request_parts = request.lines().next().map(str::split_whitespace)
                            .ok_or_else(|| "OAuth callback is invalid.".to_string())?;
                        if request_parts.next() != Some("GET") { return Err("OAuth callback method is invalid.".into()); }
                        let target = request_parts.next().ok_or_else(|| "OAuth callback is invalid.".to_string())?;
                        if !target.starts_with("/oauth/callback?") || target.starts_with("//") {
                            return Err("OAuth callback target is invalid.".into());
                        }
                        let callback = Url::parse(&format!("http://127.0.0.1{}", target))
                            .map_err(|_| "OAuth callback is invalid.".to_string())?;
                        if callback.path() != "/oauth/callback" {
                            return Err("OAuth callback path is invalid.".into());
                        }
                        let params: BTreeMap<_, _> = callback.query_pairs().into_owned().collect();
                        if params.get("state") != Some(&state) { return Err("OAuth state did not match.".into()); }
                        if params.contains_key("error") { return Err("OAuth authorization was declined.".into()); }
                        let code = params.get("code").filter(|value| !value.is_empty()).cloned()
                            .ok_or_else(|| "OAuth authorization returned no code.".to_string())?;
                        let response = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\nAuthorization completed. You may close this window now.";
                        let _ = stream.write_all(response);
                        return Ok(code);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if SystemTime::now() >= deadline { return Err("OAuth authorization timed out.".into()); }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    Err(_) => return Err("OAuth callback listener failed.".into()),
                }
            }
        }).await.map_err(|_| "OAuth callback task failed.".to_string())??;
        self.finish_connect(provider, client_id, &redirect, &verifier, &callback)
            .await
    }

    /// Exchanges an authorization code and stores the token in the vault.
    async fn finish_connect(
        &self,
        provider: CloudProvider,
        client_id: &str,
        redirect: &Url,
        verifier: &str,
        callback: &str,
    ) -> Result<bool, String> {
        let (_, token_endpoint, _) = oauth_endpoints(provider);
        let response = self
            .client
            .post(token_endpoint)
            .form(&{
                let mut form = vec![
                    ("client_id", client_id),
                    ("code", callback),
                    ("code_verifier", verifier),
                    ("redirect_uri", redirect.as_str()),
                    ("grant_type", "authorization_code"),
                ];
                if let Some(secret) = client_secret_for(provider, client_id) {
                    form.push(("client_secret", secret));
                }
                form
            })
            .send()
            .await
            .map_err(|_| "OAuth token exchange failed.".to_string())?;
        if response.status().is_redirection() || !response.status().is_success() {
            return Err(rejection("OAuth token exchange was rejected.", response).await);
        }
        let token_bytes = read_limited(response, 1024 * 1024).await?;
        let token: TokenResponse = serde_json::from_slice(&token_bytes)
            .map_err(|_| "OAuth token response was invalid.".to_string())?;
        if token.access_token.is_empty() {
            return Err("OAuth token response was invalid.".into());
        }
        let stored = StoredToken {
            access_token: token.access_token,
            refresh_token: token.refresh_token,
            expires_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                + bounded_expiry(token.expires_in).saturating_sub(60),
            client_id: client_id.to_string(),
        };
        self.vault
            .store(
                provider_account(provider),
                &serde_json::to_string(&stored).map_err(|_| "Token serialization failed.")?,
            )
            .map_err(|error| error.to_string())?;
        Ok(true)
    }

    pub fn authorized_providers(&self) -> Result<Vec<CloudProvider>, String> {
        let mut authorized = Vec::new();
        for provider in providers() {
            let Some(raw) = self
                .vault
                .load(provider_account(provider))
                .map_err(|error| error.to_string())?
            else {
                continue;
            };
            if provider == CloudProvider::Icloud && raw == "connected" {
                authorized.push(provider);
                continue;
            }
            let Ok(token) = serde_json::from_str::<StoredToken>(&raw) else {
                continue;
            };
            if !token.access_token.is_empty() && valid_client_id(&token.client_id) {
                authorized.push(provider);
            }
        }
        Ok(authorized)
    }

    pub fn disconnect(&self, provider: CloudProvider) -> Result<(), String> {
        self.vault
            .delete(provider_account(provider))
            .map_err(|error| error.to_string())
    }

    #[cfg(target_os = "macos")]
    pub async fn connect_icloud(&self) -> Result<bool, String> {
        crate::icloud::account_available().await?;
        self.vault
            .store(provider_account(CloudProvider::Icloud), "connected")
            .map_err(|error| error.to_string())?;
        Ok(true)
    }

    async fn access_token(&self, provider: CloudProvider) -> Result<String, String> {
        if provider == CloudProvider::Icloud {
            return Err("iCloud does not use OAuth tokens.".into());
        }
        let raw = self
            .vault
            .load(provider_account(provider))
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Cloud authorization is required.".to_string())?;
        let mut token: StoredToken = serde_json::from_str(&raw)
            .map_err(|_| "Stored cloud authorization is invalid.".to_string())?;
        if token.access_token.is_empty() || !valid_client_id(&token.client_id) {
            return Err("Stored cloud authorization is invalid.".into());
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if token.expires_at <= now {
            let refresh = token
                .refresh_token
                .clone()
                .ok_or_else(|| "Cloud authorization expired; reconnect is required.".to_string())?;
            let (_, endpoint, _) = oauth_endpoints(provider);
            let response = self
                .client
                .post(endpoint)
                .form(&{
                    let mut form = vec![
                        ("client_id", token.client_id.as_str()),
                        ("refresh_token", refresh.as_str()),
                        ("grant_type", "refresh_token"),
                    ];
                    if let Some(secret) = client_secret_for(provider, &token.client_id) {
                        form.push(("client_secret", secret));
                    }
                    form
                })
                .send()
                .await
                .map_err(|_| "Cloud token refresh failed.".to_string())?;
            if response.status().is_redirection() || !response.status().is_success() {
                return Err(format!("{} Saved authorization belongs to OAuth client {}. Reconnect if that client is no longer configured.", rejection("Cloud token refresh was rejected.", response).await, token.client_id));
            }
            let refreshed_bytes = read_limited(response, 1024 * 1024).await?;
            let refreshed: TokenResponse = serde_json::from_slice(&refreshed_bytes)
                .map_err(|_| "Cloud token refresh response was invalid.".to_string())?;
            if refreshed.access_token.is_empty() {
                return Err("Cloud token refresh response was invalid.".into());
            }
            token.access_token = refreshed.access_token;
            if let Some(value) = refreshed.refresh_token {
                token.refresh_token = Some(value);
            }
            token.expires_at = now + bounded_expiry(refreshed.expires_in).saturating_sub(60);
            self.vault
                .store(
                    provider_account(provider),
                    &serde_json::to_string(&token).map_err(|_| "Token serialization failed.")?,
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(token.access_token)
    }

    async fn send(
        &self,
        provider: CloudProvider,
        method: reqwest::Method,
        url: Url,
        content_type: Option<&str>,
        body: Option<Vec<u8>>,
    ) -> Result<reqwest::Response, String> {
        self.send_matching(provider, method, url, content_type, body, None)
            .await
    }

    async fn send_matching(
        &self,
        provider: CloudProvider,
        method: reqwest::Method,
        url: Url,
        content_type: Option<&str>,
        body: Option<Vec<u8>>,
        etag: Option<&str>,
    ) -> Result<reqwest::Response, String> {
        let mut request = self
            .client
            .request(method, url)
            .bearer_auth(self.access_token(provider).await?);
        if let Some(etag) = etag {
            request = request.header(reqwest::header::IF_MATCH, etag);
        }
        if let Some(content_type) = content_type {
            request = request.header(reqwest::header::CONTENT_TYPE, content_type);
        }
        if let Some(body) = body {
            if body.len() > MAX_BODY_BYTES {
                return Err("Cloud request body is too large.".into());
            }
            request = request.body(body);
        }
        let response = request
            .send()
            .await
            .map_err(|_| "Cloud request failed.".to_string())?;
        if response.status().is_redirection() {
            return Err("Cloud redirects are not followed.".into());
        }
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err("Cloud authorization expired; reconnect is required.".into());
        }
        Ok(response)
    }

    async fn google_file_id(&self, name: &str) -> Result<Option<String>, String> {
        Ok(self.google_file_ids(name).await?.into_iter().next())
    }
    async fn google_file_ids(&self, name: &str) -> Result<Vec<String>, String> {
        let mut url = Url::parse("https://www.googleapis.com/drive/v3/files")
            .map_err(|_| "Google Drive endpoint is invalid.".to_string())?;
        url.query_pairs_mut()
            .append_pair("spaces", "appDataFolder")
            .append_pair("q", &format!("name = '{}' and trashed = false", name))
            .append_pair("fields", "files(id)")
            .append_pair("pageSize", "100");
        let response = self
            .send(
                CloudProvider::GoogleDrive,
                reqwest::Method::GET,
                url,
                None,
                None,
            )
            .await?;
        if !response.status().is_success() {
            return Err(format!(
                "Google Drive lookup failed ({}).",
                response.status().as_u16()
            ));
        }
        let bytes = read_limited(response, 1024 * 1024).await?;
        let files: GoogleDriveFiles = serde_json::from_slice(&bytes)
            .map_err(|_| "Google Drive lookup response was invalid.".to_string())?;
        let mut ids: Vec<_> = files
            .files
            .unwrap_or_default()
            .into_iter()
            .map(|file| file.id)
            .collect();
        ids.sort();
        Ok(ids)
    }

    async fn read_file(
        &self,
        provider: CloudProvider,
        name: &str,
    ) -> Result<Option<Vec<u8>>, String> {
        let response = match provider {
            CloudProvider::GoogleDrive => {
                let Some(id) = self.google_file_id(name).await? else {
                    return Ok(None);
                };
                let mut url =
                    Url::parse(&format!("https://www.googleapis.com/drive/v3/files/{id}"))
                        .map_err(|_| "Google Drive endpoint is invalid.".to_string())?;
                url.query_pairs_mut().append_pair("alt", "media");
                self.send(provider, reqwest::Method::GET, url, None, None)
                    .await?
            }
            CloudProvider::Onedrive => {
                let url = Url::parse(&format!(
                    "https://graph.microsoft.com/v1.0/me/drive/special/approot:/{name}:/content"
                ))
                .map_err(|_| "OneDrive endpoint is invalid.".to_string())?;
                let response = self
                    .send(provider, reqwest::Method::GET, url, None, None)
                    .await?;
                if response.status() == reqwest::StatusCode::NOT_FOUND {
                    return Ok(None);
                }
                response
            }
            CloudProvider::Icloud => {
                #[cfg(target_os = "macos")]
                return crate::icloud::read(name).await;
                #[cfg(not(target_os = "macos"))]
                return Err("iCloud sync is only available on Apple devices.".into());
            }
        };
        if !response.status().is_success() {
            return Err(format!(
                "Cloud file read failed ({}).",
                response.status().as_u16()
            ));
        }
        read_limited(response, MAX_BODY_BYTES).await.map(Some)
    }

    async fn write_file(
        &self,
        provider: CloudProvider,
        name: &str,
        bytes: Vec<u8>,
    ) -> Result<(), String> {
        let response = match provider {
            CloudProvider::GoogleDrive => {
                if let Some(id) = self.google_file_id(name).await? {
                    let mut url = Url::parse(&format!(
                        "https://www.googleapis.com/upload/drive/v3/files/{id}"
                    ))
                    .map_err(|_| "Google Drive endpoint is invalid.".to_string())?;
                    url.query_pairs_mut().append_pair("uploadType", "media");
                    self.send(
                        provider,
                        reqwest::Method::PATCH,
                        url,
                        Some("application/json"),
                        Some(bytes),
                    )
                    .await?
                } else {
                    let boundary = format!("isct-companion-{}", random_urlsafe(18));
                    let metadata = serde_json::to_string(&serde_json::json!({
                        "name": name,
                        "parents": ["appDataFolder"]
                    }))
                    .map_err(|_| "Google Drive metadata serialization failed.".to_string())?;
                    let mut body = Vec::new();
                    write!(
                        body,
                        "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n--{boundary}\r\nContent-Type: application/json\r\n\r\n"
                    )
                    .map_err(|_| "Google Drive upload could not be prepared.".to_string())?;
                    body.extend_from_slice(&bytes);
                    write!(body, "\r\n--{boundary}--\r\n")
                        .map_err(|_| "Google Drive upload could not be prepared.".to_string())?;
                    let mut url = Url::parse("https://www.googleapis.com/upload/drive/v3/files")
                        .map_err(|_| "Google Drive endpoint is invalid.".to_string())?;
                    url.query_pairs_mut().append_pair("uploadType", "multipart");
                    self.send(
                        provider,
                        reqwest::Method::POST,
                        url,
                        Some(&format!("multipart/related; boundary={boundary}")),
                        Some(body),
                    )
                    .await?
                }
            }
            CloudProvider::Onedrive => {
                let url = Url::parse(&format!(
                    "https://graph.microsoft.com/v1.0/me/drive/special/approot:/{name}:/content"
                ))
                .map_err(|_| "OneDrive endpoint is invalid.".to_string())?;
                self.send(
                    provider,
                    reqwest::Method::PUT,
                    url,
                    Some("application/json"),
                    Some(bytes),
                )
                .await?
            }
            CloudProvider::Icloud => {
                #[cfg(target_os = "macos")]
                {
                    return crate::icloud::write(name, bytes).await;
                }
                #[cfg(not(target_os = "macos"))]
                return Err("iCloud sync is only available on Apple devices.".into());
            }
        };
        if !response.status().is_success() {
            return Err(format!(
                "Cloud file write failed ({}).",
                response.status().as_u16()
            ));
        }
        Ok(())
    }

    pub(crate) async fn read_workspace(
        &self,
        provider: CloudProvider,
    ) -> Result<Option<CloudWorkspaceFile>, String> {
        if provider == CloudProvider::GoogleDrive {
            // Drive names are not unique. Two first-time offline devices can
            // create siblings; choose a stable canonical id and retain both
            // documents until their union has been committed there.
            let mut current: Option<CloudWorkspaceFile> = None;
            for id in self.google_file_ids(CLOUD_FILE_NAME).await? {
                let response = self
                    .send(
                        provider,
                        reqwest::Method::GET,
                        Url::parse(&format!(
                            "https://www.googleapis.com/drive/v3/files/{id}?alt=media"
                        ))
                        .map_err(|e| e.to_string())?,
                        None,
                        None,
                    )
                    .await?;
                if !response.status().is_success() {
                    return Err("Google Drive workspace read failed.".into());
                }
                let file =
                    document::parse_cloud_workspace(&read_limited(response, MAX_BODY_BYTES).await?)
                        .map_err(|error| error.to_string())?;
                if let Some(current) = current.as_mut() {
                    current.siblings.push(file.document);
                } else {
                    current = Some(file);
                }
            }
            Ok(current)
        } else {
            self.read_file(provider, CLOUD_FILE_NAME)
                .await?
                .map(|bytes| {
                    document::parse_cloud_workspace(&bytes).map_err(|error| error.to_string())
                })
                .transpose()
        }
    }

    pub(crate) async fn versions(
        &self,
        provider: CloudProvider,
    ) -> Result<Vec<crate::cloud_sync::CloudVersion>, String> {
        Ok(self
            .read_workspace(provider)
            .await?
            .map(|file| document::versions(provider, &file))
            .unwrap_or_default())
    }

    pub(crate) async fn version_document(
        &self,
        provider: CloudProvider,
        revision: u64,
    ) -> Result<Value, String> {
        self.read_workspace(provider)
            .await?
            .and_then(|file| file.version_document(revision).cloned())
            .ok_or_else(|| DocumentError::VersionMissing.to_string())
    }

    /// Re-read before committing: another device may have written since the
    /// multi-provider pull phase. Conditional writes retain that guarantee.
    pub(crate) async fn merge_and_push(
        &self,
        provider: CloudProvider,
        document: &Value,
        device: &str,
    ) -> Result<Value, String> {
        let device_name = std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .unwrap_or_else(|_| {
                format!(
                    "{} · {}",
                    std::env::consts::OS,
                    device.chars().take(8).collect::<String>()
                )
            });
        document::merge_and_push(self, provider, document, device, Some(&device_name))
            .await
            .map_err(|error| error.to_string())
    }

    async fn write_workspace_file(
        &self,
        provider: CloudProvider,
        file: &CloudWorkspaceFile,
        expected: Option<u64>,
    ) -> Result<(), DocumentError> {
        let transport = DocumentError::Transport;
        let bytes = serde_json::to_vec(file).map_err(|e| transport(e.to_string()))?;
        if bytes.len() > MAX_BODY_BYTES {
            return Err(transport("Cloud workspace exceeds the size limit.".into()));
        }
        match provider {
            CloudProvider::GoogleDrive => {
                if let Some(id) = self
                    .google_file_id(CLOUD_FILE_NAME)
                    .await
                    .map_err(transport)?
                {
                    // v2 exposes the file ETag explicitly; it is honored by
                    // conditional media reads and writes of the same resource.
                    let meta = self
                        .send(
                            provider,
                            reqwest::Method::GET,
                            Url::parse(&format!(
                                "https://www.googleapis.com/drive/v2/files/{id}?fields=etag"
                            ))
                            .map_err(|e| transport(e.to_string()))?,
                            None,
                            None,
                        )
                        .await
                        .map_err(transport)?;
                    if !meta.status().is_success() {
                        return Err(transport("Google Drive metadata read failed.".into()));
                    }
                    let meta: Value = serde_json::from_slice(
                        &read_limited(meta, 65536).await.map_err(transport)?,
                    )
                    .map_err(|e| transport(e.to_string()))?;
                    let etag = meta["etag"].as_str().ok_or_else(|| {
                        transport("Google Drive did not return a version tag.".into())
                    })?;
                    let current = self
                        .send_matching(
                            provider,
                            reqwest::Method::GET,
                            Url::parse(&format!(
                                "https://www.googleapis.com/drive/v2/files/{id}?alt=media"
                            ))
                            .map_err(|e| transport(e.to_string()))?,
                            None,
                            None,
                            Some(etag),
                        )
                        .await
                        .map_err(transport)?;
                    if current.status() == reqwest::StatusCode::PRECONDITION_FAILED {
                        log::warn!("Google Drive changed during conditional read");
                        return Err(DocumentError::Conflict);
                    }
                    if !current.status().is_success() {
                        return Err(transport("Google Drive workspace read failed.".into()));
                    }
                    let current = document::parse_cloud_workspace(
                        &read_limited(current, MAX_BODY_BYTES)
                            .await
                            .map_err(transport)?,
                    )?;
                    if Some(current.revision) != expected
                        || file.previous.first().is_some_and(|previous| {
                            !document::workspace_contents_equal(
                                &current.document,
                                &previous.document,
                            )
                        })
                    {
                        log::warn!(
                            "Google Drive changed before write: expected {:?}, observed {}",
                            expected,
                            current.revision
                        );
                        return Err(DocumentError::Conflict);
                    }
                    let response = self.send_matching(provider,reqwest::Method::PUT,Url::parse(&format!("https://www.googleapis.com/upload/drive/v2/files/{id}?uploadType=media")).map_err(|e|transport(e.to_string()))?,Some("application/json"),Some(bytes),Some(etag)).await.map_err(transport)?;
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
                } else if expected.is_none() {
                    self.write_file(provider, CLOUD_FILE_NAME, bytes)
                        .await
                        .map_err(transport)
                } else {
                    Err(DocumentError::Conflict)
                }
            }
            #[cfg(target_vendor = "apple")]
            CloudProvider::Icloud => {
                crate::icloud::write_checked(CLOUD_FILE_NAME, bytes, expected).await
            }
            _ => self
                .write_file(provider, CLOUD_FILE_NAME, bytes)
                .await
                .map_err(transport),
        }
    }

    pub fn credential_sync_configured(&self) -> Result<bool, String> {
        Ok(self
            .vault
            .load(cloud_credentials::PASSWORD_ACCOUNT)
            .map_err(|error| error.to_string())?
            .is_some())
    }

    pub fn set_credential_sync_password(&self, password: &str) -> Result<(), String> {
        cloud_credentials::validate_password(password).map_err(|error| error.to_string())?;
        self.vault
            .store(cloud_credentials::PASSWORD_ACCOUNT, password)
            .map_err(|error| error.to_string())
    }

    pub fn clear_credential_sync_password(&self) -> Result<(), String> {
        self.vault
            .delete(cloud_credentials::PASSWORD_ACCOUNT)
            .map_err(|error| error.to_string())
    }

    pub async fn credential_backup_exists(&self, provider: CloudProvider) -> Result<bool, String> {
        if !cloud_credentials::supports_provider(provider) {
            return Ok(false);
        }
        Ok(self
            .read_file(provider, cloud_credentials::FILE_NAME)
            .await?
            .is_some())
    }

    pub async fn back_up_credentials(
        &self,
        provider: CloudProvider,
        payload: &str,
    ) -> Result<(), String> {
        let password = self
            .vault
            .load(cloud_credentials::PASSWORD_ACCOUNT)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Credential sync is not configured on this device.".to_string())?;
        let encrypted =
            cloud_credentials::encrypt(payload, &password).map_err(|error| error.to_string())?;
        self.write_file(provider, cloud_credentials::FILE_NAME, encrypted)
            .await
    }

    pub async fn pull_credentials(
        &self,
        provider: CloudProvider,
    ) -> Result<Option<String>, String> {
        let Some(password) = self
            .vault
            .load(cloud_credentials::PASSWORD_ACCOUNT)
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        let Some(encrypted) = self
            .read_file(provider, cloud_credentials::FILE_NAME)
            .await?
        else {
            return Ok(None);
        };
        cloud_credentials::decrypt(&encrypted, &password)
            .map(Some)
            .map_err(|error| error.to_string())
    }
}

impl<V: SecretVault> WorkspaceTransport for DesktopCloudService<V> {
    fn read<'a>(
        &'a self,
        provider: CloudProvider,
    ) -> document::BoxFuture<'a, Option<CloudWorkspaceFile>> {
        Box::pin(async move {
            self.read_workspace(provider)
                .await
                .map_err(DocumentError::Transport)
        })
    }

    fn write<'a>(
        &'a self,
        provider: CloudProvider,
        file: &'a CloudWorkspaceFile,
        expected_revision: Option<u64>,
    ) -> document::BoxFuture<'a, ()> {
        Box::pin(async move {
            self.write_workspace_file(provider, file, expected_revision)
                .await
        })
    }
}

#[cfg(test)]
#[path = "tests/cloud.rs"]
mod tests;
