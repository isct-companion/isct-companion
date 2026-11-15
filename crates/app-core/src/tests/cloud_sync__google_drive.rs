use super::*;
use crate::ports::SecretError;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
struct MemoryVault(Mutex<HashMap<String, String>>);

impl SecretVault for MemoryVault {
    fn store(&self, account: &str, secret: &str) -> Result<(), SecretError> {
        self.0.lock().unwrap().insert(account.into(), secret.into());
        Ok(())
    }

    fn load(&self, account: &str) -> Result<Option<String>, SecretError> {
        Ok(self.0.lock().unwrap().get(account).cloned())
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        self.0.lock().unwrap().remove(account);
        Ok(())
    }
}

fn desktop() -> Pkce<MemoryVault> {
    Pkce::new(
        MemoryVault::default(),
        OAuthClient {
            id: "desktop.apps.googleusercontent.com".into(),
            secret: Some("desktop-secret".into()),
        },
    )
    .unwrap()
}

/// A desktop's saved Drive authorization (the vault format the shell wrote
/// before Drive moved into Core) is still recognized, refreshed only when
/// expired, and disconnecting removes only the Drive account.
#[tokio::test]
async fn saved_authorization_is_read_and_disconnect_removes_only_drive() {
    let pkce = desktop();
    assert!(!pkce.authorized().unwrap());
    pkce.vault()
        .store(
            VAULT_ACCOUNT,
            &serde_json::json!({
                "access_token": "access",
                "refresh_token": "refresh",
                "expires_at": now() + 600,
                "client_id": "desktop.apps.googleusercontent.com",
            })
            .to_string(),
        )
        .unwrap();
    pkce.vault().store("icloud", "connected").unwrap();
    assert!(pkce.authorized().unwrap());
    assert_eq!(pkce.access_token(false).await.unwrap(), "access");

    DriveAuthorization::disconnect(&pkce).unwrap();
    assert!(!pkce.authorized().unwrap());
    assert_eq!(
        pkce.vault().load("icloud").unwrap().as_deref(),
        Some("connected")
    );
    assert!(pkce.access_token(false).await.is_err());
}

/// A shell with several stores routes each provider to its own; a provider
/// it has no store for is an error, not another provider's data.
#[tokio::test]
async fn providers_route_each_provider_to_its_own_store() {
    struct Named(&'static str);
    impl WorkspaceTransport for Named {
        fn read<'a>(&'a self, _: CloudProvider) -> BoxFuture<'a, Option<CloudWorkspaceFile>> {
            Box::pin(async move { Err(DocumentError::Transport(self.0.into())) })
        }
        fn write<'a>(
            &'a self,
            _: CloudProvider,
            _: &'a CloudWorkspaceFile,
            _: Option<u64>,
        ) -> BoxFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }
        fn disconnect(&self, _: CloudProvider) -> Result<(), DocumentError> {
            Ok(())
        }
    }
    let routed = document::Providers::default()
        .with(CloudProvider::Icloud, Named("icloud"))
        .with(CloudProvider::GoogleDrive, Named("drive"));
    for (provider, name) in [
        (CloudProvider::Icloud, "icloud"),
        (CloudProvider::GoogleDrive, "drive"),
    ] {
        assert_eq!(
            routed.read(provider).await.unwrap_err(),
            DocumentError::Transport(name.into())
        );
    }
    // A provider with no transport routed refuses rather than doing nothing.
    assert!(document::Providers::default()
        .with(CloudProvider::Icloud, Named("icloud"))
        .disconnect(CloudProvider::GoogleDrive)
        .is_err());
}
