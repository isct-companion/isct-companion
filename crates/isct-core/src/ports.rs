use crate::events::CoreEvent;
use thiserror::Error;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum EventError {
    #[error("{0}")]
    Delivery(String),
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: CoreEvent) -> Result<(), EventError>;
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum SecretError {
    #[error("Native vault callback failed: {0}")]
    Callback(String),
    #[error("Secret account is not allowed.")]
    InvalidAccount,
    #[error("Secret payload is invalid.")]
    InvalidPayload,
    #[error("{0}")]
    Access(String),
}

pub trait SecretVault: Send + Sync {
    fn store(&self, account: &str, secret: &str) -> Result<(), SecretError>;
    fn load(&self, account: &str) -> Result<Option<String>, SecretError>;
    fn delete(&self, account: &str) -> Result<(), SecretError>;
}

/// Keychain service every shell stores its secrets under.
pub const VAULT_SERVICE: &str = "app.isctcompanion";

/// Where a vault account lives in the platform keychain. Shells use the same
/// identity, so a school login synchronized through iCloud Keychain on one
/// Apple device signs in on the others.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VaultItem {
    pub service: &'static str,
    pub key: String,
    /// School logins follow the person across devices; OAuth tokens and the
    /// credential-sync password stay on the device.
    pub synchronized: bool,
}

/// The keychain key of `account` under an application identifier. Desktop also
/// reads the identifiers of earlier releases through this.
pub fn vault_key(app_id: &str, account: &str) -> Result<String, SecretError> {
    let suffix = match account {
        "school-accounts" => "passwords.school-accounts",
        "school-science" => "passwords.isct-portal",
        "school-tokyo-tech" => "passwords.titech-portal",
        "google-drive" => "passwords.google-drive",
        "onedrive" => "passwords.onedrive",
        "icloud" => "passwords.icloud",
        "cloud-credential-sync" => "passwords.cloud-credential-sync",
        _ => return Err(SecretError::InvalidAccount),
    };
    Ok(format!("{app_id}.{suffix}"))
}

pub fn vault_item(account: &str) -> Result<VaultItem, SecretError> {
    Ok(VaultItem {
        service: VAULT_SERVICE,
        key: vault_key(VAULT_SERVICE, account)?,
        synchronized: matches!(
            account,
            "school-accounts" | "school-science" | "school-tokyo-tech"
        ),
    })
}
