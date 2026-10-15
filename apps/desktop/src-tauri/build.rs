fn main() {
    // Mirrors isct-ocr's default backend support: no prebuilt ONNX Runtime
    // exists for Intel macOS, so the scan command is unavailable there.
    println!("cargo::rustc-check-cfg=cfg(matrix_ocr)");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if !(os == "macos" && arch == "x86_64") {
        println!("cargo::rustc-cfg=matrix_ocr");
    }
    tauri_build::build()
}
