use app_core::cloud_credentials::CredentialBackup;
use app_core::cloud_sync::document::{self, CloudWorkspaceFile, DocumentError, WorkspaceTransport};
use app_core::cloud_sync::google_drive::{GoogleDrive, OAuthClient, Pkce};
pub use app_core::cloud_sync::CloudProvider;
use app_core::ports::SecretVault;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, SystemTime};
use url::Url;

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

/// Google requires the client secret of a Desktop OAuth client in token
/// requests, even with PKCE; Google documents it as not confidential for
/// installed apps. `scripts/desktop/tauri.mjs` supplies both values at build time
/// from config/oauth-clients.yaml, and Core sends the secret only with the
/// client it belongs to (not with a development override).
const GOOGLE_DESKTOP_CLIENT_ID: Option<&str> = option_env!("ISCT_GOOGLE_DESKTOP_CLIENT_ID");
const GOOGLE_DESKTOP_CLIENT_SECRET: Option<&str> = option_env!("ISCT_GOOGLE_DESKTOP_CLIENT_SECRET");

/// Drive's REST calls and token refresh live in Core; the desktop shell runs
/// the loopback sign-in and keeps the iCloud store.
pub struct DesktopCloudService<V: SecretVault> {
    drive: GoogleDrive<Pkce<V>>,
    #[cfg(target_os = "macos")]
    icloud: document::TaggedWorkspace<crate::icloud::Store>,
}

impl<V: SecretVault> DesktopCloudService<V> {
    pub fn new(vault: V) -> Result<Self, String> {
        let client = OAuthClient {
            id: GOOGLE_DESKTOP_CLIENT_ID.unwrap_or_default().into(),
            secret: GOOGLE_DESKTOP_CLIENT_SECRET.map(str::to_owned),
        };
        let pkce = Pkce::new(vault, client).map_err(|error| error.to_string())?;
        Ok(Self {
            drive: GoogleDrive::new(pkce).map_err(|error| error.to_string())?,
            #[cfg(target_os = "macos")]
            icloud: document::TaggedWorkspace::new(crate::icloud::Store),
        })
    }

    fn vault(&self) -> &V {
        self.drive.authorization().vault()
    }

    /// Desktop Drive sign-in: the system browser returns to a loopback listener.
    pub async fn connect<F>(
        &self,
        client_id: &str,
        redirect_uri: &str,
        open_browser: F,
    ) -> Result<(), String>
    where
        F: FnOnce(&str) -> Result<(), String>,
    {
        let redirect = validate_redirect_uri(redirect_uri)?;
        let port = redirect
            .port()
            .ok_or_else(|| "OAuth redirect must be an exact loopback callback.".to_string())?;
        let pkce = self.drive.authorization();
        let pending = pkce
            .begin(client_id, redirect.as_str())
            .map_err(|error| error.to_string())?;
        let listener = TcpListener::bind(("127.0.0.1", port))
            .map_err(|_| "OAuth loopback port is unavailable.".to_string())?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "OAuth callback listener failed.".to_string())?;
        open_browser(&pending.url)?;
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
                        let response = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\nAuthorization completed. You may close this window now.";
                        let _ = stream.write_all(response);
                        // Core checks the path, state and code of this redirect.
                        return Ok(format!("http://127.0.0.1:{port}{target}"));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if SystemTime::now() >= deadline { return Err("OAuth authorization timed out.".into()); }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    Err(_) => return Err("OAuth callback listener failed.".into()),
                }
            }
        }).await.map_err(|_| "OAuth callback task failed.".to_string())??;
        pkce.finish(client_id, &pending, &callback)
            .await
            .map_err(|error| error.to_string())
    }

    pub fn drive_signed_in(&self) -> Result<bool, DocumentError> {
        self.drive.authorization().authorized()
    }

    pub fn authorized_providers(&self) -> Result<Vec<CloudProvider>, String> {
        Ok(
            if self.drive_signed_in().map_err(|error| error.to_string())? {
                vec![CloudProvider::GoogleDrive]
            } else {
                Vec::new()
            },
        )
    }

    pub fn credential_backup(&self) -> CredentialBackup<'_, V, GoogleDrive<Pkce<V>>> {
        CredentialBackup::new(self.vault(), &self.drive)
    }
}

impl<V: SecretVault> WorkspaceTransport for DesktopCloudService<V> {
    fn read<'a>(
        &'a self,
        provider: CloudProvider,
    ) -> document::BoxFuture<'a, Option<CloudWorkspaceFile>> {
        #[cfg(target_os = "macos")]
        if provider == CloudProvider::Icloud {
            return self.icloud.read(provider);
        }
        self.drive.read(provider)
    }

    fn write<'a>(
        &'a self,
        provider: CloudProvider,
        file: &'a CloudWorkspaceFile,
        expected_revision: Option<u64>,
    ) -> document::BoxFuture<'a, ()> {
        #[cfg(target_os = "macos")]
        if provider == CloudProvider::Icloud {
            return self.icloud.write(provider, file, expected_revision);
        }
        self.drive.write(provider, file, expected_revision)
    }

    fn disconnect(&self, provider: CloudProvider) -> Result<(), DocumentError> {
        #[cfg(target_os = "macos")]
        if provider == CloudProvider::Icloud {
            return self.icloud.disconnect(provider);
        }
        self.drive.disconnect(provider)
    }
}
