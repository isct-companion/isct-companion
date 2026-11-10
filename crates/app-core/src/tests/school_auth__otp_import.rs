use super::*;

fn google(accounts: Vec<Parameters>, batch_size: i32) -> String {
    export(accounts, batch_size, 1)
}
fn export(accounts: Vec<Parameters>, batch_size: i32, version: i32) -> String {
    let wire = Migration {
        accounts,
        version,
        batch_size,
        batch_index: 0,
        batch_id: 17,
    }
    .encode_to_vec();
    let mut url = Url::parse("otpauth-migration://offline").unwrap();
    url.query_pairs_mut()
        .append_pair("data", &general_purpose::STANDARD.encode(wire));
    url.into()
}
fn school() -> Parameters {
    Parameters {
        secret: b"12345678901234567890".to_vec(),
        name: "Science Tokyo:student".into(),
        issuer: "Science Tokyo".into(),
        algorithm: 1,
        digits: 1,
        kind: 2,
        counter: 0,
    }
}

#[test]
fn standard_and_google_imports_preserve_accounts_and_real_totp_parameters() {
    let raw="otpauth://totp/Science%20Tokyo%3Astudent?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=Science%20Tokyo";
    let imported = import_otp(raw).unwrap();
    assert_eq!(imported[0].label, "Science Tokyo:student");
    assert_eq!(imported[0].issuer, "Science Tokyo");
    assert_eq!(
        super::super::otp_at(&imported[0].setup, 59).unwrap(),
        "287082"
    );
    let mut second = school();
    second.name = "Other:student".into();
    second.issuer = "Other".into();
    second.digits = 2;
    let imported = import_otp(&google(vec![school(), second], 1)).unwrap();
    assert_eq!(imported.len(), 2);
    assert_eq!(imported[0].label, "Science Tokyo:student");
    assert_eq!(
        super::super::otp_at(&imported[0].setup, 59).unwrap(),
        "287082"
    );
    assert_eq!(
        super::super::otp_at(&imported[1].setup, 59).unwrap(),
        "94287082"
    ); // Current Google Authenticator exports are version 2 with the same fields.
    let current = import_otp(&export(vec![school()], 1, 2)).unwrap();
    assert_eq!(
        super::super::otp_at(&current[0].setup, 59).unwrap(),
        "287082"
    );
    assert!(import_otp(&export(vec![school()], 1, 3)).is_err());
}

#[test]
fn incomplete_unsupported_and_unrelated_codes_never_produce_partial_imports() {
    assert!(matches!(
        import_otp(&google(vec![school()], 2)),
        Err(OtpImportError::MultiplePages)
    ));
    let mut hotp = school();
    hotp.kind = 1;
    assert!(matches!(
        import_otp(&google(vec![school(), hotp], 1)),
        Err(OtpImportError::Unsupported)
    ));
    let mut unknown = school();
    unknown.algorithm = 0;
    assert!(matches!(
        import_otp(&google(vec![unknown], 1)),
        Err(OtpImportError::Unsupported)
    ));
    for raw in [
        "https://example.com",
        "123456",
        "otpauth://hotp/test?secret=GEZDGNBVGY3TQOJQ",
        "otpauth://totp/test?secret=bad",
        "otpauth-migration://offline?data=AAAA",
        "otpauth-migration://offline?data=%",
    ] {
        assert!(import_otp(raw).is_err(), "{raw}");
    }
    assert!(import_otp(&"x".repeat(65537)).is_err());
    assert!(import_otp(&google(vec![], 1)).is_err());
}
