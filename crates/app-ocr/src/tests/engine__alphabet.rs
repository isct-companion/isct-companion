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
    // Edge acuity falls when the strokes blur and holds when only the
    // contrast changes, so a dim but sharp frame is not taken for blur.
    let acuity = |gray: &GrayImage| {
        let interior = image::imageops::crop_imm(gray, 9, 9, 78, 78).to_image();
        edge_acuity(&image::imageops::resize(
            &interior,
            48,
            48,
            FilterType::Triangle,
        ))
    };
    let median = |values: &mut Vec<f64>| {
        values.sort_by(f64::total_cmp);
        values[values.len() / 2]
    };
    let mut levels = vec![vec![]; 4];
    let mut dim = vec![];
    for g in &captured.glyphs {
        let gray = GrayImage::from_raw(96, 96, g.gray.clone()).unwrap();
        for (level, sigma) in levels.iter_mut().zip([0., 1.5, 3., 5.]) {
            level.push(if sigma > 0. {
                acuity(&imageproc::filter::gaussian_blur_f32(&gray, sigma))
            } else {
                acuity(&gray)
            });
        }
        dim.push(acuity(&GrayImage::from_fn(96, 96, |x, y| {
            image::Luma([(128 + (gray.get_pixel(x, y)[0] as i32 - 128) / 2) as u8])
        })));
    }
    let levels: Vec<f64> = levels.iter_mut().map(median).collect();
    assert!(levels.windows(2).all(|w| w[1] < w[0]), "{levels:?}");
    assert!(levels[3] < 0.6 * levels[0], "{levels:?}");
    let dim = median(&mut dim);
    assert!(
        (dim - levels[0]).abs() < 0.1 * levels[0],
        "dim {dim} vs {levels:?}"
    );
    // Keep black ink on both paper colors, and prevent input whitening from
    // changing physical glare/admission measurements.
    for paper in [[245, 205, 205], [240; 3], [130, 110, 100]] {
        let rgb = RgbImage::from_fn(96, 96, |x, y| {
            if (38..52).contains(&x) && (20..76).contains(&y) {
                Rgb([20; 3])
            } else {
                Rgb(paper)
            }
        });
        let projected = project_glyph(&rgb, crate::MatrixGlyphProjection::PaperDistance);
        assert!(projected.get_pixel(30, 30)[0] >= 250);
        assert!(projected.get_pixel(45, 45)[0] < 55);
        assert!(prepare_matrix_alphabet_input(projected.as_raw())
            .unwrap()
            .is_some());
    }
    for projection in [
        crate::MatrixGlyphProjection::RedChroma,
        crate::MatrixGlyphProjection::PaperDistance,
    ] {
        let color = capture_matrix_frame_configured(
            fixture,
            crate::MatrixCaptureConfig {
                glyph_projection: projection,
            },
        )
        .unwrap();
        assert_eq!(captured.quads, color.quads);
        assert!(captured
            .glyphs
            .iter()
            .zip(&color.glyphs)
            .any(|(a, b)| a.gray != b.gray));
        for (a, b) in captured.glyphs.iter().zip(color.glyphs) {
            assert_eq!(
                (
                    a.contrast,
                    a.sharpness,
                    a.centering,
                    a.glare,
                    a.source_size,
                    a.acuity
                ),
                (
                    b.contrast,
                    b.sharpness,
                    b.centering,
                    b.glare,
                    b.source_size,
                    b.acuity
                )
            );
        }
    }
    #[cfg(feature = "training-tools")]
    {
        let body = [
            captured.quads[0][0],
            captured.quads[9][1],
            captured.quads[69][2],
            captured.quads[60][3],
        ];
        let recovered = capture_matrix_training_frame(fixture, body).unwrap();
        let model = MatrixAlphabetService::default();
        for (batch_index, glyphs) in recovered.glyphs.chunks(10).enumerate() {
            let inputs = glyphs.iter().map(|g| g.gray.clone()).collect::<Vec<_>>();
            for (offset, reading) in model.recognize(&inputs).unwrap().iter().enumerate() {
                let cell = batch_index * 10 + offset;
                assert_eq!(
                    reading.text,
                    ((b'A' + (cell % 26) as u8) as char).to_string(),
                    "assisted cell {cell}"
                );
            }
        }
        for invalid in [
            [[0., 0.]; 4],
            [body[0], body[2], body[1], body[3]],
            [[f64::NAN, 0.], body[1], body[2], body[3]],
            [[-0.1, 0.], body[1], body[2], body[3]],
        ] {
            assert!(matches!(
                capture_matrix_training_frame(fixture, invalid),
                Err(MatrixOcrError::InvalidImage(_))
            ));
        }
    }
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
    for size in [96 * 96 - 1, 96 * 96 + 1] {
        assert!(matches!(
            prepare_matrix_alphabet_input(&vec![0; size]),
            Err(MatrixOcrError::InvalidImage(_))
        ));
    }
    let prepared = prepare_matrix_alphabet_input(&images[0]).unwrap().unwrap();
    let direct = model.recognize_raw(&[prepared.original]).unwrap();
    assert_eq!(direct[0].text, "A");
    assert!(
        (direct[0].confidence - model.recognize(&images[..1]).unwrap()[0].confidence).abs() < 1e-6
    );
    let guard = model.0.lock().unwrap();
    assert!(matches!(
        model.clone().recognize(&images[..1]),
        Err(MatrixOcrError::Busy)
    ));
    drop(guard);
    assert!(capture_matrix_frame(b"invalid image").is_err());
}

#[test]
fn complete_letters_near_cell_edges_do_not_become_confident_substitutions() {
    // Public Times Italic L/Z/I, with complete ink inside each cell. The old
    // fixed 9% inset turned all 24 Ls into confidently accepted Is.
    let captured =
        capture_matrix_frame(include_bytes!("fixtures/matrix-letter-boundaries.jpg")).unwrap();
    let model = MatrixAlphabetService::default();
    for (batch_index, batch) in captured.glyphs.chunks(10).enumerate() {
        let images: Vec<_> = batch.iter().map(|g| g.gray.clone()).collect();
        for (offset, actual) in model.recognize(&images).unwrap().iter().enumerate() {
            let cell = batch_index * 10 + offset;
            let expected = ["L", "Z", "I"][cell % 3];
            assert_eq!(actual.text, expected, "cell {cell}: {actual:?}");
            assert!(actual.confidence >= 0.8, "cell {cell}: {actual:?}");
        }
    }
    // Public Times New Roman Q, drawn on a 140x100 physical cell with only
    // two pixels below the descender, then sampled into the 96x96 transport.
    // An inward crop can erase its tail and produce a confidently wrong O.
    let q = image::load_from_memory(include_bytes!("fixtures/matrix-q-bottom.png"))
        .unwrap()
        .to_luma8();
    // On the printed card the tail often reaches the row line itself: the
    // same Q moved down until its tail touches the bottom edge is still read,
    // as a narrow bottom contact is a descender, not a clipped stroke.
    let lowest = (0..96)
        .rev()
        .find(|&y| (0..96).any(|x| q.get_pixel(x, y)[0] < 100))
        .unwrap();
    let paper = *q.get_pixel(0, 0);
    let touching = GrayImage::from_fn(96, 96, |x, y| match y.checked_sub(95 - lowest) {
        Some(source) => *q.get_pixel(x, source),
        None => paper,
    });
    assert!(normalize_glyph(&touching).is_some());
    let reading = model
        .recognize(&[q.into_raw(), touching.into_raw()])
        .unwrap();
    for read in &reading {
        assert_eq!(read.text, "Q");
        assert!(read.confidence >= 0.8, "{read:?}");
    }
    // A bar cut by the bottom edge touches it wide: an E losing its lower
    // bar there would otherwise read as F.
    let cut_bar = GrayImage::from_fn(96, 96, |x, y| {
        let stem = (30..37).contains(&x) && y >= 20;
        let bars =
            (30..70).contains(&x) && ((20..27).contains(&y) || (52..58).contains(&y) || y >= 90);
        image::Luma([if stem || bars { 20 } else { 230 }])
    });
    assert!(normalize_glyph(&cut_bar).is_none());
    // A central stroke joined to an incomplete edge is not safe to delete:
    // deleting that connected component could leave a different letter.
    let clipped = GrayImage::from_fn(96, 96, |x, y| {
        image::Luma([
            if ((45..51).contains(&x) && y < 70) || ((60..66).contains(&x) && (25..70).contains(&y))
            {
                20
            } else {
                230
            },
        ])
    });
    assert!(normalize_glyph(&clipped).is_none());
    let rejected = model.recognize(&[clipped.into_raw()]).unwrap();
    assert!(rejected[0].text.is_empty());
    assert_eq!(rejected[0].confidence, 0.);
}

#[test]
fn capture_fits_body_rows_independently_of_header_height() {
    // Real cards need not give the label header the same height as a body row.
    // Dividing the whole contour into eight equal rows clips otherwise clear ink.
    let fixture = image::load_from_memory(include_bytes!("fixtures/matrix-card.jpg"))
        .unwrap()
        .to_rgb8();
    let header = image::imageops::crop_imm(&fixture, 250, 211, 1101, 89).to_image();
    for height in [40, 125] {
        let mut card = fixture.clone();
        for y in 0..300 {
            for x in 0..card.width() {
                card.put_pixel(x, y, Rgb([255; 3]));
            }
        }
        let header = image::imageops::resize(&header, 1101, height, FilterType::Triangle);
        image::imageops::replace(&mut card, &header, 250, (300 - height) as i64);
        let mut encoded = std::io::Cursor::new(Vec::new());
        card.write_to(&mut encoded, image::ImageFormat::Jpeg)
            .unwrap();
        let captured = capture_matrix_frame(encoded.get_ref()).unwrap();
        // Alternating labels may locate the body, but must not shift individual
        // row seams after the single projective transform is established.
        let body = [
            captured.quads[0][0],
            captured.quads[9][1],
            captured.quads[69][2],
            captured.quads[60][3],
        ]
        .map(|[x, y]| geometry::Point { x, y });
        let mechanical = geometry::body_cells(body);
        for (actual, expected) in captured.quads.iter().zip(mechanical) {
            for ([x, y], p) in actual.iter().zip(expected) {
                assert!((x - p.x).abs() < 1e-10 && (y - p.y).abs() < 1e-10);
            }
        }
        for (i, quad) in captured.quads.iter().enumerate() {
            let (c, r) = ((i % 10) as f64, (i / 10) as f64);
            for ([x, y], (dc, dr)) in quad.iter().zip([(0., 0.), (1., 0.), (1., 1.), (0., 1.)]) {
                assert!(
                    (x * card.width() as f64 - (250. + (c + dc) * 110.)).abs() < 8.
                        && (y * card.height() as f64 - (300. + (r + dr) * 85.)).abs() < 8.,
                    "header {height}, cell {i}: {quad:?}"
                );
            }
        }
    }
}

#[test]
fn illumination_recovers_dim_admission_and_distinguishes_glare_from_white_paper() {
    let fixture = image::load_from_memory(include_bytes!("fixtures/matrix-card.jpg"))
        .unwrap()
        .to_rgb8();
    let mut dark = fixture.clone();
    for pixel in dark.pixels_mut() {
        for value in &mut pixel.0 {
            *value = (*value as f64 * 0.15).round() as u8;
        }
    }
    let mut encoded = std::io::Cursor::new(Vec::new());
    dark.write_to(&mut encoded, image::ImageFormat::Jpeg)
        .unwrap();
    let captured = capture_matrix_frame(encoded.get_ref()).unwrap();
    assert!(
        captured
            .glyphs
            .iter()
            .filter(|g| g.contrast >= 12.
                && g.sharpness >= 10.
                && g.source_size >= 10.
                && g.centering >= 0.5)
            .count()
            >= 35
    );
    let model = MatrixAlphabetService::default();
    for (batch_index, glyphs) in captured.glyphs.chunks(10).enumerate() {
        let inputs = glyphs.iter().map(|g| g.gray.clone()).collect::<Vec<_>>();
        for (offset, reading) in model.recognize(&inputs).unwrap().iter().enumerate() {
            let cell = batch_index * 10 + offset;
            let expected = ((b'A' + (cell % 26) as u8) as char).to_string();
            assert_eq!(reading.text, expected, "dim cell {cell}");
            assert!(
                reading.confidence >= 0.8,
                "dim cell {cell}: {reading:?}, quad {:?}",
                captured.quads[cell]
            );
        }
    }

    let white = GrayImage::from_fn(96, 96, |x, y| {
        image::Luma([if (43..53).contains(&x) && (20..76).contains(&y) {
            20
        } else {
            255
        }])
    });
    let light = illumination::analyze(&white);
    assert_eq!(light.glare, 0.);
    assert_eq!(light.glare_against(255.), 0.);
    assert!(light.glare_against(160.) > 0.12);
    let glare = GrayImage::from_fn(96, 96, |x, y| {
        image::Luma([if x > 48 {
            255
        } else if (38..48).contains(&x) && (20..76).contains(&y) {
            20
        } else {
            160
        }])
    });
    assert!(illumination::analyze(&glare).glare > 0.12);

    // A reflection through a pink row used to yield three confidently wrong
    // settled letters. Reject those observations before they can vote. (On a
    // white row a highlight only whitens paper; row 5 is pink on the card.)
    let mut striped = fixture.clone();
    for (_, y, pixel) in striped.enumerate_pixels_mut() {
        let highlight =
            400. * (-((y as f64 / fixture.height() as f64 - 0.625) / 0.012).powi(2)).exp();
        for value in &mut pixel.0 {
            *value = (*value as f64 + highlight).min(255.) as u8;
        }
    }
    let mut encoded = std::io::Cursor::new(Vec::new());
    striped
        .write_to(&mut encoded, image::ImageFormat::Jpeg)
        .unwrap();
    let captured = capture_matrix_frame(encoded.get_ref()).unwrap();
    for cell in [45, 46, 47] {
        assert!(captured.glyphs[cell].glare > 0.12, "reflection cell {cell}");
    }
    for (dx, dy) in [(0.4, 0.), (0.8, 0.6), (0., 1.2)] {
        let shifted = imageproc::geometric_transformations::warp(
            &striped,
            Projection::translate(-dx, -dy),
            Interpolation::Bilinear,
            Border::Constant(Rgb([255; 3])),
        );
        let mut encoded = vec![];
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 90)
            .encode_image(&shifted)
            .unwrap();
        let captured = capture_matrix_frame(&encoded).unwrap();
        // Glare must not change row pitch and move intact lower rows into
        // adjacent cells when the camera shifts by a fraction of a pixel.
        for (actual, expected) in [
            (captured.quads[0][0], [250. - dx as f64, 300. - dy as f64]),
            (captured.quads[69][2], [1350. - dx as f64, 895. - dy as f64]),
        ] {
            assert!(
                (actual[0] * fixture.width() as f64 - expected[0]).abs() < 8.
                    && (actual[1] * fixture.height() as f64 - expected[1]).abs() < 8.,
                "reflection shift ({dx}, {dy}): {actual:?} vs {expected:?}"
            );
        }
    }
}
