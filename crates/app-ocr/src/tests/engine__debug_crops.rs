use super::*;
#[test]
fn exports_under_the_supplied_debug_directory() {
    let root = std::env::temp_dir().join(format!("matrix-debug-test-{}", uuid::Uuid::new_v4()));
    save(
        &root,
        vec![Crop::from_png(
            "A1.png".into(),
            b"\x89PNG\r\n\x1a\n".to_vec(),
        )],
    )
    .unwrap();
    let directories = fs::read_dir(&root)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(directories.len(), 1);
    assert!(directories[0]
        .file_name()
        .to_string_lossy()
        .starts_with("matrix-"));
    assert!(directories[0].path().join("A1.png").is_file());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn only_accepts_cell_png_names() {
    for name in ["../a.png", "K1.png", "A8.png", "results.json"] {
        assert!(decode(Crop {
            name: name.into(),
            data: STANDARD.encode(b"\x89PNG\r\n\x1a\n")
        })
        .is_err());
    }
    assert!(decode(Crop {
        name: "A1.png".into(),
        data: STANDARD.encode(b"\x89PNG\r\n\x1a\n")
    })
    .is_ok());
}
