use super::*;

/// The shim is what openconnect runs as its hook, and cscript ends with 0 on
/// an uncaught error: a shim that cannot find the helper once left the tunnel
/// with no route and nothing in any log. Runs the real file through cscript
/// from a path with a space, as under Program Files, beside a stand-in that
/// exits 1 for the `hook` argument it does not know.
#[test]
fn the_route_hook_shim_starts_the_helper_beside_it() {
    let dir = std::env::temp_dir().join(format!("isct vpn shim {}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let script = dir.join(SCRIPT);
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join(SCRIPT),
        &script,
    )
    .unwrap();
    let run = || {
        Command::new("cscript.exe")
            .args(["//nologo", "//e:JScript"])
            .arg(&script)
            .output()
            .unwrap()
            .status
            .code()
    };
    assert_eq!(run(), Some(2), "no helper beside the shim");
    let system = std::env::var_os("SystemRoot").unwrap();
    fs::copy(
        Path::new(&system).join(r"System32\whoami.exe"),
        dir.join(HELPER_EXE),
    )
    .unwrap();
    assert_eq!(run(), Some(1), "the helper's own exit code");
    let _ = fs::remove_dir_all(&dir);
}

/// The service runs as LocalSystem from this command line. A path into the
/// per-user app directory would let the signed-in user swap what SYSTEM runs.
#[test]
fn the_service_runs_only_the_installed_copies() {
    let dir = Path::new(r"C:\Program Files\IsctCompanion\VPN");
    let data = Path::new(r"C:\ProgramData\IsctCompanion\VPN");
    let args: Vec<String> = service_arguments(dir, data, "apm.example")
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert_eq!(args[0], "service");
    let value = |flag: &str| {
        let at = args.iter().position(|arg| arg == flag).expect(flag);
        args[at + 1].clone()
    };
    assert_eq!(
        value("--openconnect"),
        r"C:\Program Files\IsctCompanion\VPN\openconnect\openconnect.exe"
    );
    assert_eq!(
        value("--script"),
        r"C:\Program Files\IsctCompanion\VPN\isct-vpn-script.js"
    );
    assert_eq!(
        value("--log"),
        r"C:\ProgramData\IsctCompanion\VPN\helper.log"
    );
    assert_eq!(value("--server"), "apm.example");
}
