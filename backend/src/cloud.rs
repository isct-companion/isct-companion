use crate::secrets::SecretVault;
use base64::{engine::general_purpose::STANDARD, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::Url;

const MAX_BODY_BYTES: usize = 25 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CloudProvider {
    GoogleDrive,
    Onedrive,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudRequestInput {
    pub method: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    pub body_text: Option<String>,
    pub body_base64: Option<String>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct ValidatedCloudRequest {
    pub method: String,
    pub url: Url,
    pub content_type: Option<String>,
    pub body_size: usize,
}

fn allowed_endpoint(provider: CloudProvider, url: &Url) -> bool {
    match provider {
        CloudProvider::GoogleDrive => {
            url.host_str() == Some("www.googleapis.com")
                && (url.path().starts_with("/drive/v3/")
                    || url.path().starts_with("/upload/drive/v3/"))
        }
        CloudProvider::Onedrive => {
            url.host_str() == Some("graph.microsoft.com")
                && (url.path() == "/v1.0/me/drive/special/approot"
                    || url.path().starts_with("/v1.0/me/drive/special/approot/")
                    || url.path().starts_with("/v1.0/me/drive/special/approot:"))
        }
    }
}

pub fn validate_cloud_request(
    provider: CloudProvider,
    raw_url: &str,
    input: &CloudRequestInput,
) -> Result<ValidatedCloudRequest, String> {
    let url = Url::parse(raw_url).map_err(|_| "Cloud URL is invalid.".to_string())?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return Err("Cloud URL must be credential-free HTTPS.".into());
    }
    if url.fragment().is_some() {
        return Err("Cloud URL fragments are not allowed.".into());
    }
    let encoded_path = url.path().to_ascii_lowercase();
    if encoded_path.contains("%2e") || encoded_path.contains("%2f") || encoded_path.contains("%5c")
    {
        return Err("Cloud URL contains an encoded path delimiter.".into());
    }
    if !allowed_endpoint(provider, &url) {
        return Err("Cloud endpoint is not allowed for this provider.".into());
    }
    let method = input
        .method
        .as_deref()
        .unwrap_or("GET")
        .to_ascii_uppercase();
    if !matches!(method.as_str(), "GET" | "POST" | "PUT" | "PATCH") {
        return Err("Cloud method is not allowed.".into());
    }
    let mut content_type = None;
    for (name, value) in &input.headers {
        if !name.eq_ignore_ascii_case("content-type") {
            return Err("Cloud request header is not allowed.".into());
        }
        if value.contains('\r') || value.contains('\n') {
            return Err("Cloud content type is invalid.".into());
        }
        content_type = Some(value.clone());
    }
    if input.body_text.is_some() && input.body_base64.is_some() {
        return Err("Cloud request has more than one body.".into());
    }
    let body_size = input
        .body_text
        .as_ref()
        .map(|value| value.len())
        .or_else(|| {
            input
                .body_base64
                .as_ref()
                .map(|value| value.len().saturating_mul(3) / 4)
        })
        .unwrap_or(0);
    if body_size > MAX_BODY_BYTES {
        return Err("Cloud request body is too large.".into());
    }
    Ok(ValidatedCloudRequest {
        method,
        url,
        content_type,
        body_size,
    })
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudResponse {
    pub status: u16,
    pub status_text: String,
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

fn provider_account(provider: CloudProvider) -> &'static str {
    match provider {
        CloudProvider::GoogleDrive => "google-drive",
        CloudProvider::Onedrive => "onedrive",
    }
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
        if !valid_client_id(client_id) {
            return Err("OAuth client ID is invalid.".into());
        }
        let redirect = validate_redirect_uri(redirect_uri)?;
        let verifier = random_urlsafe(48);
        let state = random_urlsafe(32);
        validate_pkce_state(&verifier, &state)?;
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        let (authorize_endpoint, token_endpoint, scope) = oauth_endpoints(provider);
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
        let response = self
            .client
            .post(token_endpoint)
            .form(&[
                ("client_id", client_id),
                ("code", callback.as_str()),
                ("code_verifier", verifier.as_str()),
                ("redirect_uri", redirect.as_str()),
                ("grant_type", "authorization_code"),
            ])
            .send()
            .await
            .map_err(|_| "OAuth token exchange failed.".to_string())?;
        if response.status().is_redirection() || !response.status().is_success() {
            return Err("OAuth token exchange was rejected.".into());
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
        Ok(true)
    }

    async fn access_token(&self, provider: CloudProvider) -> Result<String, String> {
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
                .form(&[
                    ("client_id", token.client_id.as_str()),
                    ("refresh_token", refresh.as_str()),
                    ("grant_type", "refresh_token"),
                ])
                .send()
                .await
                .map_err(|_| "Cloud token refresh failed.".to_string())?;
            if response.status().is_redirection() || !response.status().is_success() {
                return Err("Cloud token refresh was rejected.".into());
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

    pub async fn request(
        &self,
        provider: CloudProvider,
        raw_url: &str,
        input: CloudRequestInput,
    ) -> Result<CloudResponse, String> {
        let validated = validate_cloud_request(provider, raw_url, &input)?;
        let body = match (&input.body_text, &input.body_base64) {
            (Some(text), None) => text.as_bytes().to_vec(),
            (None, Some(encoded)) => STANDARD
                .decode(encoded)
                .map_err(|_| "Cloud request body encoding is invalid.".to_string())?,
            (None, None) => Vec::new(),
            _ => return Err("Cloud request has more than one body.".into()),
        };
        if body.len() > MAX_BODY_BYTES {
            return Err("Cloud request body is too large.".into());
        }
        let method = reqwest::Method::from_bytes(validated.method.as_bytes())
            .map_err(|_| "Cloud method is invalid.".to_string())?;
        let mut request = self
            .client
            .request(method, validated.url)
            .bearer_auth(self.access_token(provider).await?);
        if let Some(content_type) = validated.content_type {
            request = request.header(reqwest::header::CONTENT_TYPE, content_type);
        }
        if !body.is_empty() {
            request = request.body(body);
        }
        let response = request
            .send()
            .await
            .map_err(|_| "Cloud request failed.".to_string())?;
        if response.status().is_redirection() {
            return Err("Cloud redirects are not followed.".into());
        }
        let status = response.status();
        let mut headers = BTreeMap::new();
        for name in [reqwest::header::CONTENT_TYPE, reqwest::header::ETAG] {
            if let Some(value) = response
                .headers()
                .get(&name)
                .and_then(|value| value.to_str().ok())
            {
                headers.insert(name.as_str().to_string(), value.to_string());
            }
        }
        let bytes = read_limited(response, MAX_BODY_BYTES).await?;
        Ok(CloudResponse {
            status: status.as_u16(),
            status_text: status.canonical_reason().unwrap_or("").to_string(),
            headers,
            body: STANDARD.encode(bytes),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> CloudRequestInput {
        CloudRequestInput {
            method: Some("GET".into()),
            headers: BTreeMap::new(),
            body_text: None,
            body_base64: None,
        }
    }

    #[test]
    fn rejects_cross_provider_and_credentials() {
        assert!(validate_cloud_request(
            CloudProvider::GoogleDrive,
            "https://graph.microsoft.com/v1.0/me/drive/special/approot/x",
            &request()
        )
        .is_err());
        assert!(validate_cloud_request(
            CloudProvider::Onedrive,
            "https://token@example.com/v1.0/me/drive/special/approot/x",
            &request()
        )
        .is_err());
        assert!(validate_cloud_request(
            CloudProvider::GoogleDrive,
            "https://www.googleapis.com/drive/v3/%2e%2e/token",
            &request()
        )
        .is_err());
        assert!(validate_cloud_request(
            CloudProvider::GoogleDrive,
            "https://www.googleapis.com/drive/v3/files#access_token=secret",
            &request()
        )
        .is_err());
    }

    #[test]
    fn accepts_scoped_provider_paths() {
        assert!(validate_cloud_request(
            CloudProvider::GoogleDrive,
            "https://www.googleapis.com/drive/v3/files",
            &request()
        )
        .is_ok());
        assert!(validate_cloud_request(
            CloudProvider::Onedrive,
            "https://graph.microsoft.com/v1.0/me/drive/special/approot:/backup.bin:/content",
            &request()
        )
        .is_ok());
    }
}
