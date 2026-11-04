//! Builds the root CUPS helper (`src/cups/helper.rs`) for the target being
//! built, so the library can embed it (`cups::HELPER`) and the app never
//! downloads code that runs as root. The helper uses std only, so rustc
//! builds it directly: a nested cargo would wait on the target directory
//! lock the outer build holds.
use std::{env, path::PathBuf, process::Command};

fn main() {
    for path in ["src/cups", "src/student_id.rs", "src/endpoint.rs", "assets"] {
        println!("cargo::rerun-if-changed={path}");
    }
    println!("cargo::rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if os != "macos" && os != "linux" {
        return;
    }
    let target = env::var("TARGET").expect("cargo sets TARGET");
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"))
        .join("institute-printer-helper");
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());

    let mut command = Command::new(rustc);
    command
        .args(["--edition", "2021", "--crate-type", "bin"])
        .args(["--crate-name", "institute_printer_helper"])
        .args([
            "-C",
            "opt-level=s",
            "-C",
            "panic=abort",
            "-C",
            "strip=symbols",
        ])
        .args(["--target", &target])
        .arg("-o")
        .arg(&out)
        .arg(manifest.join("src/cups/helper.rs"));
    // A cross build names its linker this way; rustc alone would not see it.
    let linker = format!(
        "CARGO_TARGET_{}_LINKER",
        target.to_uppercase().replace(['-', '.'], "_")
    );
    println!("cargo::rerun-if-env-changed={linker}");
    if let Some(linker) = env::var_os(&linker) {
        command.arg("-C").arg({
            let mut flag = std::ffi::OsString::from("linker=");
            flag.push(linker);
            flag
        });
    }
    let status = command.status().expect("rustc runs");
    assert!(status.success(), "the CUPS helper did not build");
}
