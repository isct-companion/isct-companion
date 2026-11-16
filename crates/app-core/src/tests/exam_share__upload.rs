use super::*;
use image::{ImageFormat, Rgba, RgbaImage};

fn draft(role: &str) -> ExamUploadDraft {
    ExamUploadDraft {
        files: Vec::new(),
        subject_codes: vec!["CSC.T252".into()],
        teacher_ids: vec!["t1".into(), "t2".into(), "t1".into()],
        academic_year: Some(2024),
        occasion: Some("quiz".into()),
        occasion_number: 3,
        variant: "  A  ".into(),
        role: role.into(),
        provenance: Some("original".into()),
        answer_source: None,
        note: "  ".into(),
        request_id: None,
    }
}

/// The site's rules: only quizzes, checks and reports keep a number; notes
/// belong to no sitting; a question needs its provenance; an answer
/// without a source is "unknown"; teachers are deduplicated; text is trimmed
/// and an empty note is null.
#[test]
fn fields_follow_the_sites_rules() {
    let quiz = fields(&draft("question")).expect("quiz");
    assert_eq!(quiz["occasionNumber"], 3);
    assert_eq!(quiz["variant"], "A");
    assert_eq!(quiz["teacherIds"], json!(["t1", "t2"]));
    assert_eq!(quiz["note"], Value::Null);
    assert_eq!(quiz["answerSource"], Value::Null);

    let mut final_exam = draft("answer");
    final_exam.occasion = Some("final".into());
    final_exam.provenance = None;
    let answer = fields(&final_exam).expect("answer");
    assert_eq!(answer["occasionNumber"], 0);
    assert_eq!(answer["provenance"], "original");
    assert_eq!(answer["answerSource"], "unknown");

    let mut notes = draft("notes");
    notes.occasion = None;
    let notes = fields(&notes).expect("notes");
    assert_eq!(
        (
            &notes["occasion"],
            &notes["variant"],
            &notes["occasionNumber"]
        ),
        (&json!("none"), &json!(""), &json!(0))
    );

    for broken in [
        ExamUploadDraft {
            provenance: None,
            ..draft("question")
        },
        ExamUploadDraft {
            occasion: None,
            ..draft("answer")
        },
        ExamUploadDraft {
            subject_codes: vec![" ".into()],
            ..draft("answer")
        },
        ExamUploadDraft {
            note: "x".repeat(41),
            ..draft("answer")
        },
        draft("summary"),
    ] {
        assert!(matches!(fields(&broken), Err(ExamError::InvalidInput)));
    }
}

fn encoded(image: &RgbaImage, format: ImageFormat) -> Vec<u8> {
    let mut bytes = Vec::new();
    // JPEG has no alpha channel.
    let image = match format {
        ImageFormat::Jpeg => {
            DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(image.clone()).to_rgb8())
        }
        _ => DynamicImage::ImageRgba8(image.clone()),
    };
    image
        .write_to(&mut Cursor::new(&mut bytes), format)
        .expect("encode");
    bytes
}

fn page_boxes(pdf: &[u8]) -> Vec<(i64, i64)> {
    let document = Document::load_mem(pdf).expect("a readable PDF");
    document
        .get_pages()
        .values()
        .map(|id| {
            let page = document.get_dictionary(*id).expect("page");
            let media = page
                .get(b"MediaBox")
                .and_then(Object::as_array)
                .expect("box");
            (
                media[2].as_i64().expect("width"),
                media[3].as_i64().expect("height"),
            )
        })
        .collect()
}

/// Photos merge like the site's uploader: a wide, tall photo is scaled to
/// 1600 px and cut into pages at most 2048 px tall; a small one keeps its
/// size; a transparent PNG lands on white rather than black.
#[test]
fn merges_photos_into_page_sized_slices() {
    let tall = RgbaImage::from_pixel(3000, 5000, Rgba([20, 20, 20, 255]));
    let clear = RgbaImage::from_pixel(800, 600, Rgba([0, 0, 0, 0]));
    let pdf = merge(&[
        encoded(&tall, ImageFormat::Jpeg),
        encoded(&clear, ImageFormat::Png),
    ])
    .expect("merged");
    // 5000 px at 1600/3000 is 2667 px: one full 2048 px page and the rest.
    assert_eq!(page_boxes(&pdf), [(1600, 2048), (1600, 619), (800, 600)]);

    let document = Document::load_mem(&pdf).expect("PDF");
    let last = *document.get_pages().values().last().expect("page");
    let picture = document
        .get_page_resources(last)
        .ok()
        .and_then(|(resources, _)| resources.cloned())
        .and_then(|resources| resources.get(b"XObject").ok().cloned())
        .and_then(|xobjects| xobjects.as_dict().ok().cloned())
        .and_then(|xobjects| xobjects.get(b"Im0").ok().cloned())
        .and_then(|id| id.as_reference().ok())
        .and_then(|id| document.get_object(id).ok())
        .and_then(|object| object.as_stream().ok())
        .expect("image stream");
    let decoded = image::load_from_memory(&picture.content).expect("JPEG");
    let pixel = decoded.to_rgb8().get_pixel(400, 300).0;
    assert!(pixel.iter().all(|channel| *channel > 245), "{pixel:?}");
}

fn scratch(name: &str, bytes: &[u8]) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("exam-upload-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&folder).expect("folder");
    let path = folder.join(name);
    std::fs::write(&path, bytes).expect("write");
    path
}

/// A PDF goes up as picked and photos as a PDF; several files must all be
/// photos; types the site does not store, empty files and files over 25 MB
/// are refused before any request.
#[test]
fn prepares_only_what_the_site_stores() {
    let photo = encoded(
        &RgbaImage::from_pixel(10, 10, Rgba([0, 0, 0, 255])),
        ImageFormat::Png,
    );
    let pdf = scratch("exam.PDF", b"%PDF-1.4\n");
    let single = prepare(std::slice::from_ref(&pdf)).expect("single");
    assert_eq!(
        (single.name.as_str(), single.content_type),
        ("exam.PDF", "application/pdf")
    );

    // One photo becomes a PDF named after it, deduplicated on the photo.
    let one = scratch("IMG_0001.JPG", &photo);
    let single = prepare(std::slice::from_ref(&one)).expect("photo");
    assert_eq!(
        (single.name.as_str(), single.content_type),
        ("IMG_0001.pdf", "application/pdf")
    );
    assert!(single.bytes.starts_with(b"%PDF"));
    assert_eq!(single.hash, sha256_hex(&photo));

    let photos = [scratch("a.png", &photo), scratch("b.png", &photo)];
    let merged = prepare(&photos).expect("photos");
    assert_eq!(
        (merged.name.as_str(), merged.content_type),
        ("merged_document.pdf", "application/pdf")
    );

    let mixed = [photos[0].clone(), pdf.clone()];
    assert!(matches!(prepare(&mixed), Err(ExamError::InvalidInput)));
    let word = scratch("exam.docx", b"PK");
    assert!(matches!(prepare(&[word]), Err(ExamError::InvalidInput)));
    let empty = scratch("empty.txt", b"");
    assert!(matches!(prepare(&[empty]), Err(ExamError::InvalidInput)));
    let huge = scratch("huge.txt", &vec![b'a'; LIMIT + 1]);
    assert!(matches!(prepare(&[huge]), Err(ExamError::TooLarge)));
}
