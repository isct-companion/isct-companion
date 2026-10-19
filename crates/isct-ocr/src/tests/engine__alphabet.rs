use super::*;
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
