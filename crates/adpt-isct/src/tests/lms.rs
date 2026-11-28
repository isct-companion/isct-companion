//! LMS activity pages are a course's own, so none is committed. The
//! `lms_module_captures` test reads pages saved from a signed-in account from
//! the directory in `LMS_MODULE_CAPTURES`, kept outside the repository, and
//! is compiled only when that directory exists, and a file
//! missing from it fails its test (`build.rs`): for
//! each of `resource`, `folder` and `url`, `<module>.html` is one activity
//! page (`https://lms.s.isct.ac.jp/<year>/mod/<module>/view.php?id=<cmid>`,
//! saved with view-source) and `<module>.url` holds that address on one
//! line. The resource must be one Moodle shows on its page (embedded or
//! "click to open"), not one it answers with the file itself.
//!
//! `LMS_MODULE_CAPTURES=/path/to/dir cargo test -p adpt-isct --lib lms::tests`
use super::*;

/// Only the LMS's exact origin is the LMS: any academic year below it, never
/// a lookalike host, another scheme or port, or an address carrying a user.
#[test]
fn the_lms_origin_is_matched_exactly() {
    assert!(lms_url(
        "https://lms.s.isct.ac.jp/2026/mod/resource/view.php?id=42"
    ));
    assert!(lms_url("https://lms.s.isct.ac.jp/2025/my/"));
    for other in [
        "https://evil.lms.s.isct.ac.jp/2026/mod/resource/view.php?id=42",
        "https://lms.s.isct.ac.jp.evil.example/2026/",
        "http://lms.s.isct.ac.jp/2026/",
        "https://lms.s.isct.ac.jp:8443/2026/",
        "https://user@lms.s.isct.ac.jp/2026/",
    ] {
        assert!(!lms_url(other), "{other}");
    }
}

#[cfg(lms_module_captures)]
fn capture(name: &str) -> String {
    let path = std::path::Path::new(env!("LMS_MODULE_CAPTURES")).join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// Each module page resolves to what a download fetches: the resource to
/// one LMS file, the folder to its LMS files, the URL to its target. The
/// same page answered from a lookalike host resolves to nothing.
#[test]
#[cfg(lms_module_captures)]
fn captured_module_pages_resolve_to_lms_files_and_links() {
    let lms_file = |raw: &str| Url::parse(raw).is_ok_and(|url| is_plugin_file_url(&url));
    for module in ["resource", "folder", "url"] {
        let html = capture(&format!("{module}.html"));
        let page = Url::parse(capture(&format!("{module}.url")).trim()).unwrap();
        assert!(is_lms_origin(&page), "{module}.url is not an LMS page");
        let resolved = parse_resource_page(&html, &page, module);
        match (module, &resolved) {
            ("resource", ResolvedResource::File { url }) => assert!(lms_file(url), "{url}"),
            ("folder", ResolvedResource::Folder { files }) => {
                assert!(files.iter().all(|file| lms_file(&file.url)), "{files:?}");
            }
            ("url", ResolvedResource::External { url }) => {
                assert!(
                    Url::parse(url).is_ok_and(|url| url.scheme() == "https"),
                    "{url}"
                );
            }
            ("url", ResolvedResource::File { url }) => assert!(lms_file(url), "{url}"),
            _ => panic!("{module}: {resolved:?}"),
        }
        let mut lookalike = page.clone();
        lookalike.set_host(Some("evil.lms.s.isct.ac.jp")).unwrap();
        assert_eq!(
            parse_resource_page(&html, &lookalike, module),
            ResolvedResource::Unresolved
        );
    }
}
