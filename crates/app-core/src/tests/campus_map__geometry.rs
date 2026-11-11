//! The plane geometry the focus rule and taps rest on. Shapes here are
//! squares in metres, not map data: the rules over the published export are
//! in `campus_map__page.rs`.
use super::*;

const ORIGIN: Position = [139.68, 35.6];

/// `[east, north]` metres from [`ORIGIN`], as a position.
fn at(east: f64, north: f64) -> Position {
    let scale = ORIGIN[1].to_radians().cos() * METRES_PER_DEGREE_LNG;
    [
        ORIGIN[0] + east / scale,
        ORIGIN[1] + north / METRES_PER_DEGREE_LAT,
    ]
}

/// A square of `side` metres centred `east`/`north` of the origin.
fn square(east: f64, north: f64, side: f64) -> Vec<Rings> {
    let h = side / 2.0;
    vec![vec![vec![
        at(east - h, north - h),
        at(east + h, north - h),
        at(east + h, north + h),
        at(east - h, north + h),
        at(east - h, north - h),
    ]]]
}

/// A view `width` × `height` metres centred on the origin, turned by
/// `quarter_turns` × 90° (a rotated map).
fn view(width: f64, height: f64, quarter_turns: usize, width_points: f64) -> Viewport {
    let (w, h) = (width / 2.0, height / 2.0);
    let mut corners = [(-w, h), (w, h), (w, -h), (-w, -h)];
    for _ in 0..quarter_turns {
        for corner in &mut corners {
            *corner = (corner.1, -corner.0);
        }
    }
    let corners = corners.map(|(east, north)| at(east, north));
    Viewport::new(corners, width_points).expect("a view with area")
}

#[test]
fn a_view_measures_along_its_own_sides_however_the_map_is_turned() {
    let building = square(0.0, 0.0, 50.0);
    for turns in 0..4 {
        let upright = view(100.0, 200.0, turns, 400.0);
        // 50 m of a 100 m wide view, whichever way north points.
        assert!(
            (upright.share(&building) - 0.5).abs() < 1e-6,
            "turns {turns}"
        );
        assert!(
            (upright.metres_per_point - 0.25).abs() < 1e-6,
            "turns {turns}"
        );
        assert!(upright.meets(&building));
        assert!(!upright.meets(&square(500.0, 0.0, 50.0)));
    }
    // A view inside one large building still meets it: no vertex of either
    // lies in the other's edges.
    assert!(view(20.0, 20.0, 1, 100.0).meets(&square(0.0, 0.0, 300.0)));
    // A box with no width is no view.
    assert!(Viewport::new([ORIGIN; 4], 390.0).is_none());
}

#[test]
fn containment_respects_holes_and_distance_is_to_the_nearest_edge() {
    let mut courtyard = square(0.0, 0.0, 100.0);
    courtyard[0].push(square(0.0, 0.0, 40.0)[0][0].clone());
    assert!(multipolygon_contains(&courtyard, at(35.0, 0.0)));
    assert!(!multipolygon_contains(&courtyard, at(0.0, 0.0)));
    let outside = at(60.0, 0.0);
    assert!(!multipolygon_contains(&courtyard, outside));
    assert!((distance_to_edges(&courtyard, outside) - 10.0).abs() < 0.01);
}
