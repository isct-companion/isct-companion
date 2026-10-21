use super::*;
use image::{GrayImage, Luma};

fn card() -> RgbImage {
    let mut image = RgbImage::from_pixel(600, 460, Rgb([255; 3]));
    let mut fill = |x: u32, y: u32, w: u32, h: u32, color: [u8; 3]| {
        for j in y..y + h {
            for i in x..x + w {
                image.put_pixel(i, j, Rgb(color));
            }
        }
    };
    fill(0, 0, 600, 55, [175, 130, 70]);
    fill(140, 100, 350, 25, [20; 3]);
    for i in 0..10 {
        fill(151 + i * 35, 105, 8, 14, [245; 3]);
    }
    fill(105, 125, 35, 175, [65; 3]);
    for i in 0..7 {
        fill(115, 132 + i * 25, 10, 12, [240; 3]);
    }
    fill(140, 125, 350, 175, [245, 190, 190]);
    fill(143, 128, 344, 169, [255, 50, 50]);
    fill(489, 125, 2, 175, [45; 3]);
    fill(140, 299, 352, 2, [45; 3]);
    for i in 0..30 {
        fill(330 + i * 5, 325, 2, 50, [20; 3]);
    }
    fill(10, 320, 40, 50, [210, 175, 50]);
    for j in 0..3 {
        for i in 0..50 {
            fill(15 + i * 10, 395 + j * 15, 4, 7, [45; 3]);
        }
    }
    image
}

fn alternating_matrix() -> RgbImage {
    let mut image = RgbImage::from_pixel(800, 620, Rgb([255; 3]));
    for y in 190..540 {
        for x in 200..700 {
            image.put_pixel(x, y, Rgb([245, 205, 205]));
        }
    }
    for x in 200..700 {
        for thickness in 0..2 {
            image.put_pixel(x, 190 + thickness, Rgb([40; 3]));
            image.put_pixel(x, 538 + thickness, Rgb([40; 3]));
        }
    }
    for y in 190..540 {
        for thickness in 0..2 {
            image.put_pixel(200 + thickness, y, Rgb([40; 3]));
            image.put_pixel(698 + thickness, y, Rgb([40; 3]));
        }
    }
    for column in 1..10 {
        for y in 190..540 {
            for thickness in 0..2 {
                image.put_pixel(200 + column * 50 + thickness, y, Rgb([40; 3]));
            }
        }
    }
    for row in 1..7 {
        for x in 200..700 {
            for thickness in 0..2 {
                image.put_pixel(x, 190 + row * 50 + thickness, Rgb([40; 3]));
            }
        }
    }
    for y in 140..188 {
        for x in 200..700 {
            image.put_pixel(x, y, Rgb([25; 3]));
        }
    }
    for row in 0..7 {
        let color = if row % 2 == 0 { 55 } else { 145 };
        for y in 190 + row * 50..190 + (row + 1) * 50 {
            for x in 150..200 {
                image.put_pixel(x, y, Rgb([color; 3]));
            }
        }
    }
    image
}

#[test]
fn colored_backgrounds_and_noise() {
    for color in [[235, 165, 165], [150; 3], [255; 3], [210, 65, 65]] {
        let image = RgbImage::from_fn(32, 32, |x, y| {
            if (14..=17).contains(&x) && (6..26).contains(&y) {
                Rgb([20; 3])
            } else {
                Rgb(color)
            }
        });
        let binary = preprocess::binarize(&image);
        assert_eq!(binary.ink, 80);
        assert!(binary.cleaned.pixels().all(|p| p[0] == 0 || p[0] == 255));
    }
    for color in [[255, 50, 50], [90; 3]] {
        assert_eq!(
            preprocess::binarize(&RgbImage::from_pixel(32, 32, Rgb(color))).ink,
            0
        );
    }
    let mut image = GrayImage::from_pixel(32, 32, Luma([255]));
    for i in 0..8 {
        image.put_pixel(10 + i, 10 + i, Luma([0]));
    }
    for (x, y) in [(1, 1), (2, 1), (30, 30)] {
        image.put_pixel(x, y, Luma([0]));
    }
    let (clean, ink) = preprocess::remove_specks(&image);
    assert_eq!(ink, 8);
    assert_eq!(clean.get_pixel(1, 1)[0], 255);
    assert_eq!(image.get_pixel(1, 1)[0], 0);
    let mut crop = RgbImage::from_pixel(64, 64, Rgb([255; 3]));
    for y in 15..45 {
        for x in 28..33 {
            crop.put_pixel(x, y, Rgb([0; 3]));
        }
    }
    crop.put_pixel(4, 4, Rgb([0; 3]));
    let binary = preprocess::binarize(&crop);
    assert_eq!(binary.original.get_pixel(4, 4)[0], 0);
    assert_eq!(binary.cleaned.get_pixel(4, 4)[0], 255);
}

#[test]
fn grayscale_localizer_finds_cells_from_alternating_labels_and_dark_header() {
    let cells = scan_image_locator::locate(&alternating_matrix()).expect("synthetic matrix grid");
    assert_eq!(cells.len(), MATRIX_CELLS);
    for (actual, expected) in [
        (cells[0][0].x, 200.),
        (cells[0][0].y, 190.),
        (cells[69][2].x, 700.),
        (cells[69][2].y, 540.),
        (cells[1][0].x, 250.),
    ] {
        assert!((actual - expected).abs() < 8., "{actual} vs {expected}");
    }
    assert!(scan_image_locator::locate(&RgbImage::from_pixel(800, 620, Rgb([255; 3]))).is_none());
}

#[test]
fn model_input_padding_and_normalization() {
    let image = GrayImage::from_fn(16, 32, |x, y| {
        Luma([if (6..10).contains(&x) && (8..24).contains(&y) {
            20
        } else {
            255
        }])
    });
    let p = preprocess::prepare(&image);
    assert_eq!(p.tensor.len(), 3 * 48 * 320);
    assert_eq!(p.preview.dimensions(), (320, 48));
    for channel in 0..3 {
        assert!((p.tensor[channel * 48 * 320 + 24 * 320 + 15] - (20. / 127.5 - 1.)).abs() < 1e-6);
        assert_eq!(p.tensor[channel * 48 * 320 + 319], 0.);
    }
    assert_eq!(p.preview.get_pixel(319, 0)[0], 128);
}
#[test]
fn decode_limits_alpha_and_busy_scan() {
    assert!(decode_image(b"not an image").is_err());
    assert!(decode_image(&[]).is_err());
    let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 0, 0]));
    let mut bytes = Cursor::new(vec![]);
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    assert_eq!(
        decode_image(bytes.get_ref()).unwrap().get_pixel(0, 0).0,
        [255; 3]
    );
    let service = MatrixOcrService::default();
    let _lock = service.0.lock().unwrap();
    assert_eq!(service.scan(b"", |_| true), Err(MatrixOcrError::Busy));
}

#[test]
fn poisoned_scan_state_is_cleared_for_the_next_call() {
    let service = MatrixOcrService::default();
    let interrupted = service.clone();
    let stopped = std::thread::spawn(move || {
        let _model = interrupted.0.lock().unwrap();
        panic!("simulate an interrupted scan");
    });
    assert!(stopped.join().is_err());

    assert!(matches!(
        service.scan(b"not an image", |_| true),
        Err(MatrixOcrError::Model(message)) if message.contains("recovered")
    ));
    assert!(matches!(
        service.scan(b"not an image", |_| true),
        Err(MatrixOcrError::InvalidImage(_))
    ));
}

#[test]
fn embedded_model_runs_a_synthetic_blank_tensor() {
    let mut paddle = Paddle::new().expect("embedded Paddle model should initialize");
    let result = paddle
        .recognize(vec![0.; 3 * 48 * 320])
        .expect("synthetic tensor inference should succeed");
    assert!(result.confidence.is_finite());
}

// Local private fixtures are deliberately not embedded in the app or ordinary tests.
// Run explicitly with MATRIX_CROP_DIR pointing at the 18 previously exported PNGs.
#[test]
#[ignore = "requires local private crop fixtures"]
fn real_crops_native_model() {
    let dir =
        std::path::PathBuf::from(std::env::var("MATRIX_CROP_DIR").expect("Set MATRIX_CROP_DIR"));
    let cases = [
        ("D1", "I"),
        ("I1", "Z"),
        ("J1", "P"),
        ("A3", "I"),
        ("A4", "V"),
        ("H4", "Z"),
        ("J4", "R"),
        ("E5", "I"),
        ("G5", "I"),
        ("J5", "S"),
        ("A6", "B"),
        ("E6", "O"),
        ("F6", "O"),
        ("I6", "O"),
        ("C7", "Z"),
        ("G7", "R"),
        ("I7", "S"),
        ("J7", "I"),
    ];
    let start = std::time::Instant::now();
    let mut paddle = Paddle::new().unwrap();
    let load = start.elapsed();
    let mut results = vec![];
    let start = std::time::Instant::now();
    for (cell, expected) in cases {
        let image = image::open(dir.join(format!("{cell}.png")))
            .unwrap()
            .to_luma8();
        let (cleaned, _) = preprocess::remove_specks(&image);
        let raw = paddle
            .recognize(preprocess::prepare(&cleaned).tensor)
            .unwrap();
        let original = paddle
            .recognize(preprocess::prepare(&image).tensor)
            .unwrap();
        let accepted = recognition::review(
            recognition::accept(&raw.text, raw.confidence),
            &original.text,
        );
        assert_eq!(accepted.text, expected, "cell {cell}");
        results.push(
            serde_json::json!({"cell":cell,"expected":expected,"raw":raw,"accepted":accepted}),
        );
    }
    let report = serde_json::json!({"correct":18,"total":18,"model_load_ms":load.as_millis(),"recognition_ms":start.elapsed().as_millis(),"results":results});
    if let Ok(path) = std::env::var("MATRIX_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    eprintln!(
        "Native model: 18/18; load {}ms; 36 inferences {}ms",
        load.as_millis(),
        start.elapsed().as_millis()
    );
}
#[test]
#[ignore = "requires local full-card photo; writes only red model input PNGs"]
fn full_photo_native_pipeline() {
    let file = std::env::var("MATRIX_PHOTO").expect("Set MATRIX_PHOTO");
    let bytes = std::fs::read(file).unwrap();
    let service = MatrixOcrService::default();
    let mut previous_session = 0;
    for pass in 0..2 {
        let progress = std::sync::atomic::AtomicUsize::new(0);
        let start = std::time::Instant::now();
        let result = service
            .scan(&bytes, |state| {
                progress.store(state.completed, std::sync::atomic::Ordering::Relaxed);
                true
            })
            .unwrap();
        assert_eq!(result.len(), 70);
        assert_eq!(progress.load(std::sync::atomic::Ordering::Relaxed), 70);
        // Visible F–J cells in rows 5–7 of the supplied reference photo.
        for (row, text) in [(4, "VIUAS"), (5, "OEXOV"), (6, "PRISI")] {
            for (c, letter) in text.chars().enumerate() {
                assert_eq!(
                    result[row * 10 + 5 + c].text,
                    letter.to_string(),
                    "row {}, col {}",
                    row + 1,
                    c + 6
                );
            }
        }
        let lock = service.0.lock().unwrap();
        let address = lock.as_ref().unwrap() as *const Paddle as usize;
        if pass > 0 {
            assert_eq!(address, previous_session);
        }
        previous_session = address;
        eprintln!(
            "Full photo pass {pass}: {}ms, 70 cells",
            start.elapsed().as_millis()
        );
    }
}

#[test]
fn filter_matches_frozen_typescript_reference_pixel_for_pixel() {
    use sha2::{Digest, Sha256};
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../engine/fixtures/ts-filter.json")).unwrap();
    for case in cases {
        let (w, h, seed) = (
            case["width"].as_u64().unwrap() as u32,
            case["height"].as_u64().unwrap() as u32,
            case["seed"].as_u64().unwrap() as u32,
        );
        let mut state = seed;
        let mut rgb = vec![0; (w * h * 3) as usize];
        for v in &mut rgb {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            *v = (state >> 24) as u8;
        }
        let mut image = RgbImage::from_raw(w, h, rgb).unwrap();
        for y in 0..h {
            for x in 0..w {
                if (x as f64) < w as f64 / 2. {
                    let ink = x == w / 4 || y == h / 2;
                    image.put_pixel(
                        x,
                        y,
                        if ink {
                            Rgb([20; 3])
                        } else {
                            Rgb([220, 150, 150])
                        },
                    );
                }
            }
        }
        let result = preprocess::binarize(&image);
        assert_eq!(
            format!("{:x}", Sha256::digest(result.original.as_raw())),
            case["original"].as_str().unwrap(),
            "binary {w}x{h} seed {seed}"
        );
        assert_eq!(
            format!("{:x}", Sha256::digest(result.cleaned.as_raw())),
            case["cleaned"].as_str().unwrap(),
            "denoised {w}x{h} seed {seed}"
        );
        assert_eq!(result.ink, case["ink"].as_u64().unwrap() as usize);
    }
}

#[test]
fn raw_image_decoding_preserves_source_resolution() {
    let source = RgbImage::from_pixel(2400, 1200, Rgb([17, 29, 41]));
    let mut png = Cursor::new(vec![]);
    source.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let decoded = decode_image(png.get_ref()).unwrap();
    assert_eq!(decoded.dimensions(), (2400, 1200));
    assert_eq!(decoded.get_pixel(0, 0).0, [17, 29, 41]);
    assert!(matches!(
        decode_image(&vec![0; MAX_IMAGE_BYTES + 1]),
        Err(MatrixOcrError::InvalidImage(_))
    ));
}

#[test]
fn invalid_input_does_not_poison_the_service() {
    let service = MatrixOcrService::default();
    for _ in 0..2 {
        assert!(matches!(
            service.scan(b"not an image", |_| true),
            Err(MatrixOcrError::InvalidImage(_))
        ));
    }
}

#[test]
fn cancellation_releases_the_service_for_a_later_scan() {
    let service = MatrixOcrService::default();
    assert_eq!(
        service.scan(b"not decoded", |_| false),
        Err(MatrixOcrError::Cancelled)
    );
    assert!(matches!(
        service.scan(b"not decoded", |_| true),
        Err(MatrixOcrError::InvalidImage(_))
    ));
    let mut png = Cursor::new(Vec::new());
    alternating_matrix()
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    assert!(matches!(
        service.scan_frame_with_geometry(png.get_ref(), |_| false, |_| true),
        Err(MatrixOcrError::Cancelled)
    ));
    assert!(service.scan_frame(png.get_ref(), |_| true).is_ok());
}

#[test]
#[ignore = "requires an explicitly supplied private full-card image and optional ground truth"]
fn supplied_photo_pipeline() {
    let source = std::fs::read(std::env::var("MATRIX_PHOTO").expect("Set MATRIX_PHOTO")).unwrap();
    let service = MatrixOcrService::default();
    #[cfg(debug_assertions)]
    let service = service.with_debug_dir(Some(std::path::PathBuf::from(
        std::env::var("MATRIX_PHOTO").unwrap(),
    )));
    // A file as the export directory forces a write failure on every OS.
    // Recognition and all progress events must still be delivered.
    let progress = std::sync::atomic::AtomicUsize::new(0);
    let results = service
        .scan(&source, |state| {
            progress.store(state.completed, std::sync::atomic::Ordering::Relaxed);
            assert_eq!(state.total, MATRIX_CELLS);
            true
        })
        .unwrap();
    assert_eq!(progress.load(std::sync::atomic::Ordering::Relaxed), 70);
    assert_eq!(results.len(), 70);
    let accepted = results.iter().filter(|r| !r.text.is_empty()).count();
    let expected = std::env::var("MATRIX_EXPECTED_FILE")
        .ok()
        .map(|p| std::fs::read_to_string(p).unwrap());
    let matched = expected
        .as_ref()
        .map(|expected| {
            results
                .iter()
                .enumerate()
                .filter(|(i, r)| r.text == expected[*i..*i + 1])
                .count()
        })
        .unwrap_or(0);
    if expected.is_some() {
        assert_eq!(
            matched, accepted,
            "private photo accepted an incorrect letter"
        );
        assert!(
            accepted >= 60,
            "private photo lost too many readable letters"
        );
    }
    eprintln!("Private photo: 70 cells returned, {accepted} accepted, {matched} matched");
}

#[test]
fn narrow_batches_preserve_latin_cell_text_order_and_live_selection_boundaries() {
    let sheet = image::load_from_memory(include_bytes!("fixtures/latin-cells.png"))
        .unwrap()
        .to_luma8();
    let mut paddle = Paddle::new().unwrap();
    for start in (0..26).step_by(5) {
        let end = (start + 5).min(26);
        let crops: Vec<_> = (start..end)
            .map(|i| image::imageops::crop_imm(&sheet, i * 96, 0, 96, 96).to_image())
            .collect();
        let (narrow, _, _) = paddle
            .recognize_glyphs(&crops.iter().collect::<Vec<_>>())
            .unwrap();
        for (offset, (crop, actual)) in crops.iter().zip(narrow).enumerate() {
            let expected = ((b'A' + start as u8 + offset as u8) as char).to_string();
            let wide = paddle.recognize(preprocess::prepare(crop).tensor).unwrap();
            let actual = recognition::accept(&actual.text, actual.confidence);
            let wide = recognition::accept(&wide.text, wide.confidence);
            assert_eq!(
                actual.text, expected,
                "adaptive batch changed {expected}: {actual:?}, wide {wide:?}"
            );
        }
    }
    let full = MatrixOcrService::default()
        .scan_live_frame(
            include_bytes!("fixtures/matrix-card.jpg"),
            &(0..70).collect::<Vec<_>>(),
            |_| true,
            |_| true,
        )
        .unwrap();
    assert_eq!(full.cells.len(), MATRIX_CELLS);
    assert_eq!(full.quads.len(), MATRIX_CELLS);
    for (cell, corner, expected_x, expected_y) in [
        (0, 0, 250. / 1600., 300. / 1100.),
        (9, 1, 1350. / 1600., 300. / 1100.),
        (69, 2, 1350. / 1600., 900. / 1100.),
        (60, 3, 250. / 1600., 900. / 1100.),
    ] {
        let [x, y] = full.quads[cell][corner];
        assert!((x - expected_x).abs() < 0.02, "cell {cell} x={x}");
        assert!((y - expected_y).abs() < 0.02, "cell {cell} y={y}");
    }
    for row in 0..7 {
        for column in 0..9 {
            assert!(
                full.quads[row * 10 + column][0][0] < full.quads[row * 10 + column + 1][0][0],
                "columns are out of order at row {row}"
            );
        }
    }
    for row in 0..6 {
        for column in 0..10 {
            assert!(
                full.quads[row * 10 + column][0][1] < full.quads[(row + 1) * 10 + column][0][1],
                "rows are out of order at column {column}"
            );
        }
    }
    let mut encoded = Cursor::new(Vec::new());
    card()
        .write_to(&mut encoded, image::ImageFormat::Jpeg)
        .unwrap();
    let service = MatrixOcrService::default();
    for invalid in [vec![], vec![70], vec![1, 1]] {
        assert!(matches!(
            service.scan_live_frame(encoded.get_ref(), &invalid, |_| true, |_| true),
            Err(MatrixOcrError::InvalidImage(_))
        ));
    }
    assert!(matches!(
        service.scan_live_frame(encoded.get_ref(), &[20, 11], |_| false, |_| true),
        Err(MatrixOcrError::Cancelled)
    ));
    let selected = service
        .scan_live_frame(encoded.get_ref(), &[20, 11], |_| true, |_| true)
        .unwrap();
    assert_eq!(selected.cells.len(), 2);
    assert_eq!(selected.quads.len(), MATRIX_CELLS);
    assert!(selected.timing.total_ms >= selected.timing.inference_ms);
}
