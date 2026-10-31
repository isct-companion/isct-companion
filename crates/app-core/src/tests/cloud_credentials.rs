use super::*;
use serde_json::Value;

const PASSWORD: &str = "eight888";

/// New passwords count characters, so a passphrase in any script fits; a
/// backup made under the old 8–12 rule still opens with its password.
#[test]
fn new_passwords_follow_the_length_policy_and_old_ones_still_unlock() {
    let shortest = "a".repeat(PASSWORD_LENGTH.min);
    let longest = "密".repeat(PASSWORD_LENGTH.max);
    assert!(validate_password(&shortest).is_ok());
    assert!(validate_password(&longest).is_ok());
    for invalid in [&shortest[1..], &format!("{longest}a"), "twelve chars\n"] {
        assert_eq!(
            validate_password(invalid),
            Err(CredentialEncryptionError::InvalidPassword),
            "{invalid:?}"
        );
    }
    assert_eq!(
        encrypt("{}", PASSWORD),
        Err(CredentialEncryptionError::InvalidPassword)
    );
    let old = encrypt_with_parameters("{}", PASSWORD, [1; 16], [2; 12]).unwrap();
    assert_eq!(decrypt(&old, PASSWORD).unwrap(), "{}");
    for unusable in ["", &format!("{longest}a"), "eight88\n"] {
        assert_eq!(
            decrypt(&old, unusable),
            Err(CredentialDecryptionError::InvalidPassword)
        );
    }
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

mod backup {
    use super::*;
    use crate::school_auth::SchoolAuthService;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    const NEW: &str = "correct horse battery staple";

    #[derive(Clone, Default)]
    struct MemoryVault(Arc<Mutex<HashMap<String, String>>>);

    impl SecretVault for MemoryVault {
        fn store(&self, account: &str, secret: &str) -> Result<(), SecretError> {
            self.0.lock().unwrap().insert(account.into(), secret.into());
            Ok(())
        }

        fn load(&self, account: &str) -> Result<Option<String>, SecretError> {
            Ok(self.0.lock().unwrap().get(account).cloned())
        }

        fn delete(&self, account: &str) -> Result<(), SecretError> {
            self.0.lock().unwrap().remove(account);
            Ok(())
        }
    }

    /// Drive has no headless mode, so the file store is a map that can be
    /// taken offline; the vault, encryption and school logins are real.
    #[derive(Default)]
    struct MemoryFiles {
        files: Mutex<HashMap<String, Vec<u8>>>,
        offline: AtomicBool,
    }

    impl MemoryFiles {
        fn backup(&self) -> Option<Vec<u8>> {
            self.files.lock().unwrap().get(FILE_NAME).cloned()
        }
    }

    impl CredentialFiles for MemoryFiles {
        fn read<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Option<Vec<u8>>> {
            Box::pin(async move { Ok(self.files.lock().unwrap().get(name).cloned()) })
        }

        fn write<'a>(&'a self, name: &'a str, bytes: Vec<u8>) -> BoxFuture<'a, ()> {
            Box::pin(async move {
                if self.offline.load(Ordering::SeqCst) {
                    return Err(DocumentError::Transport("offline".into()));
                }
                self.files.lock().unwrap().insert(name.into(), bytes);
                Ok(())
            })
        }
    }

    /// The password is kept only once the file it opened or wrote agrees
    /// with it, so a failed upload or a change that never reached Drive
    /// cannot leave this device holding a password the backup does not
    /// open. A device that finds a backup restores it, and a wrong password
    /// there never overwrites another device's logins.
    #[tokio::test]
    async fn the_password_is_kept_only_when_the_backup_agrees() {
        let files = MemoryFiles::default();
        let first_vault = MemoryVault::default();
        let first_school = SchoolAuthService::new(MemoryVault::default());
        first_school
            .stage_enrollment_otp("student", "JBSWY3DPEHPK3PXP", "one")
            .unwrap();
        first_school
            .confirm_enrollment_otp("one", "student")
            .unwrap();
        let first = CredentialBackup::new(&first_vault, &files);

        assert!(matches!(
            first.enable("too short", &first_school).await,
            Err(CredentialBackupError::Encryption(
                CredentialEncryptionError::InvalidPassword
            ))
        ));
        files.offline.store(true, Ordering::SeqCst);
        assert!(matches!(
            first.enable(NEW, &first_school).await,
            Err(CredentialBackupError::Cloud(_))
        ));
        assert!(!first.configured().unwrap());
        assert!(!first.exists().await.unwrap());
        files.offline.store(false, Ordering::SeqCst);
        assert_eq!(
            first.enable(NEW, &first_school).await.unwrap(),
            BackupOutcome::Uploaded
        );
        assert!(first.configured().unwrap());

        let second_vault = MemoryVault::default();
        let second_school = SchoolAuthService::new(MemoryVault::default());
        let second = CredentialBackup::new(&second_vault, &files);
        let uploaded = files.backup();
        assert!(matches!(
            second
                .enable("a different passphrase", &second_school)
                .await,
            Err(CredentialBackupError::Decryption(
                CredentialDecryptionError::Authentication
            ))
        ));
        assert!(!second.configured().unwrap());
        assert_eq!(files.backup(), uploaded);
        assert_eq!(
            second.enable(NEW, &second_school).await.unwrap(),
            BackupOutcome::Restored
        );
        assert!(second_school.status().unwrap().science.has_otp);

        files.offline.store(true, Ordering::SeqCst);
        assert!(first
            .enable("another long passphrase", &first_school)
            .await
            .is_err());
        assert_eq!(
            first_vault.load(PASSWORD_ACCOUNT).unwrap().as_deref(),
            Some(NEW)
        );
        assert!(decrypt(&files.backup().unwrap(), NEW).is_ok());

        first.disable().unwrap();
        assert!(!first.configured().unwrap());
        assert!(first.exists().await.unwrap());
    }
}
