use app_core::cloud_credentials::CredentialBackup;
use app_core::cloud_sync::document::{self, CloudWorkspaceFile, DocumentError, WorkspaceTransport};
use app_core::cloud_sync::google_drive::{GoogleDrive, Pkce};
pub(crate) use app_core::cloud_sync::CloudProvider;
use app_core::cloud_sync::CloudSyncError;
use app_core::oauth::{
    code_exchange::{self, OAuthClient},
    NativeBrowserAuth,
};
use app_core::ports::SecretVault;
use tokio_util::sync::CancellationToken;

/// Drive's REST calls and token refresh live in Core; the desktop shell runs
/// the system browser and keeps the iCloud store.
pub(crate) struct DesktopCloudService<V: SecretVault> {
    drive: GoogleDrive<Pkce<V>>,
    shutdown: CancellationToken,
    #[cfg(target_os = "macos")]
    icloud: document::TaggedWorkspace<crate::icloud::Store>,
}

impl<V: SecretVault> DesktopCloudService<V> {
    pub(crate) fn new(vault: V) -> Result<Self, String> {
        // A build without the client still opens; Drive then fails to sign in.
        let client = code_exchange::installed_client().unwrap_or(OAuthClient {
            id: String::new(),
            secret: None,
        });
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

    pub(crate) async fn connect(
        &self,
        override_id: Option<&str>,
        browser: &dyn NativeBrowserAuth,
    ) -> Result<(), CloudSyncError> {
        self.drive
            .authorization()
            .connect(override_id, browser, &self.shutdown)
            .await
    }

    pub(crate) fn shutdown(&self) {
        self.shutdown.cancel();
    }

    pub(crate) fn drive_signed_in(&self) -> Result<bool, DocumentError> {
        self.drive.authorization().authorized()
    }

    pub(crate) fn authorized_providers(&self) -> Result<Vec<CloudProvider>, String> {
        Ok(
            if self.drive_signed_in().map_err(|error| error.to_string())? {
                vec![CloudProvider::GoogleDrive]
            } else {
                Vec::new()
            },
        )
    }

    pub(crate) fn credential_backup(&self) -> CredentialBackup<'_, V, GoogleDrive<Pkce<V>>> {
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
