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

    let invalid_utf8 = encrypt_bytes_with_parameters(&[0xff], PASSWORD, [5; 16], [6; 12]).unwrap();
    assert_eq!(
        decrypt(&invalid_utf8, PASSWORD),
        Err(CredentialDecryptionError::InvalidPlaintext)
    );
}
