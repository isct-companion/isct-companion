/// The desktop tools, in page order, and the systems each is built for: the
/// one place a tool's platforms are named. The code tests the tool, not the
/// system (`#[cfg(tool = "campus-printer")]`), and the page lists what the
/// binary reports (`__ISCT_COMPANION_TOOLS__`).
const TOOLS: &[(&str, &[&str])] = &[
    ("campus-wifi", &["macos"]),
    ("campus-vpn", &["macos", "windows"]),
    ("campus-printer", &["macos", "windows"]),
];

/// The campus printer plugin's commands on `os`, and which of them the main
/// window may call (`campus-printer:default`). The Windows print window has
/// its own grant (`capabilities/campus-print.json`).
fn campus_printer(os: &str) -> tauri_build::InlinedPlugin {
    use tauri_build::{DefaultPermissionRule, InlinedPlugin};
    let (commands, main): (&'static [&'static str], &'static [&'static str]) = match os {
        "windows" => (
            &["status", "add_menu", "print_request", "print"],
            &["allow-status", "allow-add-menu"],
        ),
        _ => (&["status", "install"], &["allow-status", "allow-install"]),
    };
    InlinedPlugin::new()
        .commands(commands)
        .default_permission(DefaultPermissionRule::Allow(
            main.iter()
                .map(|permission| permission.to_string())
                .collect(),
        ))
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

    // Mirrors app-ocr's default backend support: no prebuilt ONNX Runtime
    // exists for Intel macOS, so the scan command is unavailable there.
    println!("cargo::rustc-check-cfg=cfg(matrix_ocr)");
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if !(os == "macos" && arch == "x86_64") {
        println!("cargo::rustc-cfg=matrix_ocr");
    }
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
    let mut attributes = tauri_build::Attributes::new();
    if tools.contains(&"campus-printer") {
        attributes = attributes.plugin("campus-printer", campus_printer(&os));
    }
    if let Err(error) = tauri_build::try_build(attributes) {
        panic!("{error:#}");
    }
}
