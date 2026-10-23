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

/// The vault the shell wrote before Drive moved into Core still reads as
/// connected, and each disconnect removes only its own provider's entry.
#[test]
fn saved_authorizations_are_reported_and_disconnect_only_removes_its_own() {
    let vault = MemoryVault::default();
    vault
        .store(
            "google-drive",
            &serde_json::json!({
                "access_token": "access",
                "refresh_token": "refresh",
                "expires_at": 1,
                "client_id": "client.apps.googleusercontent.com",
            })
            .to_string(),
        )
        .unwrap();
    vault.store("icloud", "connected").unwrap();
    vault.store("onedrive", "connected").unwrap();
    let service = DesktopCloudService::new(vault).unwrap();
    assert_eq!(
        service.authorized_providers().unwrap(),
        vec![CloudProvider::GoogleDrive, CloudProvider::Icloud]
    );

    service.disconnect(CloudProvider::Icloud).unwrap();
    assert_eq!(
        service.authorized_providers().unwrap(),
        vec![CloudProvider::GoogleDrive]
    );
    service.disconnect(CloudProvider::GoogleDrive).unwrap();
    assert!(service.authorized_providers().unwrap().is_empty());
    assert_eq!(
        service.vault().load("onedrive").unwrap().as_deref(),
        Some("connected")
    );
}
