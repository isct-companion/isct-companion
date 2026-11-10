//! Builds the two root programs of the CUPS queue for the target being
//! built, macOS only: `titech-lpd`, the backend, then the
//! installer, which embeds it. The library embeds the installer
//! (`INSTALLER`), so the app never downloads code that runs as root. They
//! use std only, so rustc builds them directly: a nested cargo would wait on
//! the target directory lock the outer build holds.
use std::{env, ffi::OsString, path::Path, path::PathBuf, process::Command};

fn main() {
    for path in [
        "programs",
        "src",
        "assets",
        "../adpt-printer/src/endpoint.rs",
        "../adpt-printer/src/student_id.rs",
    ] {
        println!("cargo::rerun-if-changed={path}");
    }
    println!("cargo::rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if os != "macos" {
        return;
    }
    let target = env::var("TARGET").expect("cargo sets TARGET");
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    // A cross build names its linker this way; rustc alone would not see it.
    let linker_variable = format!(
        "CARGO_TARGET_{}_LINKER",
        target.to_uppercase().replace(['-', '.'], "_")
    );
    println!("cargo::rerun-if-env-changed={linker_variable}");
    let linker = env::var_os(&linker_variable);

    let backend = out.join("titech-lpd");
    compile(
        &target,
        linker.as_ref(),
        &manifest.join("programs/titech_lpd.rs"),
        &backend,
        None,
    );
    compile(
        &target,
        linker.as_ref(),
        &manifest.join("programs/installer.rs"),
        &out.join("institute-printer-installer"),
        Some(&backend),
    );
}

/// One std-only program from `source` to `output`; `backend` is the
/// backend the installer embeds (`TITECH_LPD`).
fn compile(
    target: &str,
    linker: Option<&OsString>,
    source: &Path,
    output: &Path,
    backend: Option<&Path>,
) {
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let mut command = Command::new(rustc);
    command
        .args(["--edition", "2021", "--crate-type", "bin"])
        .args([
            "-C",
            "opt-level=s",
            "-C",
            "panic=abort",
            "-C",
            "strip=symbols",
        ])
        .args(["--target", target])
        .arg("-o")
        .arg(output)
        .arg(source);
    if let Some(linker) = linker {
        let mut flag = OsString::from("linker=");
        flag.push(linker);
        command.arg("-C").arg(flag);
    }
    if let Some(backend) = backend {
        command.env("TITECH_LPD", backend);
    }
    let status = command.status().expect("rustc runs");
    assert!(status.success(), "{} did not build", source.display());
}
