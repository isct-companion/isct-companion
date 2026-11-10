use super::*;

/// The daemon and the app agree on these exact strings; a rename here would
/// silently break every installed helper.
#[test]
fn start_request_uses_the_wire_names() {
    let request = Request::from_line(r#"{"op":"start","cookie":"MRHSession=a; F5_ST=b"}"#).unwrap();
    assert_eq!(
        request,
        Request::Start {
            cookie: SessionCookie::new("MRHSession=a; F5_ST=b").unwrap()
        }
    );
    assert!(Request::from_line(r#"{"op":"stop"}"#).is_ok());
    assert!(Request::from_line(r#"{"op":"status"}"#).is_ok());
}

/// The helper accepts start/stop/status only; anything else must be refused
/// instead of falling through to a default operation.
#[test]
fn unknown_operations_and_empty_cookies_are_refused() {
    assert!(Request::from_line(r#"{"op":"reboot"}"#).is_err());
    assert!(Request::from_line(r#"{"op":"start","cookie":"  "}"#).is_err());
    assert!(Request::from_line(r#"{"op":"start"}"#).is_err());
}

#[test]
fn cookie_with_a_line_break_is_refused() {
    assert_eq!(
        SessionCookie::new("a=b\nc=d"),
        Err(CookieError::ControlCharacter)
    );
}

#[test]
fn cookie_never_prints_its_value() {
    let cookie = SessionCookie::new("MRHSession=secret").unwrap();
    let rendered = format!("{cookie:?} {:?}", Request::Start { cookie: cookie.clone() });
    assert!(!rendered.contains("secret"));
}

#[test]
fn reply_state_serializes_as_lowercase_names() {
    let json = serde_json::to_string(&Reply::disconnected()).unwrap();
    assert_eq!(
        json,
        format!(r#"{{"ok":true,"state":"disconnected","revision":{HELPER_REVISION}}}"#)
    );
    // The first helpers sent no revision; they read as revision none.
    let old: Reply = serde_json::from_str(r#"{"ok":true,"state":"connected"}"#).unwrap();
    assert_eq!(old.revision, None);
    let error = serde_json::to_value(Reply::error("boom")).unwrap();
    assert_eq!(error["state"], "error");
    assert_eq!(error["detail"], "boom");
}
