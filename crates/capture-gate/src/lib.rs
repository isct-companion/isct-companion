//! The build-script half of the tests that read files kept outside the
//! repository: pages saved from a real account, a private camera frame, the
//! published campus map. Each group of such tests sits behind a cfg that is on
//! only when its variable is set and names a directory or file that exists, so
//! a build without them does not compile the tests at all. Which files a
//! directory holds is the tests' business: a missing one fails its test. The
//! tests take the path with `env!`, which makes a relative path relative to
//! the crate, as it is here. Without the variable nothing here touches the
//! filesystem.
//!
//! Call these from a crate's `build.rs`, for every group the crate has, on
//! every build: each one also registers its cfg and the variable.

use std::ffi::OsString;
use std::path::Path;

/// Turns `cfg` on when `var` names a directory that exists.
pub fn dir(cfg: &str, var: &str) {
    if let Some(path) = declare(cfg, var) {
        enable_if(cfg, var, Path::new(&path), Path::is_dir);
    }
}

/// Turns `cfg` on when `var` names a file that exists.
pub fn file(cfg: &str, var: &str) {
    if let Some(path) = declare(cfg, var) {
        enable_if(cfg, var, Path::new(&path), Path::is_file);
    }
}

/// Turns `cfg` on when `var` is set, for a value such as an id that a test
/// reads with `env!` beside its files.
pub fn value(cfg: &str, var: &str) {
    if declare(cfg, var).is_some() {
        println!("cargo::rustc-cfg={cfg}");
    }
}

fn declare(cfg: &str, var: &str) -> Option<OsString> {
    println!("cargo::rustc-check-cfg=cfg({cfg})");
    println!("cargo::rerun-if-env-changed={var}");
    std::env::var_os(var)
}

fn enable_if(cfg: &str, var: &str, path: &Path, exists: fn(&Path) -> bool) {
    // A path that appears or changes later reruns the build script.
    println!("cargo::rerun-if-changed={}", path.display());
    if exists(path) {
        println!("cargo::rustc-cfg={cfg}");
    } else {
        println!(
            "cargo::warning={var} is set, but {} does not exist: the `{cfg}` tests are not compiled",
            path.display()
        );
    }
}
