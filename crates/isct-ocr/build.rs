fn main() {
    println!("cargo::rustc-check-cfg=cfg(matrix_ocr)");

    let onnx = std::env::var_os("CARGO_FEATURE_ONNX").is_some();
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if onnx && !(os == "macos" && arch == "x86_64" || os == "android" && arch != "aarch64") {
        println!("cargo::rustc-cfg=matrix_ocr");
    }
}
