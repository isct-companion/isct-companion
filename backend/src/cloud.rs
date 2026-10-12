use crate::secrets::SecretVault;
use base64::Engine;
use rand::RngCore;
use ring::{aead, pbkdf2};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(desktop)]
use sha2::{Digest, Sha256};
#[cfg(desktop)]
use std::collections::BTreeMap;
#[cfg(desktop)]
use std::io::Read;
use std::io::Write;
#[cfg(desktop)]
use std::net::TcpListener;
use std::num::NonZeroU32;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::Url;

const MAX_BODY_BYTES: usize = 25 * 1024 * 1024;
const CLOUD_FILE_NAME: &str = "IsctCompanion.json";
const CLOUD_FORMAT: &str = "IsctCompanion";
const CLOUD_FORMAT_VERSION: u64 = 1;
const CREDENTIAL_FILE_NAME: &str = "IsctCompanion.credentials.json";
const CREDENTIAL_FORMAT: &str = "IsctCompanion.credentials";
const CREDENTIAL_PASSWORD_ACCOUNT: &str = "cloud-credential-sync";
const CREDENTIAL_KDF_ITERATIONS: u32 = 600_000;
const CREDENTIAL_AAD: &[u8] = b"IsctCompanion.credentials.v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CloudProvider {
    GoogleDrive,
    Onedrive,
    Icloud,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CloudWorkspaceFile {
    format: String,
    format_version: u64,
    revision: u64,
    updated_at: String,
    updated_by: String,
    document: Value,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CloudSyncState {
    BackedUp,
    UpToDate,
    Conflict,
    Restored,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudSyncResult {
    pub state: CloudSyncState,
    pub revision: u64,
    pub updated_at: String,
    pub updated_by: String,
}

#[derive(Debug, Deserialize)]
struct GoogleDriveFile {
    id: String,
}

#[derive(Debug, Deserialize)]
struct GoogleDriveFiles {
    files: Option<Vec<GoogleDriveFile>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EncryptedCredentialFile {
    format: String,
    format_version: u64,
    kdf: String,
    iterations: u32,
    salt: String,
    cipher: String,
    nonce: String,
    ciphertext: String,
}

#[cfg(desktop)]
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

#[cfg(desktop)]
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
/// installed apps. `scripts/tauri.mjs` supplies both values at build time
/// from config/oauth-clients.yaml, and the secret is only sent with the
/// client it belongs to (not with a development override).
#[cfg(desktop)]
const GOOGLE_DESKTOP_CLIENT_ID: Option<&str> = option_env!("ISCT_GOOGLE_DESKTOP_CLIENT_ID");
#[cfg(desktop)]
const GOOGLE_DESKTOP_CLIENT_SECRET: Option<&str> =
    option_env!("ISCT_GOOGLE_DESKTOP_CLIENT_SECRET");

#[cfg(desktop)]
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
        .map(|text| text.chars().filter(|c| !c.is_control()).take(200).collect::<String>());
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

fn validate_workspace_document(document: &Value) -> Result<(), String> {
    if document.get("schemaVersion").and_then(Value::as_u64) != Some(6)
        || document.get("kind").and_then(Value::as_str) != Some("IsctCompanion.local-backup")
        || !document.get("tables").is_some_and(Value::is_object)
        || !document.get("localNotes").is_some_and(Value::is_array)
    {
        return Err("Cloud workspace format is invalid or unsupported.".into());
    }
    Ok(())
}

fn parse_cloud_workspace(bytes: &[u8]) -> Result<CloudWorkspaceFile, String> {
    let file: CloudWorkspaceFile = serde_json::from_slice(bytes)
        .map_err(|_| "Cloud workspace is not valid JSON.".to_string())?;
    if file.format != CLOUD_FORMAT
        || file.format_version != CLOUD_FORMAT_VERSION
        || file.revision == 0
        || file.updated_by.trim().is_empty()
        || chrono::DateTime::parse_from_rfc3339(&file.updated_at).is_err()
    {
        return Err("Cloud workspace format is invalid or unsupported.".into());
    }
    validate_workspace_document(&file.document)?;
    Ok(file)
}

fn sync_result(state: CloudSyncState, file: &CloudWorkspaceFile) -> CloudSyncResult {
    CloudSyncResult {
        state,
        revision: file.revision,
        updated_at: file.updated_at.clone(),
        updated_by: file.updated_by.clone(),
    }
}

fn workspace_contents_equal(left: &Value, right: &Value) -> bool {
    left.get("tables") == right.get("tables") && left.get("localNotes") == right.get("localNotes")
}

fn validate_credential_password(password: &str) -> Result<(), String> {
    let length = password.chars().count();
    if !(8..=12).contains(&length) || password.chars().any(char::is_control) {
        return Err("Encryption password must be 8–12 characters.".into());
    }
    Ok(())
}

fn derive_credential_key(password: &str, salt: &[u8], iterations: u32) -> Result<[u8; 32], String> {
    let iterations = NonZeroU32::new(iterations)
        .ok_or_else(|| "Encrypted credential KDF is invalid.".to_string())?;
    let mut key = [0u8; 32];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        iterations,
        salt,
        password.as_bytes(),
        &mut key,
    );
    Ok(key)
}

fn encrypt_credentials(payload: &str, password: &str) -> Result<Vec<u8>, String> {
    validate_credential_password(password)?;
    let mut salt = [0u8; 16];
    let mut nonce_bytes = [0u8; 12];
    rand::rng().fill_bytes(&mut salt);
    rand::rng().fill_bytes(&mut nonce_bytes);
    let key = derive_credential_key(password, &salt, CREDENTIAL_KDF_ITERATIONS)?;
    let key = aead::UnboundKey::new(&aead::AES_256_GCM, &key)
        .map_err(|_| "Credential encryption setup failed.".to_string())?;
    let key = aead::LessSafeKey::new(key);
    let mut ciphertext = payload.as_bytes().to_vec();
    key.seal_in_place_append_tag(
        aead::Nonce::assume_unique_for_key(nonce_bytes),
        aead::Aad::from(CREDENTIAL_AAD),
        &mut ciphertext,
    )
    .map_err(|_| "Credential encryption failed.".to_string())?;
    let encode = base64::engine::general_purpose::STANDARD;
    serde_json::to_vec_pretty(&EncryptedCredentialFile {
        format: CREDENTIAL_FORMAT.into(),
        format_version: 1,
        kdf: "pbkdf2-sha256".into(),
        iterations: CREDENTIAL_KDF_ITERATIONS,
        salt: encode.encode(salt),
        cipher: "aes-256-gcm".into(),
        nonce: encode.encode(nonce_bytes),
        ciphertext: encode.encode(ciphertext),
    })
    .map_err(|_| "Encrypted credentials could not be encoded.".to_string())
}

fn decrypt_credentials(bytes: &[u8], password: &str) -> Result<String, String> {
    validate_credential_password(password)?;
    let file: EncryptedCredentialFile = serde_json::from_slice(bytes)
        .map_err(|_| "Encrypted credential file is invalid.".to_string())?;
    if file.format != CREDENTIAL_FORMAT
        || file.format_version != 1
        || file.kdf != "pbkdf2-sha256"
        || file.iterations != CREDENTIAL_KDF_ITERATIONS
        || file.cipher != "aes-256-gcm"
    {
        return Err("Encrypted credential file version is unsupported.".into());
    }
    let decode = base64::engine::general_purpose::STANDARD;
    let salt = decode
        .decode(file.salt)
        .map_err(|_| "Encrypted credential salt is invalid.".to_string())?;
    if salt.len() != 16 {
        return Err("Encrypted credential salt is invalid.".into());
    }
    let nonce: [u8; 12] = decode
        .decode(file.nonce)
        .map_err(|_| "Encrypted credential nonce is invalid.".to_string())?
        .try_into()
        .map_err(|_| "Encrypted credential nonce is invalid.".to_string())?;
    let mut ciphertext = decode
        .decode(file.ciphertext)
        .map_err(|_| "Encrypted credential payload is invalid.".to_string())?;
    if !(16..=(64 * 1024 + 16)).contains(&ciphertext.len()) {
        return Err("Encrypted credential payload is invalid.".into());
    }
    let key = derive_credential_key(password, &salt, file.iterations)?;
    let key = aead::UnboundKey::new(&aead::AES_256_GCM, &key)
        .map_err(|_| "Credential decryption setup failed.".to_string())?;
    let key = aead::LessSafeKey::new(key);
    let plaintext = key
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(CREDENTIAL_AAD),
            &mut ciphertext,
        )
        .map_err(|_| "Credential decryption failed. Check the encryption password.".to_string())?;
    String::from_utf8(plaintext.to_vec())
        .map_err(|_| "Decrypted credential payload is invalid.".to_string())
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
    #[cfg(desktop)]
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
    #[cfg(desktop)]
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
    #[cfg(desktop)]
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
        self.vault.store(
            provider_account(provider),
            &serde_json::to_string(&stored).map_err(|_| "Token serialization failed.")?,
        )?;
        for other in providers().into_iter().filter(|other| *other != provider) {
            if let Err(error) = self.vault.delete(provider_account(other)) {
                let _ = self.vault.delete(provider_account(provider));
                return Err(error);
            }
        }
        Ok(true)
    }

    pub fn authorized_providers(&self) -> Result<Vec<CloudProvider>, String> {
        let mut authorized = Vec::new();
        for provider in providers() {
            let Some(raw) = self.vault.load(provider_account(provider))? else {
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

    /// Saves credentials issued by a provider's native mobile SDK.
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    #[cfg(target_os = "ios")]
    pub fn connect_with_native_token(
        &self,
        provider: CloudProvider,
        client_id: &str,
        access_token: String,
        refresh_token: Option<String>,
        expires_in: u64,
    ) -> Result<bool, String> {
        if provider != CloudProvider::GoogleDrive
            || !valid_client_id(client_id)
            || access_token.is_empty()
        {
            return Err("Native cloud authorization is invalid.".into());
        }
        let stored = StoredToken {
            access_token,
            refresh_token: refresh_token.filter(|value| !value.is_empty()),
            expires_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                + bounded_expiry(Some(expires_in)).saturating_sub(60),
            client_id: client_id.to_string(),
        };
        self.vault.store(
            provider_account(provider),
            &serde_json::to_string(&stored).map_err(|_| "Token serialization failed.")?,
        )?;
        for other in providers().into_iter().filter(|other| *other != provider) {
            if let Err(error) = self.vault.delete(provider_account(other)) {
                let _ = self.vault.delete(provider_account(provider));
                return Err(error);
            }
        }
        Ok(true)
    }

    pub fn disconnect(&self, provider: CloudProvider) -> Result<(), String> {
        self.vault.delete(provider_account(provider))
    }

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    pub async fn connect_icloud(&self) -> Result<bool, String> {
        crate::icloud::account_available().await?;
        self.vault
            .store(provider_account(CloudProvider::Icloud), "connected")?;
        for other in providers()
            .into_iter()
            .filter(|provider| *provider != CloudProvider::Icloud)
        {
            if let Err(error) = self.vault.delete(provider_account(other)) {
                let _ = self.vault.delete(provider_account(CloudProvider::Icloud));
                return Err(error);
            }
        }
        Ok(true)
    }

    async fn access_token(&self, provider: CloudProvider) -> Result<String, String> {
        if provider == CloudProvider::Icloud {
            return Err("iCloud does not use OAuth tokens.".into());
        }
        let raw = self
            .vault
            .load(provider_account(provider))?
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
                    #[cfg(desktop)]
                    if let Some(secret) = client_secret_for(provider, &token.client_id) {
                        form.push(("client_secret", secret));
                    }
                    form
                })
                .send()
                .await
                .map_err(|_| "Cloud token refresh failed.".to_string())?;
            if response.status().is_redirection() || !response.status().is_success() {
                return Err(rejection("Cloud token refresh was rejected.", response).await);
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
            self.vault.store(
                provider_account(provider),
                &serde_json::to_string(&token).map_err(|_| "Token serialization failed.")?,
            )?;
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
        let mut request = self
            .client
            .request(method, url)
            .bearer_auth(self.access_token(provider).await?);
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
        let mut url = Url::parse("https://www.googleapis.com/drive/v3/files")
            .map_err(|_| "Google Drive endpoint is invalid.".to_string())?;
        url.query_pairs_mut()
            .append_pair("spaces", "appDataFolder")
            .append_pair("q", &format!("name = '{}' and trashed = false", name))
            .append_pair("fields", "files(id)")
            .append_pair("pageSize", "2");
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
        Ok(files
            .files
            .and_then(|files| files.into_iter().next())
            .map(|file| file.id))
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
                #[cfg(any(target_os = "macos", target_os = "ios"))]
                return crate::icloud::read(name).await;
                #[cfg(not(any(target_os = "macos", target_os = "ios")))]
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
                #[cfg(any(target_os = "macos", target_os = "ios"))]
                {
                    return crate::icloud::write(name, bytes).await;
                }
                #[cfg(not(any(target_os = "macos", target_os = "ios")))]
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

    async fn read_workspace(
        &self,
        provider: CloudProvider,
    ) -> Result<Option<CloudWorkspaceFile>, String> {
        self.read_file(provider, CLOUD_FILE_NAME)
            .await?
            .map(|bytes| parse_cloud_workspace(&bytes))
            .transpose()
    }

    pub async fn back_up_workspace(
        &self,
        provider: CloudProvider,
        document: &str,
        updated_by: &str,
        overwrite: bool,
    ) -> Result<CloudSyncResult, String> {
        let document: Value = serde_json::from_str(document)
            .map_err(|_| "Local workspace is not valid JSON.".to_string())?;
        validate_workspace_document(&document)?;
        if updated_by.trim().is_empty() {
            return Err("Local device identifier is unavailable.".into());
        }
        let remote = self.read_workspace(provider).await?;
        if let Some(remote) = &remote {
            if workspace_contents_equal(&remote.document, &document) {
                return Ok(sync_result(CloudSyncState::UpToDate, remote));
            }
            if !overwrite {
                return Ok(sync_result(CloudSyncState::Conflict, remote));
            }
        }
        let file = CloudWorkspaceFile {
            format: CLOUD_FORMAT.into(),
            format_version: CLOUD_FORMAT_VERSION,
            revision: remote.as_ref().map_or(1, |file| file.revision + 1),
            updated_at: chrono::Utc::now().to_rfc3339(),
            updated_by: updated_by.to_string(),
            document,
        };
        let bytes = serde_json::to_vec_pretty(&file)
            .map_err(|_| "Cloud workspace serialization failed.".to_string())?;
        self.write_file(provider, CLOUD_FILE_NAME, bytes).await?;
        Ok(sync_result(CloudSyncState::BackedUp, &file))
    }

    pub async fn pull_workspace(
        &self,
        provider: CloudProvider,
    ) -> Result<(String, CloudSyncResult), String> {
        let file = self
            .read_workspace(provider)
            .await?
            .ok_or_else(|| "No cloud workspace exists yet.".to_string())?;
        let document = serde_json::to_string(&file.document)
            .map_err(|_| "Cloud workspace serialization failed.".to_string())?;
        Ok((document, sync_result(CloudSyncState::Restored, &file)))
    }

    pub fn credential_sync_configured(&self) -> Result<bool, String> {
        Ok(self.vault.load(CREDENTIAL_PASSWORD_ACCOUNT)?.is_some())
    }

    pub fn set_credential_sync_password(&self, password: &str) -> Result<(), String> {
        validate_credential_password(password)?;
        self.vault.store(CREDENTIAL_PASSWORD_ACCOUNT, password)
    }

    pub fn clear_credential_sync_password(&self) -> Result<(), String> {
        self.vault.delete(CREDENTIAL_PASSWORD_ACCOUNT)
    }

    pub async fn credential_backup_exists(
        &self,
        provider: CloudProvider,
    ) -> Result<bool, String> {
        if provider == CloudProvider::Icloud {
            return Ok(false);
        }
        Ok(self
            .read_file(provider, CREDENTIAL_FILE_NAME)
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
            .load(CREDENTIAL_PASSWORD_ACCOUNT)?
            .ok_or_else(|| "Credential sync is not configured on this device.".to_string())?;
        let encrypted = encrypt_credentials(payload, &password)?;
        self.write_file(provider, CREDENTIAL_FILE_NAME, encrypted)
            .await
    }

    pub async fn pull_credentials(
        &self,
        provider: CloudProvider,
    ) -> Result<Option<String>, String> {
        let Some(password) = self.vault.load(CREDENTIAL_PASSWORD_ACCOUNT)? else {
            return Ok(None);
        };
        let Some(encrypted) = self.read_file(provider, CREDENTIAL_FILE_NAME).await? else {
            return Ok(None);
        };
        decrypt_credentials(&encrypted, &password).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemoryVault(Mutex<HashMap<String, String>>);

    impl SecretVault for MemoryVault {
        fn store(&self, account: &str, secret: &str) -> Result<(), String> {
            self.0
                .lock()
                .map_err(|_| "lock failed".to_string())?
                .insert(account.to_string(), secret.to_string());
            Ok(())
        }

        fn load(&self, account: &str) -> Result<Option<String>, String> {
            Ok(self
                .0
                .lock()
                .map_err(|_| "lock failed".to_string())?
                .get(account)
                .cloned())
        }

        fn delete(&self, account: &str) -> Result<(), String> {
            self.0
                .lock()
                .map_err(|_| "lock failed".to_string())?
                .remove(account);
            Ok(())
        }
    }

    #[test]
    fn parses_workspace_wrapper_and_compares_only_restorable_content() {
        let document = serde_json::json!({
            "schemaVersion": 6,
            "exportedAt": "2026-01-01T00:00:00Z",
            "kind": "IsctCompanion.local-backup",
            "deviceId": "local-backup",
            "tables": {},
            "localNotes": []
        });
        let bytes = serde_json::to_vec(&serde_json::json!({
            "format": CLOUD_FORMAT,
            "formatVersion": CLOUD_FORMAT_VERSION,
            "revision": 2,
            "updatedAt": "2026-01-01T00:00:00Z",
            "updatedBy": "device-a",
            "document": document
        }))
        .unwrap();
        let parsed = parse_cloud_workspace(&bytes).unwrap();
        let mut later = parsed.document.clone();
        later["exportedAt"] = Value::String("2026-02-01T00:00:00Z".into());
        assert!(workspace_contents_equal(&parsed.document, &later));
    }

    #[test]
    fn credential_encryption_round_trips_and_rejects_wrong_password() {
        let encrypted = encrypt_credentials("{\"version\":1}", "eight888").unwrap();
        assert_eq!(
            decrypt_credentials(&encrypted, "eight888").unwrap(),
            "{\"version\":1}"
        );
        assert!(decrypt_credentials(&encrypted, "wrong999").is_err());
        assert!(encrypt_credentials("{}", "short").is_err());
    }

    #[test]
    fn reports_and_disconnects_saved_authorization() {
        let vault = MemoryVault::default();
        vault
            .store(
                "google-drive",
                &serde_json::to_string(&StoredToken {
                    access_token: "access".into(),
                    refresh_token: Some("refresh".into()),
                    expires_at: 1,
                    client_id: "client.apps.googleusercontent.com".into(),
                })
                .unwrap(),
            )
            .unwrap();
        let service = DesktopCloudService::new(vault).unwrap();

        assert_eq!(
            service.authorized_providers().unwrap(),
            vec![CloudProvider::GoogleDrive]
        );
        service.disconnect(CloudProvider::GoogleDrive).unwrap();
        assert!(service.authorized_providers().unwrap().is_empty());
    }
}
