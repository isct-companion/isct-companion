use app_core::ports::{vault_item, vault_key, SecretError, SecretVault, VAULT_SERVICE};
use tauri::AppHandle;
use tauri_plugin_secure_storage::{OptionsRequest, SecureStorageExt};

const LEGACY_SERVICES: &[&str] = &["app.isctcompanion.legacy", "jp.ac.isct.workstation.cloud"];
const PREVIOUS_APP_ID: &str = "app.campuslife.sciencetokyo";
const PREVIOUS_PRODUCT_NAME: &str = "Campus Life";
const APP_ID: &str = VAULT_SERVICE;

fn access(message: impl Into<String>) -> SecretError {
    SecretError::Access(message.into())
}

fn validate_secret(secret: &str) -> Result<(), SecretError> {
    if secret.is_empty() || secret.len() > 64 * 1024 {
        return Err(SecretError::InvalidPayload);
    }
    Ok(())
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

fn secure_key(account: &str) -> Result<String, SecretError> {
    vault_item(account).map(|item| item.key)
}

fn secure_key_for(app_id: &str, account: &str) -> Result<String, SecretError> {
    vault_key(app_id, account)
}

fn request(account: &str, data: Option<String>) -> Result<OptionsRequest, SecretError> {
    Ok(OptionsRequest {
        prefixed_key: Some(secure_key(account)?),
        data,
        // OAuth tokens and the optional Drive credential-sync password stay
        // device-local. School logins use the native synchronized path below.
        sync: Some(apple_keychain_sync(account)),
        keychain_access: None,
    })
}

// macOS debug builds (`tauri dev`) are unsigned, so they lack the
// keychain-access-groups entitlement the data-protection keychain requires
// (errSecMissingEntitlement, -34018). They keep school logins in the login
// keychain instead; only release builds use the synchronized keychain.
fn apple_keychain_sync(account: &str) -> bool {
    cfg!(all(target_os = "macos", not(debug_assertions)))
        && vault_item(account).is_ok_and(|item| item.synchronized)
}

impl SecretVault for OsSecretVault {
    fn store(&self, account: &str, secret: &str) -> Result<(), SecretError> {
        validate_secret(secret)?;
        #[cfg(target_os = "macos")]
        if apple_keychain_sync(account) {
            return set_apple_synced(account, secret);
        }
        self.app
            .secure_storage()
            .set_item(self.app.clone(), request(account, Some(secret.into()))?)
            .map(|_| ())
            .map_err(|_| access("OS credential vault write failed."))
    }

    fn shared(&self, account: &str) -> bool {
        apple_keychain_sync(account)
    }

    fn load(&self, account: &str) -> Result<Option<String>, SecretError> {
        let stored = self.load_current(account)?;
        if stored.is_some() {
            return Ok(stored);
        }
        #[cfg(target_os = "macos")]
        if apple_keychain_sync(account) {
            if let Some(secret) = self.load_local_current(account)? {
                migrate_secret(self, account, &secret)?;
                // Keep the source if cleanup is denied; the synchronized item
                // takes precedence from now on.
                let _ = self.delete_local_current(account);
                return Ok(Some(secret));
            }
        }
        if let Some(secret) = self.load_previous(account)? {
            migrate_secret(self, account, &secret)?;
            // A failed cleanup is harmless: the new entry takes precedence.
            let _ = self.delete_previous(account);
            return Ok(Some(secret));
        }
        self.load_legacy(account)
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        #[cfg(target_os = "macos")]
        if apple_keychain_sync(account) {
            delete_apple_synced(account)?;
            self.delete_local_current(account)?;
            self.delete_previous(account)?;
            self.delete_legacy(account)?;
            return Ok(());
        }
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
            Err(access("OS credential vault delete failed."))
        } else {
            Ok(())
        }
    }
}

// Read back through the destination, never through load()'s migration fallback.
// Keep the source untouched if writing or verification fails.
fn migrate_secret(vault: &OsSecretVault, account: &str, secret: &str) -> Result<(), SecretError> {
    copy_verified(
        secret,
        || vault.store(account, secret),
        || vault.load_current(account),
    )
}

fn copy_verified(
    secret: &str,
    write: impl FnOnce() -> Result<(), SecretError>,
    read: impl FnOnce() -> Result<Option<String>, SecretError>,
) -> Result<(), SecretError> {
    write()?;
    if read()?.as_deref() != Some(secret) {
        return Err(access("Credential migration could not be verified."));
    }
    Ok(())
}

impl OsSecretVault {
    #[cfg(target_os = "macos")]
    fn load_current(&self, account: &str) -> Result<Option<String>, SecretError> {
        if apple_keychain_sync(account) {
            return get_apple_synced(account);
        }
        self.load_local_current(account)
    }

    #[cfg(target_os = "macos")]
    fn load_local_current(&self, account: &str) -> Result<Option<String>, SecretError> {
        let service = self
            .app
            .config()
            .product_name
            .as_deref()
            .ok_or_else(|| access("OS credential vault is unavailable."))?;
        let entry = keyring::Entry::new(service, &secure_key(account)?)
            .map_err(|_| access("OS credential vault is unavailable."))?;
        read_entry(&entry)
    }

    #[cfg(target_os = "macos")]
    fn delete_local_current(&self, account: &str) -> Result<(), SecretError> {
        let service = self
            .app
            .config()
            .product_name
            .as_deref()
            .ok_or_else(|| access("OS credential vault is unavailable."))?;
        let entry = keyring::Entry::new(service, &secure_key(account)?)
            .map_err(|_| access("OS credential vault is unavailable."))?;
        delete_entry(&entry)
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    fn load_current(&self, account: &str) -> Result<Option<String>, SecretError> {
        // The plugin turns every read error into None on desktop. Query its
        // exact service/key directly so denied access cannot trigger migration
        // over an existing destination.
        let service = self
            .app
            .config()
            .product_name
            .as_deref()
            .ok_or_else(|| access("OS credential vault is unavailable."))?;
        let entry = keyring::Entry::new(service, &secure_key(account)?)
            .map_err(|_| access("OS credential vault is unavailable."))?;
        read_entry(&entry)
    }

    // Desktop secure-storage uses productName as the service, independently of
    // the key prefix. Changing just the prefix cannot find the old item.
    fn previous_entry(&self, account: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(
            PREVIOUS_PRODUCT_NAME,
            &secure_key_for(PREVIOUS_APP_ID, account)?,
        )
        .map_err(|_| access("Previous credential vault is unavailable."))
    }

    fn load_previous(&self, account: &str) -> Result<Option<String>, SecretError> {
        read_entry(&self.previous_entry(account)?)
    }

    fn delete_previous(&self, account: &str) -> Result<(), SecretError> {
        delete_entry(&self.previous_entry(account)?)
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    fn load_legacy(&self, account: &str) -> Result<Option<String>, SecretError> {
        for service in LEGACY_SERVICES {
            let entry = keyring::Entry::new(service, account)
                .map_err(|_| access("Legacy credential vault is unavailable."))?;
            if let Some(secret) = read_entry(&entry)? {
                migrate_secret(self, account, &secret)?;
                let _ = delete_entry(&entry);
                return Ok(Some(secret));
            }
        }
        Ok(None)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn load_legacy(&self, _account: &str) -> Result<Option<String>, SecretError> {
        Ok(None)
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    fn delete_legacy(&self, account: &str) -> Result<(), SecretError> {
        for service in LEGACY_SERVICES {
            let entry = keyring::Entry::new(service, account)
                .map_err(|_| access("Legacy credential vault is unavailable."))?;
            delete_entry(&entry)?;
        }
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn delete_legacy(&self, _account: &str) -> Result<(), SecretError> {
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn apple_sync_options(
    account: &str,
) -> Result<security_framework::passwords::PasswordOptions, SecretError> {
    let mut options = security_framework::passwords::PasswordOptions::new_generic_password(
        APP_ID,
        &secure_key(account)?,
    );
    options.set_access_synchronized(Some(true));
    options.use_protected_keychain();
    Ok(options)
}

#[cfg(target_os = "macos")]
fn set_apple_synced(account: &str, secret: &str) -> Result<(), SecretError> {
    security_framework::passwords::set_generic_password_options(
        secret.as_bytes(),
        apple_sync_options(account)?,
    )
    .map_err(|error| {
        access(format!(
            "iCloud Keychain write failed (OSStatus {}).",
            error.code()
        ))
    })
}

#[cfg(target_os = "macos")]
fn get_apple_synced(account: &str) -> Result<Option<String>, SecretError> {
    match security_framework::passwords::generic_password(apple_sync_options(account)?) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| access("iCloud Keychain contains invalid credentials.")),
        Err(error) if error.code() == -25300 => Ok(None), // errSecItemNotFound
        Err(error) => Err(access(format!(
            "iCloud Keychain read failed (OSStatus {}).",
            error.code()
        ))),
    }
}

#[cfg(target_os = "macos")]
fn delete_apple_synced(account: &str) -> Result<(), SecretError> {
    match security_framework::passwords::delete_generic_password_options(apple_sync_options(
        account,
    )?) {
        Ok(()) => Ok(()),
        Err(error) if error.code() == -25300 => Ok(()), // errSecItemNotFound
        Err(error) => Err(access(format!(
            "iCloud Keychain delete failed (OSStatus {}).",
            error.code()
        ))),
    }
}

fn read_entry(entry: &keyring::Entry) -> Result<Option<String>, SecretError> {
    match entry.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        // Cancellation/denied access must not be mistaken for missing passwords.
        Err(_) => Err(access("Previous credential vault read failed.")),
    }
}

fn delete_entry(entry: &keyring::Entry) -> Result<(), SecretError> {
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(access("Previous credential vault delete failed.")),
    }
}

#[cfg(test)]
#[path = "tests/secrets.rs"]
mod tests;
