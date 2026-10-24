use super::*;
use isct_core::matrix_scan::{MatrixCameraDriver, MatrixCameraPhase, MatrixYuvMatrix};
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
    let converted = crate::matrix_picture_from_yuv(&yuv).unwrap();
    assert_eq!(
        (
            decoded.width,
            decoded.height,
            decoded.analysis_width,
            decoded.analysis_height
        ),
        (
            converted.width,
            converted.height,
            converted.analysis_width,
            converted.analysis_height
        )
    );
    // Saturated edges clamp in YUV→RGB, so compare means, not single pixels.
    let mean = |a: &[u8], b: &[u8]| {
        a.iter()
            .zip(b)
            .map(|(a, b)| a.abs_diff(*b) as f64)
            .sum::<f64>()
            / a.len() as f64
    };
    let luma = mean(&decoded.luma, &converted.luma);
    let chroma = mean(&decoded.analysis_chroma, &converted.analysis_chroma);
    assert!(luma < 0.5 && chroma < 1., "luma {luma}, R−G {chroma}");

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
    assert!(crate::matrix_picture_from_yuv(&good).is_ok());
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
            matrix_vision().convert(&bad),
            Err(MatrixScanError::Unreadable(_))
        ));
    }
}
