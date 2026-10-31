//! The encrypted school-credential backup in Google Drive: its file
//! contract and the one flow every shell drives through [`CredentialBackup`].

use crate::cloud_sync::document::{BoxFuture, DocumentError};
use crate::cloud_sync::google_drive::{DriveAuthorization, GoogleDrive};
use crate::ports::{SecretError, SecretVault};
use crate::school_auth::{SchoolAuthError, SchoolAuthService};
use base64::Engine;
use ring::{
    aead, pbkdf2,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;
use thiserror::Error;

pub const FILE_NAME: &str = "IsctCompanion.credentials.json";
pub const PASSWORD_ACCOUNT: &str = "cloud-credential-sync";
const FORMAT: &str = "IsctCompanion.credentials";
const FORMAT_VERSION: u64 = 1;
const KDF: &str = "pbkdf2-sha256";
const KDF_ITERATIONS: u32 = 600_000;
const CIPHER: &str = "aes-256-gcm";
const AAD: &[u8] = b"IsctCompanion.credentials.v1";
const SALT_BYTES: usize = 16;
const NONCE_BYTES: usize = 12;
const TAG_BYTES: usize = 16;
const MAX_PLAINTEXT_BYTES: usize = 64 * 1024;

/// Characters a newly chosen password must have. The file holds every sign-in
/// factor, so a passphrase is welcome; the cap only bounds what a guess costs.
pub const PASSWORD_LENGTH: PasswordLength = PasswordLength { min: 12, max: 128 };

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct PasswordLength {
    pub min: usize,
    pub max: usize,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum CredentialEncryptionError {
    #[error("Encryption password must be 12–128 characters.")]
    InvalidPassword,
    #[error("Secure random data could not be generated.")]
    RandomUnavailable,
    #[error("Encrypted credential KDF is invalid.")]
    InvalidKdf,
    #[error("Credential encryption setup failed.")]
    Setup,
    #[error("Credential encryption failed.")]
    Encryption,
    #[error("Encrypted credentials could not be encoded.")]
    Encoding,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum CredentialDecryptionError {
    #[error("Encryption password cannot unlock any backup.")]
    InvalidPassword,
    #[error("Encrypted credential file is invalid.")]
    InvalidFile,
    #[error("Encrypted credential file version is unsupported.")]
    UnsupportedFormat,
    #[error("Encrypted credential salt is invalid.")]
    InvalidSalt,
    #[error("Encrypted credential nonce is invalid.")]
    InvalidNonce,
    #[error("Encrypted credential payload is invalid.")]
    InvalidPayload,
    #[error("Encrypted credential KDF is invalid.")]
    InvalidKdf,
    #[error("Credential decryption setup failed.")]
    Setup,
    #[error("Credential decryption failed. Check the encryption password.")]
    Authentication,
    #[error("Decrypted credential payload is invalid.")]
    InvalidPlaintext,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EncryptedCredentialFile {
    format: String,
    format_version: u64,
    kdf: String,
    iterations: u32,
    salt: String,
    cipher: String,
    nonce: String,
    ciphertext: String,
}

fn validate_password(password: &str) -> Result<(), CredentialEncryptionError> {
    let length = password.chars().count();
    if !(PASSWORD_LENGTH.min..=PASSWORD_LENGTH.max).contains(&length)
        || password.chars().any(char::is_control)
    {
        return Err(CredentialEncryptionError::InvalidPassword);
    }
    Ok(())
}

/// A backup opens with the password it was made with, which earlier versions
/// allowed from 8 characters; a wrong one fails the authentication tag.
fn unlockable(password: &str) -> bool {
    !password.is_empty()
        && password.chars().count() <= PASSWORD_LENGTH.max
        && !password.chars().any(char::is_control)
}

pub fn encrypt(payload: &str, password: &str) -> Result<Vec<u8>, CredentialEncryptionError> {
    validate_password(password)?;
    let random = SystemRandom::new();
    let mut salt = [0u8; SALT_BYTES];
    let mut nonce = [0u8; NONCE_BYTES];
    random
        .fill(&mut salt)
        .map_err(|_| CredentialEncryptionError::RandomUnavailable)?;
    random
        .fill(&mut nonce)
        .map_err(|_| CredentialEncryptionError::RandomUnavailable)?;
    encrypt_with_parameters(payload, password, salt, nonce)
}

fn encrypt_with_parameters(
    payload: &str,
    password: &str,
    salt: [u8; SALT_BYTES],
    nonce: [u8; NONCE_BYTES],
) -> Result<Vec<u8>, CredentialEncryptionError> {
    encrypt_bytes_with_parameters(payload.as_bytes(), password, salt, nonce)
}

fn encrypt_bytes_with_parameters(
    payload: &[u8],
    password: &str,
    salt: [u8; SALT_BYTES],
    nonce: [u8; NONCE_BYTES],
) -> Result<Vec<u8>, CredentialEncryptionError> {
    let key = derive_key(password, &salt, KDF_ITERATIONS)
        .map_err(|_| CredentialEncryptionError::InvalidKdf)?;
    let key = aead::UnboundKey::new(&aead::AES_256_GCM, &key)
        .map_err(|_| CredentialEncryptionError::Setup)?;
    let key = aead::LessSafeKey::new(key);
    let mut ciphertext = payload.to_vec();
    key.seal_in_place_append_tag(
        aead::Nonce::assume_unique_for_key(nonce),
        aead::Aad::from(AAD),
        &mut ciphertext,
    )
    .map_err(|_| CredentialEncryptionError::Encryption)?;
    let encode = base64::engine::general_purpose::STANDARD;
    serde_json::to_vec_pretty(&EncryptedCredentialFile {
        format: FORMAT.into(),
        format_version: FORMAT_VERSION,
        kdf: KDF.into(),
        iterations: KDF_ITERATIONS,
        salt: encode.encode(salt),
        cipher: CIPHER.into(),
        nonce: encode.encode(nonce),
        ciphertext: encode.encode(ciphertext),
    })
    .map_err(|_| CredentialEncryptionError::Encoding)
}

pub fn decrypt(bytes: &[u8], password: &str) -> Result<String, CredentialDecryptionError> {
    if !unlockable(password) {
        return Err(CredentialDecryptionError::InvalidPassword);
    }
    let file: EncryptedCredentialFile =
        serde_json::from_slice(bytes).map_err(|_| CredentialDecryptionError::InvalidFile)?;
    if file.format != FORMAT
        || file.format_version != FORMAT_VERSION
        || file.kdf != KDF
        || file.iterations != KDF_ITERATIONS
        || file.cipher != CIPHER
    {
        return Err(CredentialDecryptionError::UnsupportedFormat);
    }
    let decode = base64::engine::general_purpose::STANDARD;
    let salt = decode
        .decode(file.salt)
        .map_err(|_| CredentialDecryptionError::InvalidSalt)?;
    if salt.len() != SALT_BYTES {
        return Err(CredentialDecryptionError::InvalidSalt);
    }
    let nonce: [u8; NONCE_BYTES] = decode
        .decode(file.nonce)
        .map_err(|_| CredentialDecryptionError::InvalidNonce)?
        .try_into()
        .map_err(|_| CredentialDecryptionError::InvalidNonce)?;
    let mut ciphertext = decode
        .decode(file.ciphertext)
        .map_err(|_| CredentialDecryptionError::InvalidPayload)?;
    if !(TAG_BYTES..=(MAX_PLAINTEXT_BYTES + TAG_BYTES)).contains(&ciphertext.len()) {
        return Err(CredentialDecryptionError::InvalidPayload);
    }
    let key = derive_key(password, &salt, file.iterations)
        .map_err(|_| CredentialDecryptionError::InvalidKdf)?;
    let key = aead::UnboundKey::new(&aead::AES_256_GCM, &key)
        .map_err(|_| CredentialDecryptionError::Setup)?;
    let key = aead::LessSafeKey::new(key);
    let plaintext = key
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(AAD),
            &mut ciphertext,
        )
        .map_err(|_| CredentialDecryptionError::Authentication)?;
    String::from_utf8(plaintext.to_vec()).map_err(|_| CredentialDecryptionError::InvalidPlaintext)
}

fn derive_key(password: &str, salt: &[u8], iterations: u32) -> Result<[u8; 32], ()> {
    let iterations = NonZeroU32::new(iterations).ok_or(())?;
    let mut key = [0u8; 32];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        iterations,
        salt,
        password.as_bytes(),
        &mut key,
    );
    Ok(key)
}

#[derive(Debug, Error)]
pub enum CredentialBackupError {
    #[error(transparent)]
    Encryption(#[from] CredentialEncryptionError),
    #[error(transparent)]
    Decryption(#[from] CredentialDecryptionError),
    #[error(transparent)]
    Cloud(#[from] DocumentError),
    #[error(transparent)]
    Vault(#[from] SecretError),
    #[error(transparent)]
    School(#[from] SchoolAuthError),
}

/// What turning the backup on did.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BackupOutcome {
    /// The existing backup opened and its logins replaced this device's.
    Restored,
    /// This device's verified logins were encrypted and uploaded.
    Uploaded,
}

/// Where the encrypted file is kept: Google Drive's app-data folder. iCloud
/// has none, since Apple devices share logins through iCloud Keychain.
pub trait CredentialFiles: Send + Sync {
    fn read<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Option<Vec<u8>>>;
    fn write<'a>(&'a self, name: &'a str, bytes: Vec<u8>) -> BoxFuture<'a, ()>;
}

impl<A: DriveAuthorization> CredentialFiles for GoogleDrive<A> {
    fn read<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Option<Vec<u8>>> {
        Box::pin(self.read_file(name))
    }

    fn write<'a>(&'a self, name: &'a str, bytes: Vec<u8>) -> BoxFuture<'a, ()> {
        Box::pin(self.write_file(name, bytes))
    }
}

/// The credential backup of one device. Its password lives in the device
/// vault and is saved only after the backup it opened or wrote succeeded, so
/// a failed attempt never leaves the two disagreeing.
pub struct CredentialBackup<'a, V, F> {
    vault: &'a V,
    files: &'a F,
}

impl<'a, V: SecretVault, F: CredentialFiles> CredentialBackup<'a, V, F> {
    pub fn new(vault: &'a V, files: &'a F) -> Self {
        Self { vault, files }
    }

    /// Whether this device holds the backup password.
    pub fn configured(&self) -> Result<bool, CredentialBackupError> {
        Ok(self.vault.load(PASSWORD_ACCOUNT)?.is_some())
    }

    pub async fn exists(&self) -> Result<bool, CredentialBackupError> {
        Ok(self.files.read(FILE_NAME).await?.is_some())
    }

    /// On a device without the password, an existing backup is opened with
    /// `password` and restored rather than overwritten. Otherwise `password`
    /// is a new one: this device's verified logins are encrypted with it and
    /// uploaded, which is also how the password changes.
    pub async fn enable(
        &self,
        password: &str,
        school: &SchoolAuthService,
    ) -> Result<BackupOutcome, CredentialBackupError> {
        if !self.configured()? {
            if let Some(bytes) = self.files.read(FILE_NAME).await? {
                school.import_sync_payload(&decrypt(&bytes, password)?)?;
                self.vault.store(PASSWORD_ACCOUNT, password)?;
                return Ok(BackupOutcome::Restored);
            }
        }
        let encrypted = encrypt(&school.export_sync_payload()?, password)?;
        self.files.write(FILE_NAME, encrypted).await?;
        self.vault.store(PASSWORD_ACCOUNT, password)?;
        Ok(BackupOutcome::Uploaded)
    }

    /// Forgets this device's password; the Drive file stays for other devices.
    pub fn disable(&self) -> Result<(), CredentialBackupError> {
        Ok(self.vault.delete(PASSWORD_ACCOUNT)?)
    }
}

#[cfg(test)]
#[path = "tests/cloud_credentials.rs"]
mod tests;
