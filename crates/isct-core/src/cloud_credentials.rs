//! Portable encrypted credential-file contract used by every native shell.

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

pub fn supports_provider(provider: crate::cloud_sync::CloudProvider) -> bool {
    provider != crate::cloud_sync::CloudProvider::Icloud
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum CredentialEncryptionError {
    #[error("Encryption password must be 8–12 characters.")]
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
    #[error("Encryption password must be 8–12 characters.")]
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

pub fn validate_password(password: &str) -> Result<(), CredentialEncryptionError> {
    let length = password.chars().count();
    if !(8..=12).contains(&length) || password.chars().any(char::is_control) {
        return Err(CredentialEncryptionError::InvalidPassword);
    }
    Ok(())
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
    validate_password(password).map_err(|_| CredentialDecryptionError::InvalidPassword)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const PASSWORD: &str = "eight888";

    #[test]
    fn password_contract_counts_characters_and_rejects_controls() {
        assert_eq!(
            validate_password("1234567"),
            Err(CredentialEncryptionError::InvalidPassword)
        );
        assert_eq!(
            validate_password("1234567890123"),
            Err(CredentialEncryptionError::InvalidPassword)
        );
        assert_eq!(
            validate_password("1234567\n"),
            Err(CredentialEncryptionError::InvalidPassword)
        );
        assert!(validate_password("密码长度八个字符!!").is_ok());
        assert_eq!(
            encrypt("{}", "short"),
            Err(CredentialEncryptionError::InvalidPassword)
        );
    }

    #[test]
    fn credential_backups_exclude_icloud() {
        use crate::cloud_sync::CloudProvider;

        assert!(supports_provider(CloudProvider::GoogleDrive));
        assert!(supports_provider(CloudProvider::Onedrive));
        assert!(!supports_provider(CloudProvider::Icloud));
    }

    #[test]
    fn format_and_crypto_round_trip_match_the_existing_contract() {
        let encrypted = encrypt_with_parameters(
            "{\"version\":1}",
            PASSWORD,
            [7; SALT_BYTES],
            [9; NONCE_BYTES],
        )
        .unwrap();
        let file: Value = serde_json::from_slice(&encrypted).unwrap();
        assert_eq!(file["format"], FORMAT);
        assert_eq!(file["formatVersion"], FORMAT_VERSION);
        assert_eq!(file["kdf"], KDF);
        assert_eq!(file["iterations"], KDF_ITERATIONS);
        assert_eq!(file["cipher"], CIPHER);
        assert_eq!(decrypt(&encrypted, PASSWORD).unwrap(), "{\"version\":1}");
    }

    #[test]
    fn reports_format_and_encoding_errors_precisely() {
        assert_eq!(
            decrypt(b"not json", PASSWORD),
            Err(CredentialDecryptionError::InvalidFile)
        );
        let encrypted = encrypt_with_parameters("{}", PASSWORD, [1; 16], [2; 12]).unwrap();
        let mut file: Value = serde_json::from_slice(&encrypted).unwrap();
        file["formatVersion"] = 2.into();
        assert_eq!(
            decrypt(&serde_json::to_vec(&file).unwrap(), PASSWORD),
            Err(CredentialDecryptionError::UnsupportedFormat)
        );
        file["formatVersion"] = FORMAT_VERSION.into();
        file["salt"] = "invalid!".into();
        assert_eq!(
            decrypt(&serde_json::to_vec(&file).unwrap(), PASSWORD),
            Err(CredentialDecryptionError::InvalidSalt)
        );
    }

    #[test]
    fn rejects_wrong_password_tampering_and_oversized_payloads() {
        let encrypted = encrypt_with_parameters("{}", PASSWORD, [3; 16], [4; 12]).unwrap();
        assert_eq!(
            decrypt(&encrypted, "wrong999"),
            Err(CredentialDecryptionError::Authentication)
        );

        let mut file: Value = serde_json::from_slice(&encrypted).unwrap();
        file["ciphertext"] = base64::engine::general_purpose::STANDARD
            .encode(vec![0u8; MAX_PLAINTEXT_BYTES + TAG_BYTES + 1])
            .into();
        assert_eq!(
            decrypt(&serde_json::to_vec(&file).unwrap(), PASSWORD),
            Err(CredentialDecryptionError::InvalidPayload)
        );

        let invalid_utf8 =
            encrypt_bytes_with_parameters(&[0xff], PASSWORD, [5; 16], [6; 12]).unwrap();
        assert_eq!(
            decrypt(&invalid_utf8, PASSWORD),
            Err(CredentialDecryptionError::InvalidPlaintext)
        );
    }
}
