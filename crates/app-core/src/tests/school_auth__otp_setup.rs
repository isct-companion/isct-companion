use super::*;
use crate::school_auth::{tests::MemoryVault, totp::current_otp_at, SchoolCredentials};

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
    assert_eq!(service.otp_setup_link().unwrap(), saved);
    assert!(service.otp_setup_qr().unwrap().contains("<svg"));
    let uri = otp_setup_uri("jbsw y3dp-ehpk3pxp", "user@example");
    let imported = crate::school_auth::import_otp(&uri).unwrap();
    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0].issuer, "Science Tokyo");
    assert_eq!(imported[0].label, "Science Tokyo:user@example");
    assert_eq!(
        current_otp_at(&uri, 59).unwrap().otp,
        current_otp_at(draft, 59).unwrap().otp
    );
}
