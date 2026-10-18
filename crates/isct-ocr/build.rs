fn main() {
    println!("cargo::rustc-check-cfg=cfg(matrix_ocr)");

    let onnx = std::env::var_os("CARGO_FEATURE_ONNX").is_some();
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let intel_apple = (os == "macos" || os == "ios") && arch == "x86_64";
    if onnx && !(intel_apple || os == "android" && arch != "aarch64") {
        println!("cargo::rustc-cfg=matrix_ocr");
        println!("cargo::rerun-if-env-changed=ISCT_MATRIX_ALPHABET_MODEL");
        let model = std::env::var_os("ISCT_MATRIX_ALPHABET_MODEL")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("models/alphabet/alphabet-printed.onnx"));
        println!("cargo::rerun-if-changed={}", model.display());
        let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo output"));
        std::fs::copy(model, output.join("matrix-alphabet.onnx"))
            .expect("Alphabet model unavailable");
    }
}
