use super::*;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
struct MemoryVault(Mutex<HashMap<String, String>>);

impl SecretVault for MemoryVault {
    fn store(&self, account: &str, secret: &str) -> Result<(), SecretError> {
        self.0
            .lock()
            .map_err(|_| SecretError::Access("lock failed".into()))?
            .insert(account.to_string(), secret.to_string());
        Ok(())
    }

    fn load(&self, account: &str) -> Result<Option<String>, SecretError> {
        Ok(self
            .0
            .lock()
            .map_err(|_| SecretError::Access("lock failed".into()))?
            .get(account)
            .cloned())
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        self.0
            .lock()
            .map_err(|_| SecretError::Access("lock failed".into()))?
            .remove(account);
        Ok(())
    }
}

#[test]
fn disconnect_only_removes_its_own_authorization() {
    let vault = MemoryVault::default();
    for provider in providers() {
        vault
            .store(provider_account(provider), "connected")
            .unwrap();
    }
    let service = DesktopCloudService::new(vault).unwrap();
    service.disconnect(CloudProvider::Icloud).unwrap();
    assert_eq!(
        service.vault.load("google-drive").unwrap().as_deref(),
        Some("connected")
    );
    service.disconnect(CloudProvider::GoogleDrive).unwrap();
    assert_eq!(
        service.vault.load("onedrive").unwrap().as_deref(),
        Some("connected")
    );
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
