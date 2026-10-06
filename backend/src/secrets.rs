use tauri::AppHandle;
use tauri_plugin_secure_storage::{OptionsRequest, SecureStorageExt};

const LEGACY_SERVICE: &str = "app.isctcompanion.legacy";
const APP_ID: &str = "app.isctcompanion";

pub trait SecretVault: Send + Sync {
    fn store(&self, account: &str, secret: &str) -> Result<(), String>;
    fn load(&self, account: &str) -> Result<Option<String>, String>;
    fn delete(&self, account: &str) -> Result<(), String>;
}

#[derive(Clone)]
pub struct OsSecretVault {
    app: AppHandle,
}

impl OsSecretVault {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

fn secure_key(account: &str) -> Result<String, String> {
    let suffix = match account {
        "school-accounts" => "passwords.school-accounts",
        "school-science" => "passwords.isct-portal",
        "school-tokyo-tech" => "passwords.titech-portal",
        "google-drive" => "passwords.google-drive",
        "onedrive" => "passwords.onedrive",
        _ => return Err("Secret account is not allowed.".into()),
    };
    Ok(format!("{APP_ID}.{suffix}"))
}

fn request(account: &str, data: Option<String>) -> Result<OptionsRequest, String> {
    Ok(OptionsRequest {
        prefixed_key: Some(secure_key(account)?),
        data,
        // Credentials remain local to this device. Cloud sync has its own flow.
        sync: Some(false),
        keychain_access: None,
    })
}

impl SecretVault for OsSecretVault {
    fn store(&self, account: &str, secret: &str) -> Result<(), String> {
        if secret.is_empty() || secret.len() > 64 * 1024 {
            return Err("Secret payload is invalid.".into());
        }
        self.app
            .secure_storage()
            .set_item(self.app.clone(), request(account, Some(secret.into()))?)
            .map(|_| ())
            .map_err(|_| "OS credential vault write failed.".to_string())
    }

    fn load(&self, account: &str) -> Result<Option<String>, String> {
        let stored = self
            .app
            .secure_storage()
            .get_item(self.app.clone(), request(account, None)?)
            .map_err(|_| "OS credential vault read failed.".to_string())?
            .data;
        if stored.is_some() {
            return Ok(stored);
        }
        if account == "school-accounts" {
            return Ok(None);
        }
        self.load_legacy(account)
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        let removed = self
            .app
            .secure_storage()
            .remove_item(self.app.clone(), request(account, None)?);
        self.delete_legacy(account);
        if removed.is_ok() {
            return Ok(());
        }
        let still_present = self
            .app
            .secure_storage()
            .get_item(self.app.clone(), request(account, None)?)
            .ok()
            .and_then(|response| response.data)
            .is_some();
        if still_present {
            Err("OS credential vault delete failed.".into())
        } else {
            Ok(())
        }
    }
}

impl OsSecretVault {
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "windows"))]
    fn load_legacy(&self, account: &str) -> Result<Option<String>, String> {
        let entry = keyring::Entry::new(LEGACY_SERVICE, account)
            .map_err(|_| "Legacy credential vault is unavailable.".to_string())?;
        match entry.get_password() {
            Ok(secret) => {
                self.store(account, &secret)?;
                let _ = entry.delete_credential();
                Ok(Some(secret))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err("Legacy credential vault read failed.".into()),
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "windows")))]
    fn load_legacy(&self, _account: &str) -> Result<Option<String>, String> {
        Ok(None)
    }

    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "windows"))]
    fn delete_legacy(&self, account: &str) {
        if let Ok(entry) = keyring::Entry::new(LEGACY_SERVICE, account) {
            let _ = entry.delete_credential();
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "windows")))]
    fn delete_legacy(&self, _account: &str) {}
}

#[cfg(test)]
mod tests {
    use super::secure_key;

    #[test]
    fn school_accounts_use_stable_portal_keychain_ids() {
        assert_eq!(
            secure_key("school-accounts").unwrap(),
            "app.isctcompanion.passwords.school-accounts"
        );
        assert_eq!(
            secure_key("school-science").unwrap(),
            "app.isctcompanion.passwords.isct-portal"
        );
        assert_eq!(
            secure_key("school-tokyo-tech").unwrap(),
            "app.isctcompanion.passwords.titech-portal"
        );
    }

    #[test]
    fn arbitrary_secret_accounts_are_rejected() {
        assert!(secure_key("untrusted-account").is_err());
    }
}
