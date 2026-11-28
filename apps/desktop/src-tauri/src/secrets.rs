use app_core::ports::{vault_item, SecretError, SecretVault};
use tauri::AppHandle;
use tauri_plugin_secure_storage::{OptionsRequest, SecureStorageExt};

#[cfg(target_os = "macos")]
const APP_ID: &str = app_core::ports::VAULT_SERVICE;

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

// The data-protection keychain needs the keychain-access-groups entitlement
// (errSecMissingEntitlement, -34018), which only builds signed with the team
// profile carry (`icloud`): release builds and team-signed debug bundles.
// Development builds keep school logins in the login keychain, under their own
// service (productName).
fn apple_keychain_sync(account: &str) -> bool {
    cfg!(icloud) && vault_item(account).is_ok_and(|item| item.synchronized)
}

/// The school credentials' status metadata, in the data folder. The demo
/// keeps its status in memory (`SchoolAuthService::demo`).
#[cfg(not(feature = "demo"))]
pub const SCHOOL_STATUS_FILE: &str = "school-credential-status.json";

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

    fn load(&self, account: &str) -> Result<Option<String>, SecretError> {
        self.load_current(account)
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        #[cfg(target_os = "macos")]
        if apple_keychain_sync(account) {
            return delete_apple_synced(account);
        }
        let removed = self
            .app
            .secure_storage()
            .remove_item(self.app.clone(), request(account, None)?);
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

    // A Credential Manager item holds 2560 bytes, and keyring writes UTF-16.
    #[cfg(windows)]
    fn item_limit(&self) -> Option<usize> {
        Some(1280)
    }
}

impl OsSecretVault {
    #[cfg(target_os = "macos")]
    fn load_current(&self, account: &str) -> Result<Option<String>, SecretError> {
        if apple_keychain_sync(account) {
            return get_apple_synced(account);
        }
        self.load_local(account)
    }

    /// The plugin turns every read error into None on desktop. Its exact
    /// service/key is read directly so a denied read is not taken for a
    /// missing item.
    fn load_local(&self, account: &str) -> Result<Option<String>, SecretError> {
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

    #[cfg(target_os = "windows")]
    fn load_current(&self, account: &str) -> Result<Option<String>, SecretError> {
        self.load_local(account)
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
        Err(_) => Err(access("OS credential vault read failed.")),
    }
}

#[cfg(test)]
#[path = "tests/secrets.rs"]
mod tests;
