use super::*;

#[test]
fn limits_are_inclusive_byte_lengths() {
    assert!(NotificationContent::new("t".repeat(MAX_TITLE_BYTES), "b").is_ok());
    assert!(NotificationContent::new("t", "b".repeat(MAX_BODY_BYTES)).is_ok());
    assert_eq!(
        NotificationContent::new("t".repeat(MAX_TITLE_BYTES + 1), ""),
        Err(NotificationContentError::TooLarge)
    );
    // Multibyte text is bounded by bytes, as before.
    assert_eq!(
        NotificationContent::new("", "授".repeat(MAX_BODY_BYTES / 3 + 1)),
        Err(NotificationContentError::TooLarge)
    );
}
