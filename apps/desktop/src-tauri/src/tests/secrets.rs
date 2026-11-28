use super::{secure_key, validate_secret, SecretError};

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
