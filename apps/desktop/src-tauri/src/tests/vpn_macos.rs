use super::*;

/// The installer runs under a root shell built from paths that contain
/// spaces and quotes; a quoting slip would run something else as root.
#[test]
fn installer_quotes_every_path_it_is_given() {
    assert_eq!(sh_quote("a b"), "'a b'");
    assert_eq!(sh_quote("it's"), r"'it'\''s'");
    let script = install_script(Path::new("/Applications/Isct.app/Contents/Resources/vpn"));
    assert!(script.contains("'/Library/Application Support/IsctCompanion/vpn'"));
    assert!(script.contains(
        "launchctl bootstrap system '/Library/LaunchDaemons/app.isctcompanion.vpnhelper.plist'"
    ));
}

#[test]
fn installer_script_parses_as_sh() {
    let script = install_script(Path::new("/Applications/Isct.app/Contents/Resources/vpn"));
    let status = std::process::Command::new("/bin/sh")
        .arg("-n")
        .arg("-c")
        .arg(&script)
        .status()
        .expect("sh runs");
    assert!(
        status.success(),
        "installer script is not valid sh: {script}"
    );
}

/// The daemon must run the copied openconnect and its libraries, never a
/// Homebrew path, or an install on a Mac without Homebrew would fail.
#[test]
fn launchd_plist_points_only_at_installed_copies() {
    let plist = launchd_plist();
    assert!(plist.contains(
        "<string>/Library/Application Support/IsctCompanion/vpn/openconnect/bin/openconnect</string>"
    ));
    assert!(plist.contains(
        "<string>/Library/Application Support/IsctCompanion/vpn/openconnect/lib</string>"
    ));
    assert!(!plist.contains("/opt/homebrew"));
    assert!(!plist.contains("/tmp"));
}
