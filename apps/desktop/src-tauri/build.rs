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

/// Commands that each tool grants to the main window.
fn plugin(tool: &str) -> tauri_build::InlinedPlugin {
    use tauri_build::{DefaultPermissionRule, InlinedPlugin};
    let commands: &[&str] = match tool {
        "campus-printer" => &["status", "install"],
        "campus-vpn" => &["status", "connect", "disconnect", "keep", "install"],
        "campus-wifi" => &["status", "connect"],
        _ => panic!("{tool} has no plugin"),
    };
    InlinedPlugin::new()
        .commands(commands)
        .default_permission(DefaultPermissionRule::AllowAllCommands)
}

fn build_widget_bridge() {
    use std::{env, path::PathBuf, process::Command};

    let source = "../../apple-widgets/macOS/WidgetBridge.swift";
    println!("cargo::rerun-if-changed={source}");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is missing"));
    let arch = env::var("CARGO_CFG_TARGET_ARCH").expect("Target architecture is missing");
    let arch = match arch.as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        _ => unreachable!("Unsupported Mac architecture: {arch}"),
    };
    let target = format!("{arch}-apple-macos15.0");
    let status = Command::new("xcrun")
        .args([
            "--sdk",
            "macosx",
            "swiftc",
            "-target",
            &target,
            "-O",
            "-parse-as-library",
            "-emit-library",
            "-static",
            "-o",
        ])
        .arg(out.join("libisct_widget_bridge.a"))
        .arg(source)
        .status()
        .expect("Could not start the Swift compiler");
    assert!(status.success(), "Could not compile the WidgetKit bridge");
    println!("cargo::rustc-link-search=native={}", out.display());
    println!("cargo::rustc-link-lib=static=isct_widget_bridge");
    println!("cargo::rustc-link-lib=framework=WidgetKit");
}

fn main() {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if os == "macos" {
        build_widget_bridge();
    }
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
        tauri_build::Attributes::new().plugins(tools.iter().map(|tool| (*tool, plugin(tool))));
    if let Err(error) = tauri_build::try_build(attributes) {
        panic!("{error:#}");
    }
}
