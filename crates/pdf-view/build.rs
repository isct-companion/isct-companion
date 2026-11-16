use std::io::Read;
use std::path::{Path, PathBuf};

/// PDFium from bblanchon/pdfium-binaries, the build pdfium-render's
/// `pdfium_8125` feature binds against. Each archive is pinned by digest.
const PDFIUM_RELEASE: &str = "chromium/8125";
/// (target, archive, library inside it, file name beside the app, sha256)
const BUILDS: &[(&str, &str, &str, &str, &str)] = &[
    (
        "aarch64-apple-darwin",
        "pdfium-mac-arm64.tgz",
        "lib/libpdfium.dylib",
        "libpdfium.dylib",
        "4939bd935ac83ddc09a3a4f038bc43a19cea1660316a3667f26cb07a070e139d",
    ),
    (
        "x86_64-apple-darwin",
        "pdfium-mac-x64.tgz",
        "lib/libpdfium.dylib",
        "libpdfium.dylib",
        "8a5193535b80c7215d8932f6647ae5097465b6dba2811602c21e120c2af112b0",
    ),
    (
        "x86_64-pc-windows-msvc",
        "pdfium-win-x64.tgz",
        "bin/pdfium.dll",
        "pdfium.dll",
        "9a964261760cff52570aabf038ab346d3a5034dd8b30d0d5fe8861235d356b8b",
    ),
];

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Fetches the pinned archive once (or reads `ISCT_PDFIUM_ARCHIVE` for an
/// offline build), checks its digest, and unpacks the library and its
/// licences into OUT_DIR. A copy goes beside this profile's binaries so
/// `cargo run`, tests and examples load it the way the installed app does.
/// Dependents read the paths from `DEP_PDFIUM_LIB` and `DEP_PDFIUM_LICENSES`.
fn main() {
    use sha2::{Digest, Sha256};
    println!("cargo::rerun-if-env-changed=ISCT_PDFIUM_ARCHIVE");
    let target = std::env::var("TARGET").expect("Cargo target");
    let Some(&(_, archive, inside, name, digest)) = BUILDS.iter().find(|build| build.0 == target)
    else {
        panic!("pdf-view is desktop-only: no PDFium build for {target}");
    };
    let output = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo output"));
    let library = output.join(name);
    let licenses = output.join("pdfium-licenses");
    let stamp = output.join("pdfium.sha256");
    if std::fs::read_to_string(&stamp).ok().as_deref() != Some(digest) || !library.is_file() {
        let bytes = match std::env::var_os("ISCT_PDFIUM_ARCHIVE") {
            Some(path) => std::fs::read(path).expect("ISCT_PDFIUM_ARCHIVE is unreadable"),
            None => {
                let url = format!(
                    "https://github.com/bblanchon/pdfium-binaries/releases/download/{PDFIUM_RELEASE}/{archive}"
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
        assert_eq!(
            hex(&Sha256::digest(&bytes)),
            digest,
            "Unexpected PDFium archive"
        );
        let _ = std::fs::remove_dir_all(&licenses);
        std::fs::create_dir_all(&licenses).expect("Creating the licence folder failed");
        let mut entries = tar::Archive::new(flate2::read::GzDecoder::new(&bytes[..]));
        for entry in entries
            .entries()
            .expect("Reading the PDFium archive failed")
        {
            let mut entry = entry.expect("Reading the PDFium archive failed");
            let path = entry.path().expect("PDFium archive path").into_owned();
            let destination = if path == Path::new(inside) {
                library.clone()
            } else if path == Path::new("LICENSE") {
                licenses.join("pdfium.txt")
            } else if path.starts_with("licenses") && path.extension().is_some() {
                licenses.join(path.file_name().expect("Licence file name"))
            } else {
                continue;
            };
            let mut contents = Vec::new();
            entry
                .read_to_end(&mut contents)
                .expect("Reading the PDFium archive failed");
            std::fs::write(destination, contents).expect("Unpacking PDFium failed");
        }
        assert!(library.is_file(), "{inside} missing from {archive}");
        std::fs::write(&stamp, digest).expect("Writing the PDFium stamp failed");
    }
    // OUT_DIR is <target>/<profile>/build/<package>-<hash>/out.
    if let Some(profile) = output.ancestors().nth(3) {
        for directory in [
            profile.to_path_buf(),
            profile.join("deps"),
            profile.join("examples"),
        ] {
            if directory.is_dir() {
                let _ = std::fs::copy(&library, directory.join(name));
            }
        }
    }
    println!("cargo::metadata=lib={}", library.display());
    println!("cargo::metadata=licenses={}", licenses.display());
}
