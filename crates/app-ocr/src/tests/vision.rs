use super::*;
use app_core::matrix_scan::{MatrixCameraDriver, MatrixCameraPhase, MatrixYuvMatrix};
use std::time::Duration;

const CARD: &[u8] = include_bytes!("fixtures/matrix-card.jpg");

/// The fixture as a phone would deliver it: BT.601 full-range NV12 with
/// 2×2-averaged chroma, the subsampling a real camera applies.
fn nv12(rgb: &image::RgbImage) -> (Vec<u8>, Vec<u8>) {
    let (w, h) = (rgb.width() as usize, rgb.height() as usize);
    let y = rgb
        .pixels()
        .map(|p| (0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64).round() as u8)
        .collect();
    let mut uv = vec![128; w * h / 2];
    for cy in 0..h / 2 {
        for cx in 0..w / 2 {
            let [mut r, mut g, mut b] = [0.; 3];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = rgb.get_pixel((cx * 2 + dx) as u32, (cy * 2 + dy) as u32);
                r += p[0] as f64 / 4.;
                g += p[1] as f64 / 4.;
                b += p[2] as f64 / 4.;
            }
            let at = cy * w + cx * 2;
            uv[at] = (128. - 0.168736 * r - 0.331264 * g + 0.5 * b)
                .round()
                .clamp(0., 255.) as u8;
            uv[at + 1] = (128. + 0.5 * r - 0.418688 * g - 0.081312 * b)
                .round()
                .clamp(0., 255.) as u8;
        }
    }
    (y, uv)
}
fn frame<'a>(w: u32, h: u32, y: &'a [u8], uv: &'a [u8]) -> MatrixYuvFrame<'a> {
    MatrixYuvFrame {
        width: w,
        height: h,
        y,
        y_row_stride: w,
        u: uv,
        v: &uv[1..],
        uv_row_stride: w,
        uv_pixel_stride: 2,
        matrix: MatrixYuvMatrix::Bt601,
        full_range: true,
    }
}
fn letter(cell: usize) -> String {
    ((b'A' + (cell % 26) as u8) as char).to_string()
}

/// A still photo and live frames of the same card go through one detector,
/// one cut and one model. The photo reads every cell; the live session
/// confirms the same letters, and both pictures feed the tracker nearly the
/// same planes (only chroma subsampling separates them).
#[test]
fn a_photo_and_live_frames_of_one_card_read_the_same_letters() {
    let driver = MatrixCameraDriver::new(matrix_vision());
    let photo = driver.scan_photo(CARD, |_, _| true).unwrap();
    for (cell, vote) in photo.cells.iter().enumerate() {
        assert_eq!(vote.text, letter(cell), "photo cell {cell}");
    }

    let rgb = image::load_from_memory(CARD).unwrap().to_rgb8();
    let (y, uv) = nv12(&rgb);
    let yuv = frame(rgb.width(), rgb.height(), &y, &uv);
    let decoded = crate::matrix_picture(CARD).unwrap();
    let analysis = crate::matrix_analysis_from_yuv(&yuv).unwrap();
    let full = crate::matrix_full_luma_from_yuv(&yuv).unwrap();
    assert_eq!(
        (
            decoded.full.width,
            decoded.full.height,
            decoded.analysis.width,
            decoded.analysis.height
        ),
        (full.width, full.height, analysis.width, analysis.height)
    );
    // Saturated edges clamp in YUV→RGB, so compare means, not single pixels.
    let mean = |a: &[u8], b: &[u8]| {
        a.iter()
            .zip(b)
            .map(|(a, b)| a.abs_diff(*b) as f64)
            .sum::<f64>()
            / a.len() as f64
    };
    let luma = mean(&decoded.full.luma, &full.luma);
    let small = mean(&decoded.analysis.luma, &analysis.luma);
    let chroma = mean(&decoded.analysis.chroma, &analysis.chroma);
    assert!(
        luma < 0.5 && small < 1. && chroma < 1.,
        "luma {luma}, analysis luma {small}, R−G {chroma}"
    );

    let id = driver.begin().unwrap();
    let consumer = std::thread::scope(|scope| {
        let consumer = scope.spawn(|| driver.process(&id, |_| true));
        for frame_id in 0.. {
            let snapshot = driver
                .submit(&id, frame_id, &yuv, frame_id as f64 * 33.)
                .unwrap();
            if snapshot.phase == MatrixCameraPhase::Complete {
                break;
            }
            std::thread::sleep(Duration::from_millis(33));
        }
        consumer.join().unwrap()
    });
    let live = consumer.unwrap();
    let report = driver.stop(&id).unwrap();
    assert!(report.keyframes >= 3);
    // The ten-second budget is a release-build target measured by the
    // replay tools; a loaded test run may reach it first. Whatever the
    // session confirmed must still be the photo's letter, cell by cell.
    let confirmed = live.cells.iter().filter(|c| c.stable).count();
    assert!(
        confirmed >= 35 && (live.timed_out || confirmed == 70),
        "confirmed {confirmed}, timed out {}",
        live.timed_out
    );
    for (cell, (live, photo)) in live.cells.iter().zip(&photo.cells).enumerate() {
        if live.stable {
            assert_eq!(live.text, photo.text, "cell {cell}");
        }
    }
}

#[test]
fn malformed_native_frames_are_rejected_before_conversion() {
    let (y, uv) = (vec![0; 64 * 64], vec![128; 64 * 32]);
    let good = frame(64, 64, &y, &uv);
    assert!(matrix_vision().analyse(&good).is_ok());
    assert!(matrix_vision().full_luma(&good).is_ok());
    for bad in [
        MatrixYuvFrame {
            y_row_stride: 32,
            ..frame(64, 64, &y, &uv)
        },
        MatrixYuvFrame {
            y: &y[..100],
            ..frame(64, 64, &y, &uv)
        },
        MatrixYuvFrame {
            v: &uv[..10],
            ..frame(64, 64, &y, &uv)
        },
        MatrixYuvFrame {
            uv_pixel_stride: 0,
            ..frame(64, 64, &y, &uv)
        },
        frame(16, 16, &y, &uv),
    ] {
        assert!(matches!(
            matrix_vision().analyse(&bad),
            Err(MatrixScanError::Unreadable(_))
        ));
        assert!(matches!(
            matrix_vision().full_luma(&bad),
            Err(MatrixScanError::Unreadable(_))
        ));
    }
}

/// The cells, and so every training export, are cut from this plane: the
/// tabulated conversion must give byte for byte the luma of the per-pixel
/// YUV→RGB formula, in every matrix and range a camera reports.
#[test]
fn camera_full_luma_matches_the_per_pixel_conversion_exactly() {
    let (w, h) = (512usize, 256usize);
    let mut state = 20261108u32;
    let mut next = || {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        (state >> 24) as u8
    };
    let y: Vec<u8> = (0..w * h).map(|_| next()).collect();
    let uv: Vec<u8> = (0..w * h / 2).map(|_| next()).collect();
    for matrix in [MatrixYuvMatrix::Bt601, MatrixYuvMatrix::Bt709] {
        for full_range in [true, false] {
            let yuv = MatrixYuvFrame {
                matrix,
                full_range,
                ..frame(w as u32, h as u32, &y, &uv)
            };
            let (kr, kgu, kgv, kb) = match matrix {
                MatrixYuvMatrix::Bt601 => (1.402, 0.344136, 0.714136, 1.772),
                MatrixYuvMatrix::Bt709 => (1.5748, 0.187324, 0.468124, 1.8556),
            };
            let (ys, yo, cs) = if full_range {
                (1., 0., 1.)
            } else {
                (255. / 219., 16., 255. / 224.)
            };
            let clamp = |v: f64| v.round().clamp(0., 255.) as u8;
            let full = crate::matrix_full_luma_from_yuv(&yuv).unwrap();
            for (i, &actual) in full.luma.iter().enumerate() {
                let (px, py) = (i % w, i / w);
                let c = (py / 2) * w + (px / 2) * 2;
                let luma = (y[i] as f64 - yo) * ys;
                let cb = (uv[c] as f64 - 128.) * cs;
                let cr = (uv[c + 1] as f64 - 128.) * cs;
                let (r, g, b) = (
                    clamp(luma + kr * cr) as u32,
                    clamp(luma - kgu * cb - kgv * cr) as u32,
                    clamp(luma + kb * cb) as u32,
                );
                let expected = ((19595 * r + 38470 * g + 7471 * b + 32768) >> 16) as u8;
                assert_eq!(
                    actual, expected,
                    "{matrix:?} full {full_range} at {px},{py}"
                );
            }
        }
    }
}
