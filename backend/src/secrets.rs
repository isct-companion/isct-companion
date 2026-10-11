use tauri::AppHandle;
use tauri_plugin_secure_storage::{OptionsRequest, SecureStorageExt};

const LEGACY_SERVICES: &[&str] = &["app.isctcompanion.legacy", "jp.ac.isct.workstation.cloud"];
const PREVIOUS_APP_ID: &str = "app.campuslife.sciencetokyo";
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
const PREVIOUS_PRODUCT_NAME: &str = "Campus Life";
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
    secure_key_for(APP_ID, account)
}

fn secure_key_for(app_id: &str, account: &str) -> Result<String, String> {
    let suffix = match account {
        "school-accounts" => "passwords.school-accounts",
        "school-science" => "passwords.isct-portal",
        "school-tokyo-tech" => "passwords.titech-portal",
        "google-drive" => "passwords.google-drive",
        "onedrive" => "passwords.onedrive",
        "cloud-credential-sync" => "passwords.cloud-credential-sync",
        _ => return Err("Secret account is not allowed.".into()),
    };
    Ok(format!("{app_id}.{suffix}"))
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
        let stored = self.load_current(account)?;
        if stored.is_some() {
            return Ok(stored);
        }
        if let Some(secret) = self.load_previous(account)? {
            migrate_secret(self, account, &secret)?;
            // A failed cleanup is harmless: the new entry takes precedence.
            let _ = self.delete_previous(account);
            return Ok(Some(secret));
        }
        self.load_legacy(account)
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        let removed = self
            .app
            .secure_storage()
            .remove_item(self.app.clone(), request(account, None)?);
        self.delete_previous(account)?;
        self.delete_legacy(account)?;
        if removed.is_ok() {
            return Ok(());
        }
        let still_present = self.load_current(account)?.is_some();
        if still_present {
            Err("OS credential vault delete failed.".into())
        } else {
            Ok(())
        }
    }
}

// Read back through the destination, never through load()'s migration fallback.
// Keep the source untouched if writing or verification fails.
fn migrate_secret(vault: &OsSecretVault, account: &str, secret: &str) -> Result<(), String> {
    copy_verified(
        secret,
        || vault.store(account, secret),
        || vault.load_current(account),
    )
}

fn copy_verified(
    secret: &str,
    write: impl FnOnce() -> Result<(), String>,
    read: impl FnOnce() -> Result<Option<String>, String>,
) -> Result<(), String> {
    write()?;
    if read()?.as_deref() != Some(secret) {
        return Err("Credential migration could not be verified.".into());
    }
    Ok(())
}

impl OsSecretVault {
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn load_current(&self, account: &str) -> Result<Option<String>, String> {
        // The plugin turns every read error into None on desktop. Query its
        // exact service/key directly so denied access cannot trigger migration
        // over an existing destination.
        let service = self
            .app
            .config()
            .product_name
            .as_deref()
            .ok_or("OS credential vault is unavailable.")?;
        let entry = keyring::Entry::new(service, &secure_key(account)?)
            .map_err(|_| "OS credential vault is unavailable.".to_string())?;
        read_entry(&entry)
    }

    #[cfg(any(target_os = "ios", target_os = "android"))]
    fn load_current(&self, account: &str) -> Result<Option<String>, String> {
        self.app
            .secure_storage()
            .get_item(self.app.clone(), request(account, None)?)
            .map(|response| response.data)
            .map_err(|_| "OS credential vault read failed.".into())
    }

    // Desktop secure-storage uses productName as the service, independently of
    // the key prefix. Changing just the prefix cannot find the old item.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn previous_entry(&self, account: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(
            PREVIOUS_PRODUCT_NAME,
            &secure_key_for(PREVIOUS_APP_ID, account)?,
        )
        .map_err(|_| "Previous credential vault is unavailable.".into())
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn load_previous(&self, account: &str) -> Result<Option<String>, String> {
        read_entry(&self.previous_entry(account)?)
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn delete_previous(&self, account: &str) -> Result<(), String> {
        delete_entry(&self.previous_entry(account)?)
    }

    #[cfg(any(target_os = "ios", target_os = "android"))]
    fn previous_request(&self, account: &str) -> Result<OptionsRequest, String> {
        let mut options = request(account, None)?;
        options.prefixed_key = Some(secure_key_for(PREVIOUS_APP_ID, account)?);
        Ok(options)
    }

    #[cfg(any(target_os = "ios", target_os = "android"))]
    fn load_previous(&self, account: &str) -> Result<Option<String>, String> {
        self.app
            .secure_storage()
            .get_item(self.app.clone(), self.previous_request(account)?)
            .map(|response| response.data)
            .map_err(|_| "Previous credential vault read failed.".into())
    }

    #[cfg(any(target_os = "ios", target_os = "android"))]
    fn delete_previous(&self, account: &str) -> Result<(), String> {
        if self.load_previous(account)?.is_some() {
            self.app
                .secure_storage()
                .remove_item(self.app.clone(), self.previous_request(account)?)
                .map_err(|_| "Previous credential vault delete failed.".to_string())?;
        }
        Ok(())
    }

    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "windows"))]
    fn load_legacy(&self, account: &str) -> Result<Option<String>, String> {
        for service in LEGACY_SERVICES {
            let entry = keyring::Entry::new(service, account)
                .map_err(|_| "Legacy credential vault is unavailable.".to_string())?;
            if let Some(secret) = read_entry(&entry)? {
                migrate_secret(self, account, &secret)?;
                let _ = delete_entry(&entry);
                return Ok(Some(secret));
            }
        }
        Ok(None)
    }

    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "windows")))]
    fn load_legacy(&self, _account: &str) -> Result<Option<String>, String> {
        Ok(None)
    }

    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "windows"))]
    fn delete_legacy(&self, account: &str) -> Result<(), String> {
        for service in LEGACY_SERVICES {
            let entry = keyring::Entry::new(service, account)
                .map_err(|_| "Legacy credential vault is unavailable.".to_string())?;
            delete_entry(&entry)?;
        }
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "windows")))]
    fn delete_legacy(&self, _account: &str) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "windows",
    target_os = "linux"
))]
fn read_entry(entry: &keyring::Entry) -> Result<Option<String>, String> {
    match entry.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        // Cancellation/denied access must not be mistaken for missing passwords.
        Err(_) => Err("Previous credential vault read failed.".into()),
    }
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "windows",
    target_os = "linux"
))]
fn delete_entry(entry: &keyring::Entry) -> Result<(), String> {
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err("Previous credential vault delete failed.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::{copy_verified, secure_key, secure_key_for, PREVIOUS_APP_ID};

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
    fn previous_bundle_and_individual_keys_are_supported() {
        for (account, suffix) in [
            ("school-accounts", "school-accounts"),
            ("school-science", "isct-portal"),
            ("school-tokyo-tech", "titech-portal"),
            ("google-drive", "google-drive"),
            ("onedrive", "onedrive"),
        ] {
            assert_eq!(
                secure_key_for(PREVIOUS_APP_ID, account).unwrap(),
                format!("app.campuslife.sciencetokyo.passwords.{suffix}")
            );
        }
        assert!(secure_key_for(PREVIOUS_APP_ID, "untrusted-account").is_err());
    }

    #[test]
    fn migration_requires_a_verified_destination() {
        assert!(copy_verified("secret", || Ok(()), || Ok(Some("secret".into()))).is_ok());
        assert!(copy_verified("secret", || Ok(()), || Ok(None)).is_err());
        assert!(copy_verified("secret", || Ok(()), || Ok(Some("different".into()))).is_err());
        assert!(copy_verified("secret", || Ok(()), || Err("denied".into())).is_err());
        assert!(copy_verified(
            "secret",
            || Err("write failed".into()),
            || panic!("must not read after failed write")
        )
        .is_err());
    }

    #[test]
    fn arbitrary_secret_accounts_are_rejected() {
        assert!(secure_key("untrusted-account").is_err());
    }
}
