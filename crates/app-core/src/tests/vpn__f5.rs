use super::*;

#[test]
fn the_helper_cookie_carries_the_session_and_its_timers() {
    assert_eq!(
        session_cookie("m", Some("1c2c")).unwrap().as_str(),
        "MRHSession=m; F5_ST=1c2c"
    );
    assert_eq!(session_cookie("m", None).unwrap().as_str(), "MRHSession=m");
    assert_eq!(
        session_cookie("m", Some(" ")).unwrap().as_str(),
        "MRHSession=m"
    );
}
