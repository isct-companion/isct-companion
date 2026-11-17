use super::*;

fn photo(folder: &Path, name: &str, width: u32, height: u32) -> ExamUploadFile {
    let path = folder.join(name);
    image::RgbImage::from_pixel(width, height, image::Rgb([200, 40, 40]))
        .save(&path)
        .unwrap();
    ExamUploadFile {
        path: path.to_string_lossy().into_owned(),
        name: name.into(),
    }
}

/// The site refuses a photo declared as JPEG, so one photo goes up as a PDF
/// named after it and keyed by the photo's own bytes, as the site's uploader
/// keys it; a tall photo is cut into pages no taller than 2048.
#[test]
fn one_photo_goes_up_as_a_pdf_keyed_by_the_photo() {
    let folder = std::env::temp_dir().join(format!("exam-upload-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = photo(&folder, "past exam.png", 800, 5000);
    let original = std::fs::read(&file.path).unwrap();
    let prepared = prepare(std::slice::from_ref(&file), &CancellationToken::new()).unwrap();
    assert_eq!(prepared.name, "past exam.pdf");
    assert_eq!(prepared.content_type, "application/pdf");
    assert!(prepared.bytes.starts_with(b"%PDF-"));
    assert_eq!(
        prepared.hash,
        data_encoding::HEXLOWER.encode(&Sha256::digest(&original))
    );
    let pages = String::from_utf8_lossy(&prepared.bytes)
        .matches("/Type /Page\n")
        .count()
        + String::from_utf8_lossy(&prepared.bytes)
            .matches("/Type /Page ")
            .count();
    assert!(
        pages >= 3,
        "a 5000px photo spans several pages, got {pages}"
    );

    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        prepare(&[file], &cancelled),
        Err(ExamError::Cancelled)
    ));
    std::fs::remove_dir_all(folder).unwrap();
}
