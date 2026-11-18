fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rustc-check-cfg=cfg(matrix_ocr)");
    // Cargo forwards metadata from the directly linked app-ocr dependency.
    if std::env::var_os("DEP_LITERT_MATRIX_OCR").is_some() {
        println!("cargo::rustc-cfg=matrix_ocr");
    }
}
