use super::*;

#[test]
fn bounded_log_reads_report_gaps_and_do_not_export_secrets() {
    let mut buffer = Buffer::default();
    buffer.push("ERROR", "auth", "https://example.org/?access_token=private&code=123456 password=hidden Authorization: Bearer private-token");
    let first = buffer.snapshot(None);
    assert_eq!(first.cursor, 1);
    let text = &first.entries[0].message;
    for secret in ["private", "123456", "hidden", "private-token"] {
        assert!(!text.contains(secret), "{text}");
    }
    assert!(text.contains("[redacted]"));
    buffer.push("WARN", "sync", "first line\nsecond line");
    let export = buffer.export("IsctCompanion 1.0.0", "2026-10-20T00:00:00Z");
    assert!(
        export.starts_with("IsctCompanion 1.0.0\nExported 2026-10-20T00:00:00Z · 2 entries\n\n")
    );
    assert!(export.contains(" ERROR [auth] https://example.org/?access_token=[redacted]"));
    assert!(
        export.ends_with(" WARN [sync] first line\n    second line\n"),
        "{export}"
    );
    for secret in ["private", "123456", "hidden", "private-token"] {
        assert!(!export.contains(secret), "{export}");
    }
    for _ in 0..CAPACITY + 3 {
        buffer.push("INFO", "sync", "saved");
    }
    let all = buffer.snapshot(None);
    assert_eq!(all.entries.len(), CAPACITY);
    let delta = buffer.snapshot(Some(1));
    assert!(delta.dropped);
    assert_eq!(delta.entries.len(), CAPACITY);
    assert!(buffer.snapshot(Some(all.cursor)).entries.is_empty());
    buffer.push("ERROR", "app", &"x".repeat(MESSAGE_LIMIT * 2));
    let last = buffer.snapshot(Some(all.cursor));
    assert!(!last.dropped);
    assert_eq!(last.entries.len(), 1);
    assert_eq!(last.entries[0].message.len(), MESSAGE_LIMIT);
    assert_eq!(last.entries[0].id, all.cursor + 1);
}
