use crate::StudentId;

// The ID reaches the printer's billing field unchecked, so only the exact
// shape passes: every programme letter, in capitals, and nothing else.
#[test]
fn only_a_student_id_passes_and_comes_out_in_capitals() {
    for (text, id) in [
        ("24B12345", "24B12345"),
        ("24m12345", "24M12345"),
        ("23D00001", "23D00001"),
        ("25R99999", "25R99999"),
    ] {
        assert_eq!(
            StudentId::parse(text).map(|s| s.to_string()),
            Some(id.into())
        );
    }
    for text in [
        "",
        "24B1234",
        "24B123456",
        "24X12345",
        "2BB12345",
        "24B1234a",
        " 24B12345",
        "24B12345\n",
        "２4B12345",
        "user",
    ] {
        assert_eq!(StudentId::parse(text), None, "{text:?}");
    }
}

#[test]
fn the_address_is_the_printers_host_and_port() {
    assert_eq!(
        crate::ADDRESS.to_string(),
        format!("{}:{}", crate::HOST, crate::PORT)
    );
}
