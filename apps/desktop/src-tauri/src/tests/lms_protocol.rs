use super::*;

#[test]
fn range_preflight_is_allowed() {
    let request = http::Request::builder()
        .method(http::Method::OPTIONS)
        .header(http::header::ACCESS_CONTROL_REQUEST_HEADERS, "range")
        .body(Vec::new())
        .unwrap();
    let response = preflight(&request).expect("OPTIONS is a preflight request");
    assert_eq!(response.status(), http::StatusCode::NO_CONTENT);
    assert_eq!(
        response
            .headers()
            .get(http::header::ACCESS_CONTROL_ALLOW_METHODS),
        Some(&http::HeaderValue::from_static("GET, OPTIONS"))
    );
    assert_eq!(
        response
            .headers()
            .get(http::header::ACCESS_CONTROL_ALLOW_HEADERS),
        Some(&http::HeaderValue::from_static("Range"))
    );
}

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
    assert_eq!(response.body(), b"tail");
    fs::remove_file(path).unwrap();
}

#[test]
fn serves_exact_partial_file_bytes() {
    let path = std::env::temp_dir().join(format!("lms-range-{}.pdf", uuid::Uuid::new_v4()));
    fs::write(&path, b"0123456789").unwrap();
    let range = http::HeaderValue::from_static("bytes=3-6");
    let response = serve(&path, Some(&range), "digest");
    assert_eq!(response.status(), http::StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        response.headers()[http::header::CONTENT_RANGE],
        "bytes 3-6/10"
    );
    assert_eq!(response.headers()[http::header::ACCEPT_RANGES], "bytes");
    assert_eq!(response.body(), b"3456");
    fs::remove_file(path).unwrap();
}
