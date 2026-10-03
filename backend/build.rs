fn main() {
    // Mirrors the `ort` target condition in Cargo.toml: no prebuilt ONNX Runtime
    // exists for Intel macOS, so matrix OCR is compiled out there.
    println!("cargo::rustc-check-cfg=cfg(matrix_ocr)");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if !(os == "macos" && arch == "x86_64") {
        println!("cargo::rustc-cfg=matrix_ocr");
    }
    tauri_build::build()
}
