use super::fit::{anchored, column_gaps, fit, HEADER, LABELS};
use super::*;
use image::{GrayImage, Luma};

/// The printed card in model units (cells; A1 top-left at the origin):
/// header band, alternating black/grey labels with light digits, pink and
/// white rows with dark "H"-shaped glyphs, white card around it.
fn printed(u: f64, v: f64, pink_rows: bool) -> [u8; 3] {
    let inside = |a: f64, lo: f64, hi: f64| a >= lo && a < hi;
    let glyph = |u: f64, v: f64| {
        let (x, y) = (u - u.floor() - 0.5, v - v.floor() - 0.5);
        (inside(x.abs(), 0.14, 0.22) && y.abs() < 0.3) || (x.abs() < 0.22 && y.abs() < 0.05)
    };
    if inside(u, 0., COLS) && inside(v, -HEADER, 0.) {
        let light = glyph(u, (v + HEADER) / HEADER * 1.);
        return if light { [235; 3] } else { [22; 3] };
    }
    if inside(u, -LABELS, 0.) && inside(v, 0., ROWS) {
        if glyph((u + LABELS) / LABELS, v) {
            return [235; 3];
        }
        return if (v as usize).is_multiple_of(2) {
            [28; 3]
        } else {
            [112; 3]
        };
    }
    if inside(u, 0., COLS) && inside(v, 0., ROWS) {
        if glyph(u, v) {
            return [25; 3];
        }
        return if pink_rows && (v as usize).is_multiple_of(2) {
            [232, 186, 188]
        } else {
            [238, 237, 234]
        };
    }
    if inside(u, -2.5, COLS + 1.5) && inside(v, -2.5, ROWS + 2.5) {
        return [248, 248, 246];
    }
    [70, 72, 75]
}

/// Planes of the card seen through `h` (model to image), 960 x 540.
fn render(h: &Homography, pink_rows: bool) -> Planes {
    let inverse = invert(h).unwrap();
    let rgb = image::RgbImage::from_fn(960, 540, |x, y| {
        // 2 x 2 supersampling keeps edges antialiased like a camera.
        let mut sum = [0u32; 3];
        for (dx, dy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
            let [u, v] = apply(&inverse, [x as f64 + dx - 0.5, y as f64 + dy - 0.5]);
            for (s, c) in sum.iter_mut().zip(printed(u, v, pink_rows)) {
                *s += c as u32;
            }
        }
        image::Rgb(sum.map(|s| (s / 4) as u8))
    });
    Planes::from_rgb(&rgb).unwrap()
}

/// A card pose: body width in pixels, rotation, centre and perspective tilt
/// (relative depth change from column A to column J).
fn pose(width: f64, degrees: f64, centre: [f64; 2], tilt: f64) -> Homography {
    let pitch = width / COLS;
    let (s, c) = degrees.to_radians().sin_cos();
    // Body rows are 0.71 of a column wide on the printed card.
    let (sx, sy) = (pitch, pitch * 0.71);
    let affine = [c * sx, -s * sy, 0., s * sx, c * sy, 0., tilt / COLS, 0., 1.];
    // Centre the body (5, 3.5) on `centre`.
    let mid = apply(&affine, [COLS / 2., ROWS / 2.]);
    let shift = [
        1.,
        0.,
        centre[0] - mid[0],
        0.,
        1.,
        centre[1] - mid[1],
        0.,
        0.,
        1.,
    ];
    multiply(&shift, &affine)
}

/// Worst body-corner error in cells.
fn error(found: &Homography, truth: &Homography) -> f64 {
    let (a, b) = (body(found), body(truth));
    (0..4).map(|i| distance(a[i], b[i])).fold(0., f64::max) / cell_px(truth)
}

#[test]
fn printed_card_is_fitted_with_its_indexing_in_any_orientation_and_perspective() {
    for (name, truth) in [
        ("level", pose(480., 0., [480., 270.], 0.)),
        ("tilted 18 degrees", pose(420., 18., [470., 280.], 0.)),
        ("perspective", pose(460., -6., [480., 270.], 0.4)),
        ("quarter turn", pose(300., 90., [480., 270.], 0.)),
        ("upside down", pose(440., 180., [480., 270.], 0.25)),
        ("three quarter turn", pose(300., 270., [480., 270.], 0.)),
        ("small and off centre", pose(250., 8., [320., 200.], 0.)),
    ] {
        let planes = render(&truth, true);
        let (_, found, info) = detect(&planes).unwrap_or_else(|| panic!("{name}: not found"));
        // The A1 corner must stay A1: a quarter turn may not re-index the grid.
        assert!(
            error(&found, &truth) < 0.1,
            "{name}: {:.3} cells",
            error(&found, &truth)
        );
        assert!(
            info.residual < 0.05 && info.anchor_margin >= 0.3,
            "{name}: {info:?}"
        );
        let cells = cells(body(&found).map(|[x, y]| [x / 960., y / 540.]), 960., 540.).unwrap();
        assert_eq!(cells.len(), 70);
        let expected = apply(&truth, [7.5, 4.5]);
        let quad = cells[4 * 10 + 7];
        let centre = [0, 1].map(|k| quad.iter().map(|p| p[k]).sum::<f64>() / 4.);
        assert!(
            distance(centre, expected) < 0.15 * cell_px(&truth),
            "{name}: H5 moved"
        );
    }
}

#[test]
fn shifted_squeezed_mirrored_and_colourless_grids_are_rejected() {
    let truth = pose(480., 4., [480., 270.], 0.15);
    let planes = render(&truth, true);
    let (score, margin) = anchored(&planes, &truth);
    assert!(score > 0. && margin >= 0.3, "{score} {margin}");
    // Off by one row or column, or half a cell: the anchors must say so.
    for (du, dv) in [
        (0., 1.),
        (0., -1.),
        (1., 0.),
        (-1., 0.),
        (0., 0.5),
        (0.5, 0.),
    ] {
        let shifted = multiply(&truth, &[1., 0., du, 0., 1., dv, 0., 0., 1.]);
        assert!(
            anchored(&planes, &shifted).1 < 0.3,
            "shift ({du}, {dv}) passed"
        );
    }
    // Ten columns squeezed into four: rows and edges can still line up.
    let squeezed = multiply(&truth, &[0.4, 0., 0., 0., 1., 0., 0., 0., 1.]);
    assert!(column_gaps(&planes, &truth) > 0.9);
    assert!(column_gaps(&planes, &squeezed) < 0.7);
    // Rows read the same top to bottom; only orientation rejects a mirror.
    let mirrored = multiply(&truth, &[1., 0., 0., 0., -1., ROWS, 0., 0., 1.]);
    assert!(fit(&planes, mirrored, &[0.2], None).0.is_none());
    assert!(fit(&planes, truth, &[0.2], None).0.is_some());
    // No pink rows (a monochrome copy) and a blank frame: nothing to fit.
    assert!(detect(&render(&truth, false)).is_none());
    let blank = Planes::new(
        GrayImage::from_pixel(960, 540, Luma([230])),
        vec![128; 960 * 540],
    );
    assert!(detect(&blank.unwrap()).is_none());
}

#[test]
fn tracker_follows_motion_refits_on_schedule_and_confirms_jumps() {
    let mut tracker = Tracker::default();
    let mut modes = Vec::new();
    for k in 0..20 {
        let t = k as f64 / 30.;
        let truth = pose(
            440. + k as f64,
            3. + 0.4 * k as f64,
            [460. + 3. * k as f64, 270.],
            0.15,
        );
        let step = tracker.step(&render(&truth, true), t);
        let grid = step.grid.unwrap_or_else(|| panic!("frame {k}: lost"));
        assert!(
            error(&grid, &truth) < 0.15,
            "frame {k}: {:.3}",
            error(&grid, &truth)
        );
        // Only a fresh fit may hand cells to recognition.
        assert_eq!(step.keyframe, step.mode != TrackMode::Tracked, "frame {k}");
        modes.push(step.mode);
    }
    assert_eq!(modes[0], TrackMode::Detected);
    assert!(
        modes[1..8].iter().all(|&m| m == TrackMode::Tracked),
        "{modes:?}"
    );
    assert_eq!(modes[8], TrackMode::Refitted, "{modes:?}");
    // A sudden jump in size and place needs a second agreeing detection.
    let moved = pose(300., 40., [300., 300.], 0.);
    let first = tracker.step(&render(&moved, true), 20. / 30.);
    assert_eq!(first.mode, TrackMode::Lost);
    let second = tracker.step(&render(&moved, true), 21. / 30.);
    assert_eq!(second.mode, TrackMode::Detected);
    assert!(error(&second.grid.unwrap(), &moved) < 0.1);
    // Display smoothing restarts with the new detection instead of gliding.
    let display = second.display.unwrap();
    assert!((0..4).all(|i| distance(display[i], body(&moved)[i]) < 0.1 * cell_px(&moved)));
    let blank = Planes::new(
        GrayImage::from_pixel(960, 540, Luma([230])),
        vec![128; 960 * 540],
    );
    assert_eq!(
        tracker.step(&blank.unwrap(), 22. / 30.).mode,
        TrackMode::Lost
    );
}
