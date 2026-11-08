fn main() {
    // Mirrors app-ocr's default backend support: no prebuilt ONNX Runtime
    // exists for Intel macOS, so the scan command is unavailable there.
    println!("cargo::rustc-check-cfg=cfg(matrix_ocr)");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
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
    tauri_build::build()
}
