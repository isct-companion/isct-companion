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
    assert!(service.scan("", "", |_| {}).unwrap_err().contains("busy"));
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
    let preview =
        std::fs::read(std::env::var("MATRIX_PREVIEW").expect("Set MATRIX_PREVIEW")).unwrap();
    let service = MatrixOcrService::default();
    let mut previous_session = 0;
    for pass in 0..2 {
        let progress = std::sync::atomic::AtomicUsize::new(0);
        let start = std::time::Instant::now();
        let result = service
            .scan(&STANDARD.encode(&bytes), &STANDARD.encode(&preview), |n| {
                progress.store(n, std::sync::atomic::Ordering::Relaxed);
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
        serde_json::from_str(include_str!("fixtures/ts-filter.json")).unwrap();
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
fn prepared_canvas_pixels_are_never_resampled() {
    // Fine alternating strokes are changed by a reducing triangle kernel.
    // The native transport must preserve even these pixels exactly.
    let source = RgbImage::from_fn(73, 41, |x, y| {
        Rgb([(x % 2 * 255) as u8, (y % 2 * 255) as u8, 150])
    });
    let mut png = Cursor::new(vec![]);
    source.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let encoded = STANDARD.encode(png.into_inner());
    assert_eq!(decode_canvas(&encoded, 2200).unwrap(), source);
    assert!(decode_canvas(&encoded, 64).is_err());
    assert!(decode_canvas("not base64", 600).is_err());
    assert!(decode_canvas(&STANDARD.encode(b"not PNG"), 600).is_err());
    let service = MatrixOcrService::default();
    let preview = RgbImage::from_pixel(10, 10, Rgb([255; 3]));
    let mut png = Cursor::new(vec![]);
    preview.write_to(&mut png, image::ImageFormat::Png).unwrap();
    assert!(service
        .scan(&encoded, &STANDARD.encode(png.into_inner()), |_| {})
        .unwrap_err()
        .contains("dimensions"));
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
