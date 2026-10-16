use super::{
    access, copy_verified, secure_key, secure_key_for, validate_secret, SecretError,
    PREVIOUS_APP_ID,
};

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
    assert!(copy_verified("secret", || Ok(()), || Err(access("denied"))).is_err());
    assert!(copy_verified(
        "secret",
        || Err(access("write failed")),
        || panic!("must not read after failed write")
    )
    .is_err());
}

#[test]
fn arbitrary_secret_accounts_are_rejected() {
    assert_eq!(
        secure_key("untrusted-account"),
        Err(SecretError::InvalidAccount)
    );
    assert_eq!(validate_secret(""), Err(SecretError::InvalidPayload));
    assert_eq!(
        validate_secret(&"x".repeat(64 * 1024 + 1)),
        Err(SecretError::InvalidPayload)
    );
}
