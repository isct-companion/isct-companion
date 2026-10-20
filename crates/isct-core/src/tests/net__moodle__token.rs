use super::*;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};

const WWWROOT: &str = "https://lms.s.isct.ac.jp/2025";

fn answer(engine: &impl Engine, parts: &str) -> String {
    format!("moodlemobile://token={}", engine.encode(parts))
}

#[test]
fn signature_is_md5_of_wwwroot_then_passport_in_lowercase_hex() {
    // RFC 1321 vectors: the two inputs are concatenated in this order.
    assert_eq!(launch_signature("", ""), "d41d8cd98f00b204e9800998ecf8427e");
    assert_eq!(
        launch_signature("a", "bc"),
        "900150983cd24fb0d6963f7d28e17f72"
    );
    assert_ne!(launch_signature("bc", "a"), launch_signature("a", "bc"));
}

#[test]
fn launch_answer_yields_only_a_token_signed_for_this_passport() {
    let passport = new_passport();
    let signature = launch_signature(WWWROOT, &passport);
    let token = "0123456789abcdefABCDEF0123456789";

    // Three parts as observed: the private token is dropped, and the token
    // never shows in Debug output.
    let parsed = parse_launch_location(
        &answer(&STANDARD, &format!("{signature}:::{token}:::privateXYZ")),
        WWWROOT,
        &passport,
    )
    .unwrap();
    assert_eq!(parsed.expose(), token);
    let shown = format!("{parsed:?}");
    assert!(!shown.contains(token) && !shown.contains("privateXYZ"));

    // Two parts (no private token), unpadded or percent-encoded base64, an
    // uppercase signature.
    for location in [
        answer(&STANDARD_NO_PAD, &format!("{signature}:::{token}")),
        format!(
            "moodlemobile://token={}",
            STANDARD
                .encode(format!("{}:::{token}", signature.to_uppercase()))
                .replace('=', "%3D")
        ),
    ] {
        assert_eq!(
            parse_launch_location(&location, WWWROOT, &passport)
                .unwrap()
                .expose(),
            token
        );
    }

    // Signed for another passport or another site: someone else's launch.
    for (root, pass) in [
        (WWWROOT, "another"),
        ("https://lms.s.isct.ac.jp/2026", passport.as_str()),
    ] {
        assert_eq!(
            parse_launch_location(
                &answer(
                    &STANDARD,
                    &format!("{}:::{token}", launch_signature(root, pass))
                ),
                WWWROOT,
                &passport,
            )
            .unwrap_err(),
            LaunchError::SignatureMismatch
        );
    }
}

#[test]
fn launch_rejects_other_locations_and_malformed_answers() {
    let passport = "fixed-passport";
    let signature = launch_signature(WWWROOT, passport);
    // A sign-in redirect or another app scheme is not a launch answer.
    for location in [
        "https://lms.s.isct.ac.jp/2025/login/index.php",
        "https://isct.ex-tic.com/auth/session",
        "otherapp://token=abc",
        "",
    ] {
        assert_eq!(
            parse_launch_location(location, WWWROOT, passport).unwrap_err(),
            LaunchError::NotLaunch
        );
    }
    for parts in [
        signature.clone(),
        format!("{signature}:::"),
        format!("{signature}:::tok en"),
        format!("{signature}:::a:::b:::c"),
        "short:::abc".to_string(),
    ] {
        assert_eq!(
            parse_launch_location(&answer(&STANDARD, &parts), WWWROOT, passport).unwrap_err(),
            LaunchError::Malformed,
            "{parts}"
        );
    }
    assert_eq!(
        parse_launch_location("moodlemobile://token=***", WWWROOT, passport).unwrap_err(),
        LaunchError::Malformed
    );
}

#[test]
fn launch_url_asks_the_mobile_service_for_our_scheme() {
    let url = launch_url(WWWROOT, "p1").unwrap();
    assert_eq!(url.host_str(), Some("lms.s.isct.ac.jp"));
    assert_eq!(url.path(), "/2025/admin/tool/mobile/launch.php");
    let query: Vec<(String, String)> = url.query_pairs().into_owned().collect();
    assert_eq!(
        query,
        [
            ("service".to_string(), "moodle_mobile_app".to_string()),
            ("passport".to_string(), "p1".to_string()),
            ("urlscheme".to_string(), "moodlemobile".to_string()),
        ]
    );
    assert_ne!(new_passport(), new_passport());
}
