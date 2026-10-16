use super::*;
#[test]
fn hides_extended_windows_prefix_and_uses_unambiguous_separators() {
    assert_eq!(
        display_root(Path::new(r"\\?\E:\IsctCompanion")),
        "E:/IsctCompanion"
    );
    assert_eq!(
        display_root(Path::new(r"\\?\UNC\server\courses")),
        "//server/courses"
    );
}
