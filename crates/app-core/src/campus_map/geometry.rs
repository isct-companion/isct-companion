//! The little plane geometry the focus rule and taps need, in flat metres
//! around the point asked about: at campus scale (a few hundred metres) the
//! error is far below what a finger or GPS resolves. Written here rather than
//! pulling in a geometry crate for four functions.
use super::export::{Position, Rings};

/// Metres per degree of latitude, and of longitude at the equator.
const METRES_PER_DEGREE_LAT: f64 = 110_574.0;
const METRES_PER_DEGREE_LNG: f64 = 111_320.0;

/// `point` in metres east and north of `origin`.
fn metres(origin: Position, point: Position) -> (f64, f64) {
    let scale = origin[1].to_radians().cos() * METRES_PER_DEGREE_LNG;
    (
        (point[0] - origin[0]) * scale,
        (point[1] - origin[1]) * METRES_PER_DEGREE_LAT,
    )
}

/// Even-odd ray cast; a point on an edge may land either side.
fn ring_contains(ring: &[Position], point: Position) -> bool {
    let mut inside = false;
    let mut previous = match ring.last() {
        Some(last) => *last,
        None => return false,
    };
    for current in ring {
        let (a, b) = (*current, previous);
        if (a[1] > point[1]) != (b[1] > point[1]) {
            let x = (b[0] - a[0]) * (point[1] - a[1]) / (b[1] - a[1]) + a[0];
            if point[0] < x {
                inside = !inside;
            }
        }
        previous = *current;
    }
    inside
}

/// Inside the outer ring and outside every hole.
pub(super) fn polygon_contains(rings: &Rings, point: Position) -> bool {
    let mut rings = rings.iter();
    rings.next().is_some_and(|outer| ring_contains(outer, point))
        && rings.all(|hole| !ring_contains(hole, point))
}

pub(super) fn multipolygon_contains(polygons: &[Rings], point: Position) -> bool {
    polygons
        .iter()
        .any(|polygon| polygon_contains(polygon, point))
}

fn ring_area(ring: &[Position]) -> f64 {
    let Some(origin) = ring.first().copied() else {
        return 0.0;
    };
    let mut twice = 0.0;
    for pair in ring.windows(2) {
        let (x0, y0) = metres(origin, pair[0]);
        let (x1, y1) = metres(origin, pair[1]);
        twice += x0 * y1 - x1 * y0;
    }
    (twice / 2.0).abs()
}

/// Square metres: the outer ring less its holes.
pub(super) fn polygon_area(rings: &Rings) -> f64 {
    let mut rings = rings.iter();
    let outer = rings.next().map_or(0.0, |ring| ring_area(ring.as_slice()));
    let holes: f64 = rings.map(|hole| ring_area(hole.as_slice())).sum();
    (outer - holes).max(0.0)
}

pub(super) fn multipolygon_area(polygons: &[Rings]) -> f64 {
    polygons.iter().map(polygon_area).sum()
}

/// Metres from `point` to the nearest edge of any ring.
pub(super) fn distance_to_edges(polygons: &[Rings], point: Position) -> f64 {
    let mut nearest = f64::INFINITY;
    for ring in polygons.iter().flatten() {
        for pair in ring.windows(2) {
            let (ax, ay) = metres(point, pair[0]);
            let (bx, by) = metres(point, pair[1]);
            let (dx, dy) = (bx - ax, by - ay);
            let length = dx * dx + dy * dy;
            let t = if length > 0.0 {
                (-(ax * dx + ay * dy) / length).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (x, y) = (ax + t * dx, ay + t * dy);
            nearest = nearest.min((x * x + y * y).sqrt());
        }
    }
    nearest
}

/// A box as `[west, south, east, north]`.
pub(super) type Rect = [f64; 4];

fn rect_contains(rect: Rect, point: Position) -> bool {
    point[0] >= rect[0] && point[0] <= rect[2] && point[1] >= rect[1] && point[1] <= rect[3]
}

/// Whether segments `a`–`b` and `c`–`d` cross or touch.
fn segments_meet(a: Position, b: Position, c: Position, d: Position) -> bool {
    let side = |p: Position, q: Position, r: Position| {
        (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
    };
    let (d1, d2) = (side(c, d, a), side(c, d, b));
    let (d3, d4) = (side(a, b, c), side(a, b, d));
    ((d1 > 0.0) != (d2 > 0.0) || d1 == 0.0 || d2 == 0.0)
        && ((d3 > 0.0) != (d4 > 0.0) || d3 == 0.0 || d4 == 0.0)
}

/// Whether any part of the polygons lies in the box: a vertex inside it, a
/// box corner inside them, or an edge crossing the box's edges.
pub(super) fn multipolygon_meets_rect(polygons: &[Rings], rect: Rect) -> bool {
    let corners: [Position; 4] = [
        [rect[0], rect[1]],
        [rect[2], rect[1]],
        [rect[2], rect[3]],
        [rect[0], rect[3]],
    ];
    if corners
        .iter()
        .any(|corner| multipolygon_contains(polygons, *corner))
    {
        return true;
    }
    for ring in polygons.iter().flatten() {
        if ring.iter().any(|point| rect_contains(rect, *point)) {
            return true;
        }
        for pair in ring.windows(2) {
            let sides = corners.iter().zip(corners.iter().cycle().skip(1));
            for (start, end) in sides {
                if segments_meet(pair[0], pair[1], *start, *end) {
                    return true;
                }
            }
        }
    }
    false
}

/// The box around every ring's points.
pub(super) fn bounds<'a>(rings: impl IntoIterator<Item = &'a Vec<Position>>) -> Option<Rect> {
    let mut found: Option<Rect> = None;
    for point in rings.into_iter().flatten() {
        let rect = found.get_or_insert([point[0], point[1], point[0], point[1]]);
        rect[0] = rect[0].min(point[0]);
        rect[1] = rect[1].min(point[1]);
        rect[2] = rect[2].max(point[0]);
        rect[3] = rect[3].max(point[1]);
    }
    found
}
