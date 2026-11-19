use super::*;

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

#[test]
fn malformed_native_frames_are_rejected_before_conversion() {
    let (y, uv) = (vec![0; 64 * 64], vec![128; 64 * 32]);
    let good = frame(64, 64, &y, &uv);
    assert!(matrix_analysis_from_yuv(&good).is_ok());
    assert!(matrix_full_luma_from_yuv(&good).is_ok());
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
            matrix_analysis_from_yuv(&bad),
            Err(MatrixOcrError::InvalidImage(_))
        ));
        assert!(matches!(
            matrix_full_luma_from_yuv(&bad),
            Err(MatrixOcrError::InvalidImage(_))
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
            let full = matrix_full_luma_from_yuv(&yuv).unwrap();
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
