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
#[test]
fn whole_card_geometry_and_unrelated_photo() {
    let cells = geometry::locate(&card()).unwrap();
    assert_eq!(cells.len(), 70);
    for (actual, expected) in [
        (cells[0][0].x, 140.),
        (cells[0][0].y, 125.),
        (cells[69][2].x, 490.),
        (cells[69][2].y, 300.),
        (cells[1][0].x, 175.),
    ] {
        assert!((actual - expected).abs() < 5., "{actual} vs {expected}");
    }
    assert!(geometry::locate(&RgbImage::from_pixel(600, 460, Rgb([255; 3]))).is_none());
}
#[test]
fn cropped_grid_fallback() {
    let mut image = RgbImage::from_pixel(420, 300, Rgb([255; 3]));
    for c in 0..11 {
        for y in 10..291 {
            image.put_pixel(10 + c * 40, y, Rgb([0; 3]));
        }
    }
    for r in 0..8 {
        for x in 10..411 {
            image.put_pixel(x, 10 + r * 40, Rgb([0; 3]));
        }
    }
    let cells = geometry::locate(&image).unwrap();
    assert_eq!(cells.len(), 70);
    assert!((cells[0][0].x - 10.).abs() < 2.);
    assert!((cells[69][2].y - 290.).abs() < 2.);
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
fn raw_image_is_bounded_and_resampled_at_both_core_sizes() {
    let source = RgbImage::from_pixel(2400, 1200, Rgb([17, 29, 41]));
    let mut png = Cursor::new(vec![]);
    source.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let decoded = decode_image(png.get_ref()).unwrap();
    let prepared = resize_longest(decoded, 2200);
    let preview = resize_longest(prepared.clone(), 600);
    assert_eq!(prepared.dimensions(), (2200, 1100));
    assert_eq!(preview.dimensions(), (600, 300));
    assert_eq!(prepared.get_pixel(0, 0).0, [17, 29, 41]);
    assert!(matches!(
        decode_image(&vec![0; MAX_IMAGE_BYTES + 1]),
        Err(MatrixOcrError::InvalidImage(_))
    ));
}

#[test]
fn core_resampling_preserves_the_detected_card_grid() {
    let large = image::imageops::resize(&card(), 2400, 1840, image::imageops::FilterType::Nearest);
    let mut png = Cursor::new(vec![]);
    large.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let (source, preview) = prepare_images(png.get_ref()).unwrap();
    assert_eq!(source.dimensions(), (2200, 1687));
    assert_eq!(preview.dimensions(), (600, 460));
    let cells = geometry::locate(&preview).expect("resampled card grid");
    assert_eq!(cells.len(), MATRIX_CELLS);
    assert!((cells[0][0].x - 140.).abs() < 5.);
    assert!((cells[69][2].y - 300.).abs() < 5.);
    // The encoded camera frame must return the same cell order and normalized
    // corners used by native cropping, so the overlay cannot drift or transpose.
    let located = std::cell::Cell::new(false);
    let frame = MatrixOcrService::default()
        .scan_frame_with_geometry(
            png.get_ref(),
            |quads| {
                assert_eq!(quads.len(), MATRIX_CELLS);
                located.set(true);
                true
            },
            |progress| {
                if progress.completed > 0 {
                    assert!(located.get(), "geometry must precede inference");
                }
                true
            },
        )
        .unwrap();
    assert_eq!(frame.cells.len(), MATRIX_CELLS);
    assert_eq!(frame.quads.len(), MATRIX_CELLS);
    for (actual, expected) in frame.quads.iter().zip(&cells) {
        for ([x, y], point) in actual.iter().zip(expected) {
            assert!((x - point.x / preview.width() as f64).abs() < 1e-9);
            assert!((y - point.y / preview.height() as f64).abs() < 1e-9);
            assert!((0. ..=1.).contains(x) && (0. ..=1.).contains(y));
        }
    }
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
    card().write_to(&mut png, image::ImageFormat::Png).unwrap();
    assert!(matches!(
        service.scan_frame_with_geometry(png.get_ref(), |_| false, |_| true),
        Err(MatrixOcrError::Cancelled)
    ));
    assert!(service.scan_frame(png.get_ref(), |_| true).is_ok());
}

#[test]
fn white_header_letters_may_split_the_dark_band() {
    let mut image = card();
    // Full-height, wider white glyphs break all wide row runs for 18 rows.
    // The old fixed four-row join split the header into two invalid thin bands.
    for i in 0..10 {
        for y in 104..122 {
            for x in 150 + i * 35..164 + i * 35 {
                image.put_pixel(x, y, Rgb([245; 3]));
            }
        }
    }
    let cells = geometry::locate(&image).expect("header split by white glyphs");
    assert!((cells[0][0].x - 140.).abs() < 5.);
    assert!((cells[0][0].y - 125.).abs() < 5.);
    assert!((cells[69][2].y - 300.).abs() < 5.);
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

fn outlined_card() -> RgbImage {
    let mut image = RgbImage::from_pixel(600, 600, Rgb([255; 3]));
    for y in 180..420 {
        for x in 120..450 {
            let header = y < 210 && x >= 150;
            let label = x < 150 && y >= 210;
            let grid = x >= 150 && y >= 210;
            if header || label {
                image.put_pixel(x, y, Rgb([30; 3]));
            } else if grid {
                let boundary = (x - 150) % 30 <= 1 || (y - 210) % 30 <= 1 || x >= 448 || y >= 418;
                image.put_pixel(
                    x,
                    y,
                    if boundary {
                        Rgb([70; 3])
                    } else {
                        Rgb([245, 210, 210])
                    },
                );
            }
        }
    }
    // Distinct glyph shapes in each cell exercise crop direction as well as order.
    for i in 0..70 {
        for y in 7..23 {
            for x in 8..20 {
                if x == 8 || y == 7 || (y == 15 && x < 14) {
                    image.put_pixel(
                        150 + (i % 10) * 30 + x,
                        210 + (i / 10) * 30 + y,
                        Rgb([20; 3]),
                    );
                }
            }
        }
    }
    image
}

fn dim_blue_camera_card() -> RgbImage {
    let mut image = outlined_card();
    for pixel in image.pixels_mut() {
        *pixel = match pixel.0 {
            [30, 30, 30] => Rgb([10, 35, 75]),
            [70, 70, 70] => Rgb([105, 125, 145]),
            [245, 210, 210] => Rgb([48, 70, 96]),
            [20, 20, 20] => Rgb([5, 10, 15]),
            _ => *pixel,
        };
    }
    image
}
#[test]
fn arbitrary_rotation_stretch_and_perspective_keep_cell_order() {
    use imageproc::geometric_transformations::{warp, Border, Interpolation, Projection};
    let image = outlined_card();
    let perspective = Projection::from_control_points(
        [(120., 180.), (450., 180.), (450., 420.), (120., 420.)],
        [(135., 165.), (450., 200.), (415., 435.), (100., 400.)],
    )
    .unwrap();
    let stretch = Projection::translate(300., 300.)
        * Projection::scale(1.4, 0.75)
        * Projection::translate(-300., -300.);
    for (case, deformation) in [Projection::scale(1., 1.), stretch, perspective]
        .into_iter()
        .enumerate()
    {
        for degrees in (0..360).step_by(15).map(|d| d as f32).chain([17.5, 359.]) {
            let transform = Projection::translate(300., 300.)
                * Projection::rotate(degrees.to_radians())
                * Projection::translate(-300., -300.)
                * deformation;
            let rotated = warp(
                &image,
                transform,
                Interpolation::Bilinear,
                Border::Constant(Rgb([255; 3])),
            );
            let cells = geometry::locate(&rotated)
                .unwrap_or_else(|| panic!("grid lost at {degrees} degrees case {case}"));
            let camera = geometry::locate_capture(&rotated)
                .unwrap_or_else(|| panic!("camera grid lost at {degrees} degrees case {case}"));
            for (actual, original) in [(camera[0][0], (150., 210.)), (camera[69][2], (450., 420.))]
            {
                let expected = transform * original;
                assert!((actual.x-expected.0 as f64).hypot(actual.y-expected.1 as f64) < 6., "camera crop changed cell identity at {degrees} degrees: {actual:?} vs {expected:?}");
            }
            for (actual, original) in [(cells[0][0], (150., 210.)), (cells[69][2], (450., 420.))] {
                let expected = transform * original;
                assert!((actual.x-expected.0 as f64).hypot(actual.y-expected.1 as f64) < 6., "incorrect cell order/geometry at {degrees} degrees: {actual:?} vs {expected:?}");
            }
        }
    }
    // A backlit webcam can expose the blue card body below the old fixed dark
    // threshold. The camera path must still separate the darker header/labels,
    // and preserve cell order at every right-angle device orientation.
    let image = dim_blue_camera_card();
    for degrees in [0., 90., 180., 270.] {
        let transform = Projection::translate(300., 300.)
            * Projection::rotate(f32::to_radians(degrees))
            * Projection::translate(-300., -300.);
        let rotated = warp(
            &image,
            transform,
            Interpolation::Bilinear,
            Border::Constant(Rgb([255; 3])),
        );
        let camera = geometry::locate_capture(&rotated)
            .unwrap_or_else(|| panic!("dim blue camera grid lost at {degrees} degrees"));
        for (actual, original) in [(camera[0][0], (150., 210.)), (camera[69][2], (450., 420.))] {
            let expected = transform * original;
            assert!(
                (actual.x - expected.0 as f64).hypot(actual.y - expected.1 as f64) < 6.,
                "dim camera crop changed cell identity at {degrees} degrees: {actual:?} vs {expected:?}"
            );
        }
    }
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
    for (i, cell) in full.cells.iter().enumerate() {
        let expected = ((b'A' + (i % 26) as u8) as char).to_string();
        assert_eq!(cell.text, expected, "live cell {i}: {cell:?}");
        assert!(
            cell.confidence >= 0.8,
            "live cell {i} did not qualify: {cell:?}"
        );
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
