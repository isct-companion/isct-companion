use super::*;

#[test]
#[ignore = "requires an explicitly supplied private camera frame"]
fn supplied_camera_frame_geometry() {
    let path = std::env::var("MATRIX_PHOTO").expect("Set MATRIX_PHOTO");
    let bytes = std::fs::read(path).unwrap();
    let captured =
        capture_matrix_frame(&bytes).expect("camera frame should contain a complete Matrix grid");
    eprintln!("camera frame timing: {:?}", captured.timing);
    assert_eq!(captured.quads.len(), 70);
    assert_eq!(captured.glyphs.len(), 70);
    let usable = captured
        .glyphs
        .iter()
        .filter(|glyph| {
            glyph.contrast >= 12.
                && glyph.sharpness >= 10.
                && glyph.source_size >= 10.
                && glyph.centering >= 0.5
        })
        .count();
    eprintln!("camera acquisition usable cells: {usable}/70");
    assert!(usable >= 35, "camera acquisition must start on this frame");
    if let Some(path) = std::env::var_os("MATRIX_OVERLAY") {
        let mut overlay = image::load_from_memory(&bytes).unwrap().to_rgb8();
        let (width, height) = overlay.dimensions();
        for quad in &captured.quads {
            for i in 0..4 {
                let a = quad[i];
                let b = quad[(i + 1) % 4];
                imageproc::drawing::draw_line_segment_mut(
                    &mut overlay,
                    (a[0] as f32 * width as f32, a[1] as f32 * height as f32),
                    (b[0] as f32 * width as f32, b[1] as f32 * height as f32),
                    image::Rgb([255, 0, 0]),
                );
            }
        }
        overlay.save(path).unwrap();
    }
    let model = MatrixAlphabetService::default();
    let readings = captured
        .glyphs
        .chunks(10)
        .flat_map(|batch| {
            model
                .recognize(
                    &batch
                        .iter()
                        .map(|glyph| glyph.gray.clone())
                        .collect::<Vec<_>>(),
                )
                .unwrap()
        })
        .collect::<Vec<_>>();
    let clear = readings
        .iter()
        .filter(|reading| !reading.text.is_empty() && reading.confidence >= 0.8)
        .count();
    eprintln!("camera model clear cells: {clear}/70");
    assert!(
        clear >= 35,
        "camera frame must make at least half of the cells readable"
    );

    let source = image::load_from_memory(&bytes).unwrap().to_rgb8();
    for rotated in [
        image::imageops::rotate90(&source),
        image::imageops::rotate180(&source),
        image::imageops::rotate270(&source),
    ] {
        let mut encoded = std::io::Cursor::new(Vec::new());
        rotated
            .write_to(&mut encoded, image::ImageFormat::Jpeg)
            .unwrap();
        assert_eq!(
            capture_matrix_frame(encoded.get_ref()).unwrap().quads.len(),
            70
        );
    }
}

#[test]
fn capture_keeps_gray_texture_and_classifier_preserves_cell_order_and_batch_boundaries() {
    let fixture = include_bytes!("fixtures/matrix-card.jpg");
    let captured = capture_matrix_frame(fixture).unwrap();
    assert_eq!(captured.glyphs.len(), 70);
    assert_eq!(captured.quads.len(), 70);
    assert!(captured.glyphs.iter().all(|g| g.gray.len() == 96 * 96
        && g.contrast > 0.
        && g.sharpness > 0.
        && g.source_size > 0.));
    assert!(captured
        .glyphs
        .iter()
        .flat_map(|g| &g.gray)
        .any(|&v| v > 0 && v < 255));
    let model = MatrixAlphabetService::default();
    let images = captured
        .glyphs
        .into_iter()
        .map(|g| g.gray)
        .collect::<Vec<_>>();
    for (start, batch) in images.chunks(10).enumerate() {
        for (offset, actual) in model.recognize(batch).unwrap().iter().enumerate() {
            let expected = ((b'A' + ((start * 10 + offset) % 26) as u8) as char).to_string();
            assert_eq!(actual.text, expected, "cell {}", start * 10 + offset);
            assert!(actual.confidence >= 0.8, "{actual:?}");
        }
    }
    let source = image::load_from_memory(fixture).unwrap().to_rgb8();
    for (turn, rotated) in [
        image::imageops::rotate90(&source),
        image::imageops::rotate180(&source),
        image::imageops::rotate270(&source),
    ]
    .into_iter()
    .enumerate()
    {
        let mut encoded = std::io::Cursor::new(Vec::new());
        rotated
            .write_to(&mut encoded, image::ImageFormat::Jpeg)
            .unwrap();
        let rotated = capture_matrix_frame(encoded.get_ref()).unwrap();
        for (start, batch) in rotated
            .glyphs
            .chunks(10)
            .map(|glyphs| glyphs.iter().map(|g| g.gray.clone()).collect::<Vec<_>>())
            .enumerate()
        {
            for (offset, actual) in model.recognize(&batch).unwrap().iter().enumerate() {
                let cell = start * 10 + offset;
                let expected = ((b'A' + (cell % 26) as u8) as char).to_string();
                assert_eq!(
                    actual.text,
                    expected,
                    "quarter turn {} cell {cell}",
                    turn + 1
                );
                assert!(
                    actual.confidence >= 0.8,
                    "quarter turn {} cell {cell}: {actual:?}",
                    turn + 1
                );
            }
        }
    }
    assert!(matches!(
        model.recognize(&images[..11]),
        Err(MatrixOcrError::InvalidImage(_))
    ));
    assert!(matches!(
        model.recognize(&[vec![0; 10]]),
        Err(MatrixOcrError::InvalidImage(_))
    ));
    let guard = model.0.lock().unwrap();
    assert!(matches!(
        model.clone().recognize(&images[..1]),
        Err(MatrixOcrError::Busy)
    ));
    drop(guard);
    assert!(capture_matrix_frame(b"invalid image").is_err());
}
