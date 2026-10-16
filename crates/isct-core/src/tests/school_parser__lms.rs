use super::*;

#[test]
fn resource_page_accepts_any_academic_year_on_the_exact_lms_origin() {
    let page = Url::parse("https://lms.s.isct.ac.jp/2026/mod/resource/view.php?id=42").unwrap();
    let result = parse_resource_page(
        r#"<main><a href="/2026/pluginfile.php/1/mod_resource/content/0/file.pdf">File</a></main>"#,
        &page,
        "resource",
    );
    assert_eq!(
        result,
        ResolvedResource::File {
            url: "https://lms.s.isct.ac.jp/2026/pluginfile.php/1/mod_resource/content/0/file.pdf"
                .into()
        }
    );
}

#[test]
fn resource_page_rejects_lookalike_origins() {
    let page =
        Url::parse("https://evil.lms.s.isct.ac.jp/2026/mod/resource/view.php?id=42").unwrap();
    assert_eq!(
        parse_resource_page(
            r#"<a href="/2026/pluginfile.php/1/mod_resource/content/0/file.pdf">File</a>"#,
            &page,
            "resource"
        ),
        ResolvedResource::Unresolved
    );
}

#[test]
fn resource_page_parses_embed_folder_and_external_url_shapes() {
    let resource = Url::parse("https://lms.s.isct.ac.jp/2026/mod/resource/view.php?id=42").unwrap();
    assert!(matches!(
        parse_resource_page(
            r#"<div class="resourcecontent"><object data="/2026/pluginfile.php/1/object.pdf"></object></div>"#,
            &resource,
            "resource"
        ),
        ResolvedResource::File { url } if url.ends_with("/2026/pluginfile.php/1/object.pdf")
    ));

    let folder = Url::parse("https://lms.s.isct.ac.jp/2026/mod/folder/view.php?id=43").unwrap();
    assert!(matches!(
        parse_resource_page(
            r#"<div class="foldertree"><a href="/2026/pluginfile.php/1/a.txt">A</a><a href="/2026/pluginfile.php/1/b.txt">B</a></div>"#,
            &folder,
            "folder"
        ),
        ResolvedResource::Folder { files } if files.len() == 2
    ));

    let external = Url::parse("https://lms.s.isct.ac.jp/2026/mod/url/view.php?id=44").unwrap();
    assert_eq!(
        parse_resource_page(
            r#"<main id="region-main"><a href="https://example.edu/reading">Reading</a></main>"#,
            &external,
            "url"
        ),
        ResolvedResource::External {
            url: "https://example.edu/reading".into()
        }
    );
}
