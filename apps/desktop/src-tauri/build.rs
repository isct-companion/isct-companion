/// The desktop tools, in page order, and the systems each is built for: the
/// one place a tool's platforms are named. Each tool is a Tauri plugin of the
/// same name (`src/plugins/`); the code tests the tool, not the system
/// (`#[cfg(tool = "campus-printer")]`), and the page lists what the binary
/// reports (`__ISCT_COMPANION_TOOLS__`).
const TOOLS: &[(&str, &[&str])] = &[
    ("campus-wifi", &["macos"]),
    ("campus-vpn", &["macos", "windows"]),
    ("campus-printer", &["macos", "windows"]),
];

/// A tool plugin's commands on `os`, and which of them its `default`
/// permission allows: what the plugin grants the main window when it starts.
/// The Windows print window gets the printer's two printing commands from
/// the printer plugin instead.
fn plugin(tool: &str, os: &str) -> tauri_build::InlinedPlugin {
    use tauri_build::{DefaultPermissionRule, InlinedPlugin};
    let (commands, main): (&'static [&'static str], &'static [&'static str]) = match (tool, os) {
        ("campus-printer", "windows") => (
            &["status", "add_menu", "install", "queue_status", "print_request", "print"],
            &["allow-status", "allow-add-menu", "allow-install", "allow-queue-status"],
        ),
        ("campus-printer", _) => (&["status", "install"], &[]),
        ("campus-vpn", _) => (&["status", "connect", "disconnect", "keep", "install"], &[]),
        ("campus-wifi", _) => (&["status", "connect"], &[]),
        _ => panic!("{tool} has no plugin"),
    };
    let default = if main.is_empty() {
        DefaultPermissionRule::AllowAllCommands
    } else {
        DefaultPermissionRule::Allow(main.iter().map(|grant| grant.to_string()).collect())
    };
    InlinedPlugin::new()
        .commands(commands)
        .default_permission(default)
}

fn main() {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let ids = TOOLS
        .iter()
        .map(|(id, _)| format!("\"{id}\""))
        .collect::<Vec<_>>();
    println!(
        "cargo::rustc-check-cfg=cfg(tool, values({}))",
        ids.join(", ")
    );
    let tools = TOOLS
        .iter()
        .filter(|(_, systems)| systems.contains(&os.as_str()))
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    for tool in &tools {
        println!("cargo::rustc-cfg=tool=\"{tool}\"");
    }
    println!("cargo::rustc-env=ISCT_DESKTOP_TOOLS={}", tools.join(","));

    // CloudKit traps (SIGILL) instead of returning an error in a binary that
    // lacks the iCloud container entitlement. Unsigned macOS debug builds lack
    // it; `scripts/desktop/tauri.mjs` sets ISCT_APPLE_ENTITLED when it signs the
    // bundle with the entitlements and a provisioning profile that grants them.
    println!("cargo::rustc-check-cfg=cfg(icloud)");
    println!("cargo::rerun-if-env-changed=ISCT_APPLE_ENTITLED");
    let apple = std::env::var("CARGO_CFG_TARGET_VENDOR").is_ok_and(|vendor| vendor == "apple");
    let debug = std::env::var_os("CARGO_CFG_DEBUG_ASSERTIONS").is_some();
    let entitled = std::env::var("ISCT_APPLE_ENTITLED").is_ok_and(|value| value == "1");
    if apple && (os != "macos" || !debug || entitled) {
        println!("cargo::rustc-cfg=icloud");
    }
    // Windows reads the Matrix card with LiteRT. app-ocr's build fetched the
    // pinned DLL; stage it where tauri.windows.conf.json bundles it beside
    // the executable (before tauri_build checks that the resource exists).
    if os == "windows" {
        if let Some(dll) = std::env::var_os("DEP_LITERT_DLL") {
            let staged = std::path::Path::new("vendor/litert");
            std::fs::create_dir_all(staged).expect("Creating vendor/litert failed");
            std::fs::copy(dll, staged.join("libLiteRt.dll")).expect("Staging libLiteRt.dll failed");
        }
    }
    let attributes =
        tauri_build::Attributes::new().plugins(tools.iter().map(|tool| (*tool, plugin(tool, &os))));
    if let Err(error) = tauri_build::try_build(attributes) {
        panic!("{error:#}");
    }
}
