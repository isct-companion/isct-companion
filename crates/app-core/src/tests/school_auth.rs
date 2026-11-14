use super::*;
use std::collections::HashMap;
use std::sync::Mutex;

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
        .resolve(
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
        .promote(
            SchoolAuthProfile::Science,
            &SchoolCredentials {
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

    // The QR export carries a link other authenticators accept: a saved link
    // stays as is, and a bare secret gains an issuer and the account name
    // while still producing the same codes.
    assert_eq!(service.otp_setup_link(None).unwrap(), saved);
    assert!(service.otp_setup_qr(None).unwrap().contains("<svg"));
    assert!(service.otp_setup_qr(Some("123456")).is_err());
    let uri = otp_setup_uri("jbsw y3dp-ehpk3pxp", "user@example");
    let imported = crate::school_auth::import_otp(&uri).unwrap();
    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0].issuer, "Science Tokyo");
    assert_eq!(imported[0].label, "Science Tokyo:user@example");
    assert_eq!(otp_at(&uri, 59).unwrap(), otp_at(draft, 59).unwrap());
}

#[test]
fn matrix_text_follows_the_editors_visual_row_order() {
    let service = SchoolAuthService::new(MemoryVault::default());
    assert!(service.matrix_text().is_err());
    // Row-major A1…J1, A2…: cell i holds the i-th letter, wrapping after Z.
    let matrix: String = (0..70u8).map(|i| (b'A' + i % 26) as char).collect();
    service
        .promote(
            SchoolAuthProfile::TokyoTech,
            &SchoolCredentials {
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

fn science_credentials(username: &str, password: &str) -> SchoolCredentials {
    SchoolCredentials {
        username: username.into(),
        password: password.into(),
        otp_secret: "JBSWY3DPEHPK3PXP".into(),
        matrix: String::new(),
    }
}

#[cfg(feature = "desktop-otp-enrollment")]
#[test]
fn enrollment_keeps_unconfirmed_otp_out_of_export_and_saves_confirmed_otp_without_a_password() {
    const SECRET: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
    const USER: &str = "student";
    let vault = MemoryVault::default();
    let service = SchoolAuthService::new(vault.clone());

    assert_eq!(
        service.enrollment_account("other", USER),
        Err(EnrollmentError::Account)
    );
    let base = service.enrollment_account(USER, USER).unwrap();
    service
        .stage_enrollment("registration", USER, base, SECRET)
        .unwrap();
    assert!(service.otp_setup_text(None).is_err());
    assert!(service.confirm_enrollment("stale").is_err());

    assert!(!service.confirm_enrollment("registration").unwrap());
    let restarted = SchoolAuthService::new(vault);
    let status = restarted.status().unwrap().science;
    assert!(status.has_otp);
    assert!(!status.saved);
    assert_eq!(restarted.otp_setup_text(None).unwrap(), SECRET);
    assert!(!restarted.export_sync_payload().unwrap().contains(SECRET));

    let mut draft = science_credentials(USER, "password");
    draft.otp_secret.clear();
    let credentials = restarted
        .resolve(SchoolAuthProfile::Science, draft)
        .unwrap();
    assert_eq!(credentials.otp_secret, SECRET);
    restarted.delete(SchoolAuthProfile::Science).unwrap();
    assert!(restarted.otp_setup_text(None).is_err());
}

fn synced_accounts(vault: &MemoryVault) -> HashMap<SchoolAuthProfile, SchoolCredentials> {
    vault
        .load(SCHOOL_CREDENTIAL_ACCOUNT)
        .unwrap()
        .map(|payload| {
            serde_json::from_str::<SchoolCredentialBundle>(&payload)
                .unwrap()
                .accounts
        })
        .unwrap_or_default()
}

// Only credentials the school accepted are saved, in one synchronized item
// for both profiles. The school's refusal of them stays on this device, kept
// with exactly the credentials it is about: it holds across a relaunch, never
// syncs, and goes once the credentials change or the school accepts them.
#[test]
fn a_refusal_stays_local_and_holds_only_the_credentials_it_is_about() {
    let vault = MemoryVault::default();
    let inspect = vault.clone();
    let service = SchoolAuthService::new(vault);
    service
        .promote(
            SchoolAuthProfile::Science,
            &science_credentials("science", "password"),
        )
        .unwrap();
    let tokyo = SchoolCredentials {
        username: "tokyo".into(),
        password: "password".into(),
        otp_secret: String::new(),
        matrix: "A".repeat(70),
    };
    service
        .promote(SchoolAuthProfile::TokyoTech, &tokyo)
        .unwrap();
    let status = service.status().unwrap();
    assert!(status.science.has_otp);
    assert_eq!(status.science.username, "science");
    assert!(status.tokyo_tech.has_matrix);
    assert!(!status.tokyo_tech.has_otp);
    assert_eq!(synced_accounts(&inspect).len(), 2);

    let failure = SchoolSignInProblem {
        kind: crate::school_session::SchoolSignInProblemKind::InvalidCredentials,
        message: "The ID or password was not accepted.".into(),
    };
    // Credentials that are not the saved ones are never refused on record.
    let typo = SchoolCredentials {
        password: "typo".into(),
        ..tokyo.clone()
    };
    assert!(!service
        .record_refusal(SchoolAuthProfile::TokyoTech, &typo, &failure)
        .unwrap());
    assert!(service
        .record_refusal(SchoolAuthProfile::TokyoTech, &tokyo, &failure)
        .unwrap());
    assert_eq!(
        service.refusal(SchoolAuthProfile::TokyoTech),
        Some(failure.clone())
    );
    assert!(
        !crate::ports::vault_item(SCHOOL_REFUSAL_ACCOUNT)
            .unwrap()
            .synchronized
    );
    assert!(!service
        .export_sync_payload()
        .unwrap()
        .contains("not accepted"));

    // A relaunch shows it from the device item alone, and a sign-in reads
    // it with the credentials.
    let relaunched = SchoolAuthService::new(inspect.clone());
    relaunched.load_refusals().unwrap();
    assert_eq!(
        relaunched.refusal(SchoolAuthProfile::TokyoTech),
        Some(failure.clone())
    );
    assert_eq!(
        relaunched
            .sign_in_credentials(SchoolAuthProfile::TokyoTech)
            .unwrap()
            .unwrap()
            .refusal,
        Some(failure.clone())
    );
    assert_eq!(relaunched.refusal(SchoolAuthProfile::Science), None);

    // Only an acceptance of exactly those credentials clears it.
    relaunched
        .clear_refusal(SchoolAuthProfile::TokyoTech, &typo)
        .unwrap();
    assert!(relaunched.refusal(SchoolAuthProfile::TokyoTech).is_some());
    relaunched
        .clear_refusal(SchoolAuthProfile::TokyoTech, &tokyo)
        .unwrap();
    assert_eq!(relaunched.refusal(SchoolAuthProfile::TokyoTech), None);
    assert!(inspect.load(SCHOOL_REFUSAL_ACCOUNT).unwrap().is_none());

    // New verified credentials drop a refusal of the old ones.
    relaunched
        .record_refusal(SchoolAuthProfile::TokyoTech, &tokyo, &failure)
        .unwrap();
    relaunched
        .promote(SchoolAuthProfile::TokyoTech, &typo)
        .unwrap();
    assert_eq!(relaunched.refusal(SchoolAuthProfile::TokyoTech), None);
    assert_eq!(
        synced_accounts(&inspect)[&SchoolAuthProfile::TokyoTech],
        typo
    );

    relaunched.delete(SchoolAuthProfile::Science).unwrap();
    assert!(relaunched
        .load(SchoolAuthProfile::Science)
        .unwrap()
        .is_none());
    assert_eq!(
        relaunched
            .load(SchoolAuthProfile::TokyoTech)
            .unwrap()
            .unwrap()
            .username,
        "tokyo"
    );
}

// The device item of an earlier release may still hold a typed password and
// OTP secret no sign-in accepted. The first device-local read of a launch
// (the restore at startup, or the first unlock) deletes it without reading
// it, and every shell's vault can still name it.
#[test]
fn the_retired_candidates_item_is_deleted_unread_at_startup() {
    #[derive(Clone, Default)]
    struct Watched(MemoryVault, Arc<Mutex<Vec<String>>>);
    impl SecretVault for Watched {
        fn store(&self, account: &str, secret: &str) -> Result<(), SecretError> {
            self.0.store(account, secret)
        }
        fn load(&self, account: &str) -> Result<Option<String>, SecretError> {
            self.1.lock().unwrap().push(account.to_owned());
            self.0.load(account)
        }
        fn delete(&self, account: &str) -> Result<(), SecretError> {
            self.0.delete(account)
        }
    }
    let candidates = r#"{"version":1,"accounts":{"science":{"credentials":{"username":"student","password":"typed","otpSecret":"JBSWY3DPEHPK3PXP"},"base":""}}}"#;
    assert!(crate::ports::vault_item(RETIRED_CANDIDATE_ACCOUNT).is_ok());

    for unlock in [false, true] {
        let vault = Watched::default();
        vault.store(RETIRED_CANDIDATE_ACCOUNT, candidates).unwrap();
        let service = SchoolAuthService::new(vault.clone());
        if unlock {
            assert!(service.load(SchoolAuthProfile::Science).unwrap().is_none());
        } else {
            service.load_refusals().unwrap();
        }
        assert!(vault.0.load(RETIRED_CANDIDATE_ACCOUNT).unwrap().is_none());
        assert!(!vault
            .1
            .lock()
            .unwrap()
            .iter()
            .any(|account| account == RETIRED_CANDIDATE_ACCOUNT));
        assert_eq!(service.refusal(SchoolAuthProfile::Science), None);
    }
}

// A refusal the vault fails to keep still holds the refused credentials
// back for as long as this process runs, and the caller hears that it was not
// kept: background sign-ins must never send a refused password again because
// a keychain write failed.
#[test]
fn a_refusal_the_vault_does_not_keep_still_holds_the_credentials_back() {
    #[derive(Clone, Default)]
    struct NoRefusals(MemoryVault, Arc<std::sync::atomic::AtomicBool>);
    impl SecretVault for NoRefusals {
        fn store(&self, account: &str, secret: &str) -> Result<(), SecretError> {
            if account == SCHOOL_REFUSAL_ACCOUNT
                && !self.1.load(std::sync::atomic::Ordering::SeqCst)
            {
                return Err(SecretError::Access("denied".into()));
            }
            self.0.store(account, secret)
        }
        fn load(&self, account: &str) -> Result<Option<String>, SecretError> {
            self.0.load(account)
        }
        fn delete(&self, account: &str) -> Result<(), SecretError> {
            self.0.delete(account)
        }
    }
    let vault = NoRefusals::default();
    let inspect = vault.0.clone();
    let allow = vault.1.clone();
    let service = SchoolAuthService::new(vault);
    let saved = science_credentials("student", "password");
    service.promote(SchoolAuthProfile::Science, &saved).unwrap();
    let failure = SchoolSignInProblem {
        kind: crate::school_session::SchoolSignInProblemKind::InvalidCredentials,
        message: "the school did not accept the credentials".into(),
    };

    assert!(service
        .record_refusal(SchoolAuthProfile::Science, &saved, &failure)
        .is_err());
    assert!(inspect.load(SCHOOL_REFUSAL_ACCOUNT).unwrap().is_none());
    assert!(service.refusal_unsaved(SchoolAuthProfile::Science));
    assert_eq!(
        service.refusal(SchoolAuthProfile::Science),
        Some(failure.clone())
    );
    // A person's re-read of the vault does not lift it either.
    service.refresh();
    assert_eq!(
        service
            .sign_in_credentials(SchoolAuthProfile::Science)
            .unwrap()
            .unwrap()
            .refusal,
        Some(failure.clone())
    );
    allow.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(service
        .record_refusal(SchoolAuthProfile::Science, &saved, &failure)
        .unwrap());
    assert!(!service.refusal_unsaved(SchoolAuthProfile::Science));
    assert!(inspect.load(SCHOOL_REFUSAL_ACCOUNT).unwrap().is_some());
}

// An acceptance marker left by an earlier acceptance that cannot be removed
// must not lift a later refusal at the next launch. The refusal of the first
// profile fails its marker and its write; once a write of the refusals lands
// (the other profile's), both are kept, and a relaunch holds both back. The
// "not saved" line goes only then. A real acceptance still ends the refusal.
#[test]
fn a_refusal_outlives_an_old_acceptance_marker_it_could_not_remove() {
    use std::sync::atomic::Ordering::SeqCst;
    #[derive(Clone, Default)]
    struct Stuck {
        items: MemoryVault,
        refuse_writes: Arc<std::sync::atomic::AtomicBool>,
    }
    impl SecretVault for Stuck {
        fn store(&self, account: &str, secret: &str) -> Result<(), SecretError> {
            if account == SCHOOL_REFUSAL_ACCOUNT && self.refuse_writes.load(SeqCst) {
                return Err(SecretError::Access("denied".into()));
            }
            self.items.store(account, secret)
        }
        fn load(&self, account: &str) -> Result<Option<String>, SecretError> {
            self.items.load(account)
        }
        fn delete(&self, account: &str) -> Result<(), SecretError> {
            if account == SchoolAuthProfile::Science.acceptance_key() {
                return Err(SecretError::Access("denied".into()));
            }
            self.items.delete(account)
        }
    }
    let problem = |message: &str| SchoolSignInProblem {
        kind: crate::school_session::SchoolSignInProblemKind::InvalidCredentials,
        message: message.into(),
    };
    let vault = Stuck::default();
    let service = SchoolAuthService::new(vault.clone());
    let science = science_credentials("student", "password");
    let tokyo = SchoolCredentials {
        username: "user".into(),
        password: "password".into(),
        otp_secret: String::new(),
        matrix: (0..70u8).map(|i| (b'A' + i % 26) as char).collect(),
    };
    service.promote(SchoolAuthProfile::Science, &science).unwrap();
    service.promote(SchoolAuthProfile::TokyoTech, &tokyo).unwrap();
    vault
        .items
        .store(SchoolAuthProfile::Science.acceptance_key(), "1")
        .unwrap();

    vault.refuse_writes.store(true, SeqCst);
    assert!(service
        .record_refusal(SchoolAuthProfile::Science, &science, &problem("science"))
        .is_err());
    assert!(service.refusal_unsaved(SchoolAuthProfile::Science));
    vault.refuse_writes.store(false, SeqCst);
    assert!(service
        .record_refusal(SchoolAuthProfile::TokyoTech, &tokyo, &problem("tokyo"))
        .unwrap());

    let relaunched = SchoolAuthService::new(vault.clone());
    relaunched.load_refusals().unwrap();
    assert_eq!(
        relaunched.refusal(SchoolAuthProfile::Science),
        Some(problem("science")),
        "a relaunch must not send the refused password again"
    );
    assert_eq!(
        service.refusal_unsaved(SchoolAuthProfile::Science),
        relaunched.refusal(SchoolAuthProfile::Science).is_none()
    );

    relaunched.clear_refusal(SchoolAuthProfile::Science, &science).unwrap();
    assert_eq!(relaunched.refusal(SchoolAuthProfile::Science), None);
    let again = SchoolAuthService::new(vault);
    again.load_refusals().unwrap();
    assert_eq!(again.refusal(SchoolAuthProfile::Science), None);
    assert_eq!(again.refusal(SchoolAuthProfile::TokyoTech), Some(problem("tokyo")));
}

// Keychain is the source of truth: another device replacing the synchronized
// item (a new OTP set on the Mac), or the person editing it in the system's
// keychain app, must reach this running process at its next explicit action
// (Retry, Connect), on every platform, end a refusal of the credentials it
// replaced, and be what blank fields fill from, so a Connect with only a
// password can never send an old OTP.
#[test]
fn an_external_keychain_change_is_used_from_the_next_explicit_action() {
    let vault = MemoryVault::default();
    let inspect = vault.clone();
    let service = SchoolAuthService::new(vault);
    let saved = science_credentials("student", "password");
    service.promote(SchoolAuthProfile::Science, &saved).unwrap();
    let failure = SchoolSignInProblem {
        kind: crate::school_session::SchoolSignInProblemKind::InvalidOtp,
        message: "the school did not accept the one-time password".into(),
    };
    service
        .record_refusal(SchoolAuthProfile::Science, &saved, &failure)
        .unwrap();

    let mac_vault = MemoryVault::default();
    let mac = SchoolAuthService::new(mac_vault.clone());
    let mut renewed = science_credentials("student", "password");
    renewed.otp_secret = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ".into();
    mac.promote(SchoolAuthProfile::Science, &renewed).unwrap();
    // What iCloud Keychain does: the item changes under the running app.
    inspect
        .store(
            SCHOOL_CREDENTIAL_ACCOUNT,
            &mac_vault.load(SCHOOL_CREDENTIAL_ACCOUNT).unwrap().unwrap(),
        )
        .unwrap();

    // Between checks the copy in memory answers.
    assert_eq!(
        service.load(SchoolAuthProfile::Science).unwrap(),
        Some(saved)
    );
    assert!(service.refusal(SchoolAuthProfile::Science).is_some());
    service.refresh();
    let read = service
        .sign_in_credentials(SchoolAuthProfile::Science)
        .unwrap()
        .unwrap();
    assert_eq!(read.credentials, renewed);
    assert_eq!(read.refusal, None);
    assert_eq!(service.refusal(SchoolAuthProfile::Science), None);
    assert!(inspect.load(SCHOOL_REFUSAL_ACCOUNT).unwrap().is_none());
    let password_only = service
        .resolve(
            SchoolAuthProfile::Science,
            SchoolCredentials {
                username: String::new(),
                password: "changed".into(),
                otp_secret: String::new(),
                matrix: String::new(),
            },
        )
        .unwrap();
    assert_eq!(password_only.otp_secret, renewed.otp_secret);
    assert_eq!(
        synced_accounts(&inspect)[&SchoolAuthProfile::Science],
        renewed
    );
}

#[test]
fn blank_secret_fields_keep_the_saved_encrypted_values() {
    let service = SchoolAuthService::new(MemoryVault::default());
    service
        .promote(
            SchoolAuthProfile::Science,
            &SchoolCredentials {
                username: "first-user".into(),
                password: "password".into(),
                otp_secret: "JBSWY3DPEHPK3PXP".into(),
                matrix: String::new(),
            },
        )
        .unwrap();
    let credentials = service
        .resolve(
            SchoolAuthProfile::Science,
            SchoolCredentials {
                username: "renamed-user".into(),
                password: String::new(),
                otp_secret: String::new(),
                matrix: String::new(),
            },
        )
        .unwrap();
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
            SCHOOL_CREDENTIAL_ACCOUNT,
            &verified_bundle(&test_credentials()),
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
            SCHOOL_CREDENTIAL_ACCOUNT,
            &verified_bundle(&test_credentials()),
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
fn verified_bundle(credentials: &SchoolCredentials) -> String {
    serde_json::json!({
        "version": 1,
        "accounts": { "science": credentials },
    })
    .to_string()
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
fn repeated_login_does_not_rewrite_unchanged_values() {
    let vault = CountingVault::default();
    vault
        .inner
        .store(
            SCHOOL_CREDENTIAL_ACCOUNT,
            &verified_bundle(&test_credentials()),
        )
        .unwrap();
    let inspect = vault.clone();
    let service = SchoolAuthService::new(vault);
    for _ in 0..3 {
        let saved = service.load(SchoolAuthProfile::Science).unwrap().unwrap();
        service.promote(SchoolAuthProfile::Science, &saved).unwrap();
        service.status().unwrap();
        service.resolve(SchoolAuthProfile::Science, saved).unwrap();
    }
    // Connecting again with the verified credentials writes nothing.
    assert_eq!(inspect.writes.load(SeqCst), 0);
    // The item is read once per process whatever asks, and again only
    // after a person's explicit action (`refresh`: Retry, Connect, import),
    // once, on every platform: on the macOS login keychain every read of an
    // item another build wrote asks for the password, and a read per use
    // asked endlessly.
    let reads = inspect.reads.load(SeqCst);
    service.load(SchoolAuthProfile::Science).unwrap();
    assert_eq!(inspect.reads.load(SeqCst), reads);
    service.refresh();
    service.load(SchoolAuthProfile::Science).unwrap();
    service
        .promote(SchoolAuthProfile::Science, &test_credentials())
        .unwrap();
    assert_eq!(inspect.reads.load(SeqCst), reads + 1);
    assert_eq!(inspect.writes.load(SeqCst), 0);
}
#[test]
fn changed_credentials_update_cache_only_after_a_successful_write() {
    let vault = CountingVault::default();
    let inspect = vault.clone();
    let service = SchoolAuthService::new(vault);
    service
        .promote(SchoolAuthProfile::Science, &test_credentials())
        .unwrap();
    let mut changed = test_credentials();
    changed.password = "new-test-password".into();
    inspect.fail_write.store(true, SeqCst);
    assert!(service
        .promote(SchoolAuthProfile::Science, &changed)
        .is_err());
    assert!(service.load(SchoolAuthProfile::Science).unwrap().unwrap() == test_credentials());
    inspect.fail_write.store(false, SeqCst);
    service
        .promote(SchoolAuthProfile::Science, &changed)
        .unwrap();
    assert!(service.load(SchoolAuthProfile::Science).unwrap().unwrap() == changed);
}
#[test]
fn denied_unlock_can_be_retried_and_forget_clears_the_session_copy() {
    let vault = CountingVault::default();
    vault
        .inner
        .store(
            SCHOOL_CREDENTIAL_ACCOUNT,
            &verified_bundle(&test_credentials()),
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
    let relaunched = SchoolAuthService::new(inspect.clone());
    assert!(relaunched
        .load(SchoolAuthProfile::Science)
        .unwrap()
        .is_none());
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
fn lms_token_stays_on_this_device_until_the_account_is_forgotten() {
    let vault = MemoryVault::default();
    let service = SchoolAuthService::new(vault.clone());
    let science = SchoolAuthProfile::Science;
    service.promote(science, &test_credentials()).unwrap();
    let token = MoodleToken::new("0123456789abcdef".into());
    service.store_moodle_token(science, &token).unwrap();
    // Its own device-only vault item.
    assert!(
        !crate::ports::vault_item("lms-token-science")
            .unwrap()
            .synchronized
    );
    assert_eq!(
        service.load_moodle_token(science).unwrap(),
        Some(token.clone())
    );

    // A record of an older format is not read: it goes, and the next read
    // launches a new token.
    vault
        .store(
            "lms-token-science",
            r#"{"version":1,"account":"test-user","token":"0123456789abcdef"}"#,
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
