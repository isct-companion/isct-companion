use std::path::{Path, PathBuf};

/// Google's prebuilt LiteRT runtime for Windows x86_64
/// (https://ai.google.dev/edge/litert/next/cpp_sdk). It links the C runtime
/// statically and exports the TFLite C API `engine/letters/litert.rs` uses.
const LITERT_VERSION: &str = "2.3.1";
const LITERT_WINDOWS_SHA256: &str =
    "30fcb695ce8c2d8da5416337b6f4dc4407f04823b331cf6782a4894a82ee2ebb";

fn main() {
    println!("cargo::rustc-check-cfg=cfg(matrix_ocr)");

    let onnx = std::env::var_os("CARGO_FEATURE_ONNX").is_some();
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if onnx && !(os == "android" && arch != "aarch64") {
        println!("cargo::rustc-cfg=matrix_ocr");
        println!("cargo::rerun-if-env-changed=ISCT_MATRIX_ALPHABET_MODEL");
        let output = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo output"));
        // Each runtime embeds its export of the same trained weights
        // (models/alphabet/README.md): Core ML's float16 ML Program on macOS
        // and iOS, LiteRT with float16 weights on Android and Windows, the
        // float source itself with ONNX Runtime elsewhere.
        let (format, file) = match os.as_str() {
            "macos" | "ios" => ("mlmodel", "alphabet-printed-fp16.mlmodel"),
            "android" | "windows" => ("tflite", "alphabet-printed-fp16.tflite"),
            _ => ("onnx", "alphabet-printed.onnx"),
        };
        let model = std::env::var_os("ISCT_MATRIX_ALPHABET_MODEL")
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new("models/alphabet").join(file));
        println!("cargo::rerun-if-changed={}", model.display());
        std::fs::copy(model, output.join(format!("matrix-alphabet.{format}")))
            .expect("Alphabet model unavailable");
        if os == "windows" {
            windows_runtime(&output);
        }
    }
}

/// Fetches the pinned LiteRT DLL once (or takes `ISCT_LITERT_DLL` for an
/// offline build), checks its digest, and puts a copy beside the binaries of
/// this profile so `cargo run`, tests and examples load it the way the
/// installed app does. Dependents read its path from `DEP_LITERT_DLL`; the
/// desktop build bundles it next to the executable.
fn windows_runtime(output: &Path) {
    use sha2::{Digest, Sha256};
    println!("cargo::rerun-if-env-changed=ISCT_LITERT_DLL");
    let dll = output.join("libLiteRt.dll");
    let verified = |path: &Path| {
        std::fs::read(path)
            .is_ok_and(|bytes| format!("{:x}", Sha256::digest(&bytes)) == LITERT_WINDOWS_SHA256)
    };
    if !verified(&dll) {
        let bytes = match std::env::var_os("ISCT_LITERT_DLL") {
            Some(path) => std::fs::read(path).expect("ISCT_LITERT_DLL is unreadable"),
            None => {
                let url = format!(
                    "https://storage.googleapis.com/litert/binaries/{LITERT_VERSION}/windows_x86_64/libLiteRt.dll"
                );
                ureq::get(&url)
                    .call()
                    .and_then(|mut response| {
                        response
                            .body_mut()
                            .with_config()
                            .limit(64 << 20)
                            .read_to_vec()
                    })
                    .unwrap_or_else(|e| panic!("Downloading {url} failed: {e}"))
            }
        };
        let digest = format!("{:x}", Sha256::digest(&bytes));
        assert_eq!(digest, LITERT_WINDOWS_SHA256, "Unexpected LiteRT DLL");
        std::fs::write(&dll, bytes).expect("Writing the LiteRT DLL failed");
    }
    // OUT_DIR is <target>/<profile>/build/<package>-<hash>/out.
    if let Some(profile) = output.ancestors().nth(3) {
        for directory in [
            profile.to_path_buf(),
            profile.join("deps"),
            profile.join("examples"),
        ] {
            if directory.is_dir() {
                let _ = std::fs::copy(&dll, directory.join("libLiteRt.dll"));
            }
        }
    }
    println!("cargo::metadata=dll={}", dll.display());
}
