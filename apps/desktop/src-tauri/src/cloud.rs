use app_core::cloud_credentials::CredentialBackup;
use app_core::cloud_sync::document::{self, CloudWorkspaceFile, DocumentError, WorkspaceTransport};
use app_core::cloud_sync::google_drive::{GoogleDrive, Pkce};
pub use app_core::cloud_sync::CloudProvider;
use app_core::cloud_sync::CloudSyncError;
use app_core::oauth::{code_exchange::OAuthClient, NativeBrowserAuth};
use app_core::ports::SecretVault;
use tokio_util::sync::CancellationToken;

/// Google requires the client secret of a Desktop OAuth client in token
/// requests, even with PKCE; Google documents it as not confidential for
/// installed apps. `scripts/desktop/tauri.mjs` supplies both values at build time
/// from config/oauth-clients.yaml, and Core sends the secret only with the
/// client it belongs to (not with a development override).
const GOOGLE_DESKTOP_CLIENT_ID: Option<&str> = option_env!("ISCT_GOOGLE_DESKTOP_CLIENT_ID");
const GOOGLE_DESKTOP_CLIENT_SECRET: Option<&str> = option_env!("ISCT_GOOGLE_DESKTOP_CLIENT_SECRET");

/// Drive's REST calls and token refresh live in Core; the desktop shell runs
/// the system browser and keeps the iCloud store.
pub struct DesktopCloudService<V: SecretVault> {
    drive: GoogleDrive<Pkce<V>>,
    shutdown: CancellationToken,
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
            shutdown: CancellationToken::new(),
            #[cfg(target_os = "macos")]
            icloud: document::TaggedWorkspace::new(crate::icloud::Store),
        })
    }

    fn vault(&self) -> &V {
        self.drive.authorization().vault()
    }

    pub async fn connect(
        &self,
        override_id: Option<&str>,
        browser: &dyn NativeBrowserAuth,
    ) -> Result<(), CloudSyncError> {
        self.drive
            .authorization()
            .connect(override_id, browser, &self.shutdown)
            .await
    }

    pub fn shutdown(&self) {
        self.shutdown.cancel();
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
