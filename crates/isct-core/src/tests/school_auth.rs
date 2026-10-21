use super::*;
use std::collections::HashMap;
use std::sync::Mutex;

/// Enrollment is scoped to App Authentication: an enabled email factor or
/// a login/error page must never be mistaken for an unset app factor.
#[test]
fn otp_registration_discovery_is_scoped_and_rejects_unknown_pages() {
    let page = |app: &str| {
        format!(
            "<div class='form-group'><label for='appAuth'>App Authentication</label>{app}</div>\
         <div class='form-group'><label for='mailAuth'>Email Authentication</label>\
         <button class='btn-danger'>Remove</button></div>"
        )
    };
    assert!(!otp_setup::registration_state(&page(
        "<button onclick=\"doGet('../totp_setting/add')\">Setup</button>"
    ))
    .unwrap());
    assert!(
        otp_setup::registration_state(&page("<button class='btn-danger'>Remove</button>")).unwrap()
    );
    for html in [
        "<form><input type='password'></form>".to_string(),
        page("<p>Unknown</p>"),
        page("<button onclick=\"doGet('../totp_setting/add')\">Setup</button><button class='btn-danger'>Remove</button>"),
        format!("{}{}", page(""), page("")),
    ] {
        assert!(matches!(otp_setup::registration_state(&html), Err(OtpSetupError::UnexpectedPage)));
    }
}

#[derive(Clone, Default)]
struct MemoryVault(Arc<Mutex<HashMap<String, String>>>);

impl SecretVault for MemoryVault {
    fn store(&self, account: &str, secret: &str) -> Result<(), SecretError> {
        self.0
            .lock()
            .unwrap()
            .insert(account.to_string(), secret.to_string());
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

#[test]
fn validates_matrix_shape_and_normalizes_case() {
    let mut credentials = SchoolCredentials {
        username: " user ".into(),
        password: "password".into(),
        otp_secret: String::new(),
        matrix: "a".repeat(70),
    };
    credentials.validate(SchoolAuthProfile::TokyoTech).unwrap();
    assert_eq!(credentials.username, "user");
    assert_eq!(credentials.matrix, "A".repeat(70));
}

#[test]
fn reports_invalid_input_and_incompatible_storage_separately() {
    let invalid = SchoolAuthService::new(MemoryVault::default())
        .save(
            SchoolAuthProfile::TokyoTech,
            SchoolCredentials {
                username: "user".into(),
                password: "password".into(),
                otp_secret: String::new(),
                matrix: "A".repeat(69),
            },
        )
        .unwrap_err();
    assert!(matches!(invalid, SchoolAuthError::InvalidInput(_)));

    let vault = MemoryVault::default();
    vault.store(SCHOOL_CREDENTIAL_ACCOUNT, "not-json").unwrap();
    let incompatible = SchoolAuthService::new(vault)
        .load(SchoolAuthProfile::Science)
        .unwrap_err();
    assert!(matches!(incompatible, SchoolAuthError::Compatibility(_)));
}

#[test]
fn otp_export_prefers_a_valid_draft_without_mutating_saved_credentials() {
    let service = SchoolAuthService::new(MemoryVault::default());
    assert!(service.otp_setup_text(None).is_err());
    let saved = "otpauth://totp/School?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&digits=8&period=60";
    service
        .save(
            SchoolAuthProfile::Science,
            SchoolCredentials {
                username: "user".into(),
                password: "password".into(),
                otp_secret: saved.into(),
                matrix: String::new(),
            },
        )
        .unwrap();
    assert_eq!(service.otp_setup_text(None).unwrap(), saved);
    let draft = "JBSWY3DPEHPK3PXP";
    assert_eq!(service.otp_setup_text(Some(draft)).unwrap(), draft);
    assert!(service.otp_setup_text(Some("123456")).is_err());
    assert!(service.otp_setup_text(Some(&"A".repeat(1025))).is_err());
    assert_eq!(service.otp_setup_text(Some(" ")).unwrap(), saved);
    assert_eq!(
        service
            .load(SchoolAuthProfile::Science)
            .unwrap()
            .unwrap()
            .otp_secret,
        saved
    );
}

#[test]
fn matrix_text_follows_the_editors_visual_row_order() {
    let service = SchoolAuthService::new(MemoryVault::default());
    assert!(service.matrix_text().is_err());
    // Row-major A1…J1, A2…: cell i holds the i-th letter, wrapping after Z.
    let matrix: String = (0..70u8).map(|i| (b'A' + i % 26) as char).collect();
    service
        .save(
            SchoolAuthProfile::TokyoTech,
            SchoolCredentials {
                username: "user".into(),
                password: "password".into(),
                otp_secret: String::new(),
                matrix,
            },
        )
        .unwrap();
    let text = service.matrix_text().unwrap();
    let rows = text.lines().collect::<Vec<_>>();
    assert_eq!(rows.len(), 7);
    assert!(rows.iter().all(|row| row.len() == 10));
    assert_eq!(rows[0], "ABCDEFGHIJ");
    assert_eq!(rows[6], "IJKLMNOPQR");
}

#[test]
fn upcoming_codes_cover_consecutive_windows() {
    let windows = upcoming_otps("JBSWY3DPEHPK3PXP").unwrap();
    assert_eq!(windows.len(), 4);
    assert!(windows
        .windows(2)
        .all(|pair| pair[1].from - pair[0].from == 30));
    for window in &windows {
        assert_eq!(
            window.code,
            otp_at("JBSWY3DPEHPK3PXP", window.from).unwrap()
        );
    }
}

#[test]
fn matches_rfc_6238_sha1_vector() {
    let uri = "otpauth://totp/test?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&digits=8&period=30&algorithm=SHA1";
    assert_eq!(otp_at(uri, 59).unwrap(), "94287082");
}

#[test]
fn stores_both_school_profiles_in_one_item() {
    let vault = MemoryVault::default();
    let inspect = vault.clone();
    let service = SchoolAuthService::new(vault);
    service
        .save(
            SchoolAuthProfile::Science,
            SchoolCredentials {
                username: "science".into(),
                password: "password".into(),
                otp_secret: "JBSWY3DPEHPK3PXP".into(),
                matrix: String::new(),
            },
        )
        .unwrap();
    service
        .save(
            SchoolAuthProfile::TokyoTech,
            SchoolCredentials {
                username: "tokyo".into(),
                password: "password".into(),
                otp_secret: String::new(),
                matrix: "A".repeat(70),
            },
        )
        .unwrap();
    let status = service.status().unwrap();
    assert!(status.science.has_otp);
    assert_eq!(status.science.username, "science");
    assert!(status.tokyo_tech.has_matrix);
    assert!(!status.tokyo_tech.has_otp);
    assert_eq!(inspect.0.lock().unwrap().len(), 1);
    assert!(inspect.load(SCHOOL_CREDENTIAL_ACCOUNT).unwrap().is_some());
    service.delete(SchoolAuthProfile::Science).unwrap();
    assert!(service.load(SchoolAuthProfile::Science).unwrap().is_none());
    assert_eq!(
        service
            .load(SchoolAuthProfile::TokyoTech)
            .unwrap()
            .unwrap()
            .username,
        "tokyo"
    );
}

#[test]
fn blank_secret_fields_keep_the_saved_encrypted_values() {
    let service = SchoolAuthService::new(MemoryVault::default());
    service
        .save(
            SchoolAuthProfile::Science,
            SchoolCredentials {
                username: "first-user".into(),
                password: "password".into(),
                otp_secret: "JBSWY3DPEHPK3PXP".into(),
                matrix: String::new(),
            },
        )
        .unwrap();
    service
        .save(
            SchoolAuthProfile::Science,
            SchoolCredentials {
                username: "renamed-user".into(),
                password: String::new(),
                otp_secret: String::new(),
                matrix: String::new(),
            },
        )
        .unwrap();

    let credentials = service.load(SchoolAuthProfile::Science).unwrap().unwrap();
    assert_eq!(credentials.username, "renamed-user");
    assert_eq!(credentials.password, "password");
    assert_eq!(credentials.otp_secret, "JBSWY3DPEHPK3PXP");
}

#[test]
fn persisted_account_status_does_not_unlock_keychain_or_persist_secrets() {
    let dir = std::env::temp_dir().join(format!("school-status-test-{}", uuid::Uuid::new_v4()));
    let path = dir.join("status.json");
    let vault = CountingVault::default();
    vault
        .inner
        .store(
            SchoolAuthProfile::Science.account(),
            &serde_json::to_string(&test_credentials()).unwrap(),
        )
        .unwrap();
    let service = SchoolAuthService::with_status_cache(vault, path.clone());
    assert!(service.status().unwrap().science.saved);
    let metadata = std::fs::read_to_string(&path).unwrap();
    assert!(!metadata.contains("test-password"));
    assert!(!metadata.contains("otpSecret"));
    let locked = CountingVault::default();
    locked.fail_read.store(true, SeqCst);
    let inspect = locked.clone();
    let restarted = SchoolAuthService::with_status_cache(locked, path.clone());
    let status = restarted.status().unwrap();
    assert!(status.science.saved);
    assert_eq!(
        status.science.availability,
        CredentialAvailability::SavedLocked
    );
    assert_eq!(inspect.reads.load(SeqCst), 0);
    // Display metadata cannot authorize an actual login or bypass the vault.
    assert!(restarted.load(SchoolAuthProfile::Science).is_err());
    assert!(restarted.delete(SchoolAuthProfile::Science).is_err());
    let status = restarted.status().unwrap();
    assert!(status.science.saved);
    assert_eq!(
        status.science.availability,
        CredentialAvailability::AccessDenied
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_vault_entry_replaces_stale_saved_metadata() {
    let dir = std::env::temp_dir().join(format!("school-missing-test-{}", uuid::Uuid::new_v4()));
    let path = dir.join("status.json");
    let vault = MemoryVault::default();
    vault
        .store(
            SchoolAuthProfile::Science.account(),
            &serde_json::to_string(&test_credentials()).unwrap(),
        )
        .unwrap();
    let original = SchoolAuthService::with_status_cache(vault, path.clone());
    assert!(original.status().unwrap().science.saved);

    let restarted = SchoolAuthService::with_status_cache(MemoryVault::default(), path.clone());
    assert!(restarted
        .load(SchoolAuthProfile::Science)
        .unwrap()
        .is_none());
    let status = restarted.status().unwrap();
    assert!(!status.science.saved);
    assert_eq!(status.science.availability, CredentialAvailability::Missing);
    std::fs::remove_dir_all(dir).unwrap();
}

#[derive(Clone, Default)]
struct CountingVault {
    inner: MemoryVault,
    reads: Arc<std::sync::atomic::AtomicUsize>,
    writes: Arc<std::sync::atomic::AtomicUsize>,
    fail_read: Arc<std::sync::atomic::AtomicBool>,
    fail_write: Arc<std::sync::atomic::AtomicBool>,
}
use std::sync::atomic::Ordering::SeqCst;
impl SecretVault for CountingVault {
    fn load(&self, account: &str) -> Result<Option<String>, SecretError> {
        self.reads.fetch_add(1, SeqCst);
        if self.fail_read.load(SeqCst) {
            return Err(SecretError::Access("locked".into()));
        }
        self.inner.load(account)
    }
    fn store(&self, account: &str, value: &str) -> Result<(), SecretError> {
        self.writes.fetch_add(1, SeqCst);
        if self.fail_write.load(SeqCst) {
            return Err(SecretError::Access("write denied".into()));
        }
        self.inner.store(account, value)
    }
    fn delete(&self, account: &str) -> Result<(), SecretError> {
        self.inner.delete(account)
    }
}
fn test_credentials() -> SchoolCredentials {
    SchoolCredentials {
        username: "test-user".into(),
        password: "test-password".into(),
        otp_secret: "JBSWY3DPEHPK3PXP".into(),
        matrix: String::new(),
    }
}
#[test]
fn repeated_login_reads_keychain_once_and_does_not_rewrite_unchanged_values() {
    let vault = CountingVault::default();
    vault
        .inner
        .store(
            SchoolAuthProfile::Science.account(),
            &serde_json::to_string(&test_credentials()).unwrap(),
        )
        .unwrap();
    let inspect = vault.clone();
    let service = SchoolAuthService::new(vault);
    for _ in 0..3 {
        let saved = service.load(SchoolAuthProfile::Science).unwrap().unwrap();
        service
            .save(SchoolAuthProfile::Science, saved.clone())
            .unwrap();
        service.status().unwrap();
        service.resolve(SchoolAuthProfile::Science, saved).unwrap();
    }
    assert_eq!(inspect.reads.load(SeqCst), 4); // bundle, two legacy items, verified replacement
    assert_eq!(inspect.writes.load(SeqCst), 1); // migration only
}
#[test]
fn failed_migration_keeps_legacy_credentials_and_can_retry() {
    let vault = CountingVault::default();
    vault
        .inner
        .store(
            SchoolAuthProfile::Science.account(),
            &serde_json::to_string(&test_credentials()).unwrap(),
        )
        .unwrap();
    vault.fail_write.store(true, SeqCst);
    let inspect = vault.clone();
    let service = SchoolAuthService::new(vault);
    assert!(matches!(
        service.load(SchoolAuthProfile::Science),
        Err(SchoolAuthError::Vault(SecretError::Access(_)))
    ));
    assert!(inspect
        .inner
        .load(SchoolAuthProfile::Science.account())
        .unwrap()
        .is_some());
    assert!(inspect
        .inner
        .load(SCHOOL_CREDENTIAL_ACCOUNT)
        .unwrap()
        .is_none());
    inspect.fail_write.store(false, SeqCst);
    assert_eq!(
        service.load(SchoolAuthProfile::Science).unwrap(),
        Some(test_credentials())
    );
    assert!(inspect
        .inner
        .load(SchoolAuthProfile::Science.account())
        .unwrap()
        .is_none());
    assert!(inspect
        .inner
        .load(SCHOOL_CREDENTIAL_ACCOUNT)
        .unwrap()
        .is_some());
}

#[test]
fn changed_credentials_update_cache_only_after_a_successful_write() {
    let vault = CountingVault::default();
    let inspect = vault.clone();
    let service = SchoolAuthService::new(vault);
    service
        .save(SchoolAuthProfile::Science, test_credentials())
        .unwrap();
    let mut changed = test_credentials();
    changed.password = "new-test-password".into();
    inspect.fail_write.store(true, SeqCst);
    assert!(service
        .save(SchoolAuthProfile::Science, changed.clone())
        .is_err());
    assert!(service.load(SchoolAuthProfile::Science).unwrap().unwrap() == test_credentials());
    inspect.fail_write.store(false, SeqCst);
    service
        .save(SchoolAuthProfile::Science, changed.clone())
        .unwrap();
    assert!(service.load(SchoolAuthProfile::Science).unwrap().unwrap() == changed);
}
#[test]
fn denied_unlock_can_be_retried_and_forget_clears_the_session_copy() {
    let vault = CountingVault::default();
    vault
        .inner
        .store(
            SchoolAuthProfile::Science.account(),
            &serde_json::to_string(&test_credentials()).unwrap(),
        )
        .unwrap();
    let inspect = vault.clone();
    let service = SchoolAuthService::new(vault);
    inspect.fail_read.store(true, SeqCst);
    assert!(service.load(SchoolAuthProfile::Science).is_err());
    inspect.fail_read.store(false, SeqCst);
    assert!(service.load(SchoolAuthProfile::Science).unwrap().is_some());
    service.delete(SchoolAuthProfile::Science).unwrap();
    assert!(service.load(SchoolAuthProfile::Science).unwrap().is_none());
    assert_eq!(inspect.reads.load(SeqCst), 5);
    // A new application/service instance performs its own unlock.
    let relaunched = SchoolAuthService::new(inspect.clone());
    assert!(relaunched
        .load(SchoolAuthProfile::Science)
        .unwrap()
        .is_none());
    assert_eq!(inspect.reads.load(SeqCst), 6);
}

#[test]
fn undergraduate_student_numbers_suggest_only_the_guide_entry_year() {
    for (number, expected) in [
        ("25B12345", Some(2025)),
        (" 24b54321 ", Some(2024)),
        ("25M12345", None),
        ("25D12345", None),
        ("student", None),
        ("25B1234", None),
        ("25B1234x", None),
    ] {
        assert_eq!(student_admission_year(number), expected);
    }
}

#[test]
fn lms_token_belongs_to_the_account_that_obtained_it() {
    let vault = MemoryVault::default();
    let service = SchoolAuthService::new(vault.clone());
    let science = SchoolAuthProfile::Science;
    service.save(science, test_credentials()).unwrap();
    let token = MoodleToken::new("0123456789abcdef".into());
    service.store_moodle_token(science, &token).unwrap();
    // Its own device-only vault item, naming the account it belongs to.
    assert!(vault
        .load("lms-token-science")
        .unwrap()
        .unwrap()
        .contains("test-user"));
    assert!(
        !crate::ports::vault_item("lms-token-science")
            .unwrap()
            .synchronized
    );
    assert_eq!(
        service.load_moodle_token(science).unwrap(),
        Some(token.clone())
    );

    // Another account signs in: the old account's token is dropped, not used.
    service
        .save(
            science,
            SchoolCredentials {
                username: "other-user".into(),
                ..test_credentials()
            },
        )
        .unwrap();
    assert_eq!(service.load_moodle_token(science).unwrap(), None);
    assert!(vault.load("lms-token-science").unwrap().is_none());

    // Forgetting the account forgets its token; students cannot revoke it.
    service.store_moodle_token(science, &token).unwrap();
    service.delete(science).unwrap();
    assert!(vault.load("lms-token-science").unwrap().is_none());
    assert!(service
        .store_moodle_token(SchoolAuthProfile::TokyoTech, &token)
        .is_err());
}
