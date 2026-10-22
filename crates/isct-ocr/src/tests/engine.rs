use super::*;
use image::{GrayImage, Luma};

fn recognize_single(paddle: &mut Paddle, tensor: Vec<f32>) -> Recognition {
    paddle.recognize_batch(vec![tensor], 320).unwrap().remove(0)
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
    for channel in 0..3 {
        assert!((p.tensor[channel * 48 * 320 + 24 * 320 + 15] - (20. / 127.5 - 1.)).abs() < 1e-6);
        assert_eq!(p.tensor[channel * 48 * 320 + 319], 0.);
    }
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
    let _lock = service.model.lock().unwrap();
    assert_eq!(service.scan(b"", |_| true), Err(MatrixOcrError::Busy));
}

#[test]
fn poisoned_scan_state_is_cleared_for_the_next_call() {
    let service = MatrixOcrService::default();
    let interrupted = service.clone();
    let stopped = std::thread::spawn(move || {
        let _model = interrupted.model.lock().unwrap();
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
    let result = recognize_single(&mut paddle, vec![0.; 3 * 48 * 320]);
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
        let raw = recognize_single(&mut paddle, preprocess::prepare(&cleaned).tensor);
        let original = recognize_single(&mut paddle, preprocess::prepare(&image).tensor);
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
        let lock = service.model.lock().unwrap();
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
            let wide = recognize_single(&mut paddle, preprocess::prepare(crop).tensor);
            let actual = recognition::accept(&actual.text, actual.confidence);
            let wide = recognition::accept(&wide.text, wide.confidence);
            assert_eq!(
                actual.text, expected,
                "adaptive batch changed {expected}: {actual:?}, wide {wide:?}"
            );
        }
    }
    // The public card has a solid label strip, so supply its known cell
    // geometry when exercising the downstream live-camera Paddle path.
    let fixture = image::load_from_memory(include_bytes!("fixtures/matrix-card.jpg"))
        .unwrap()
        .to_rgb8();
    let body = [
        geometry::Point { x: 250., y: 300. },
        geometry::Point { x: 1350., y: 300. },
        geometry::Point { x: 1350., y: 895. },
        geometry::Point { x: 250., y: 895. },
    ];
    let binaries = geometry::body_cells(body)
        .iter()
        .map(|cell| preprocess::binarize(&geometry::crop(&fixture, cell)))
        .collect::<Vec<_>>();
    let (full, _, _) = paddle.recognize_cells(&binaries).unwrap();
    for (i, cell) in full.iter().enumerate() {
        let expected = ((b'A' + (i % 26) as u8) as char).to_string();
        assert_eq!(cell.text, expected, "live cell {i}: {cell:?}");
        assert!(
            cell.confidence >= 0.8,
            "live cell {i} did not qualify: {cell:?}"
        );
    }
    let service = MatrixOcrService::default();
    for invalid in [vec![], vec![70], vec![1, 1]] {
        assert!(matches!(
            service.scan_live_frame(b"not decoded", &invalid, |_| true, |_| true),
            Err(MatrixOcrError::InvalidImage(_))
        ));
    }
    assert!(matches!(
        service.scan_live_frame(b"not decoded", &[20, 11], |_| true, |_| false),
        Err(MatrixOcrError::Cancelled)
    ));
}
