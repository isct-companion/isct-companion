use super::*;

#[test]
fn parses_single_ranges() {
    let value = http::HeaderValue::from_static("bytes=2-5");
    assert_eq!(parse_range(Some(&value), 10), Ok(Some(2..=5)));
    let suffix = http::HeaderValue::from_static("bytes=-3");
    assert_eq!(parse_range(Some(&suffix), 10), Ok(Some(7..=9)));
    let multiple = http::HeaderValue::from_static("bytes=0-1,4-5");
    assert!(parse_range(Some(&multiple), 10).is_err());
}

#[test]
fn reads_a_small_range_from_a_large_sparse_file() {
    let path = std::env::temp_dir().join(format!("lms-large-range-{}.pdf", uuid::Uuid::new_v4()));
    let mut file = fs::File::create(&path).unwrap();
    let offset = 3_u64 * 1024 * 1024 * 1024;
    file.set_len(offset + 4).unwrap();
    file.seek(SeekFrom::Start(offset)).unwrap();
    std::io::Write::write_all(&mut file, b"tail").unwrap();
    drop(file);
    let range = http::HeaderValue::from_str(&format!("bytes={offset}-")).unwrap();
    let response = serve(&path, Some(&range), "digest");
    assert_eq!(response.status(), http::StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        response.headers()[http::header::CONTENT_RANGE],
        format!("bytes {offset}-{}/{}", offset + 3, offset + 4).as_str()
    );
    assert_eq!(response.headers()[http::header::ACCEPT_RANGES], "bytes");
    assert_eq!(response.body(), b"tail");
    fs::remove_file(path).unwrap();
}
