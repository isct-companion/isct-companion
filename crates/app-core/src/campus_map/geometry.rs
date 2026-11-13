//! The little plane geometry the focus rule, taps and framing need, in flat
//! metres around the point asked about: at campus scale (a few hundred metres)
//! the error is far below what a finger or GPS resolves. Written here rather
//! than pulling in a geometry crate for a handful of functions.
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
    rings
        .next()
        .is_some_and(|outer| ring_contains(outer, point))
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

/// Whether any part of the polygons lies in the closed ring `area`: a vertex
/// inside it, one of its corners inside them, or an edge crossing its edges.
pub(super) fn multipolygon_meets_ring(polygons: &[Rings], area: &[Position]) -> bool {
    if area
        .iter()
        .any(|corner| multipolygon_contains(polygons, *corner))
    {
        return true;
    }
    for ring in polygons.iter().flatten() {
        if ring.iter().any(|point| ring_contains(area, *point)) {
            return true;
        }
        for pair in ring.windows(2) {
            for side in area.windows(2) {
                if segments_meet(pair[0], pair[1], side[0], side[1]) {
                    return true;
                }
            }
        }
    }
    false
}

/// The part of the map the person can see: the screen less whatever covers
/// it, as its four corners on the ground. The corners follow the screen, so
/// a rotated map gives a rotated box; widths are along its own sides.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Viewport {
    /// Top left, top right, bottom right, bottom left, then top left again.
    pub(super) ring: Vec<Position>,
    pub(super) center: Position,
    /// Metres along the top edge and down the left edge.
    width_m: f64,
    height_m: f64,
    /// Unit vectors in the tangent plane at the viewport centre.
    across: (f64, f64),
    down: (f64, f64),
    /// Metres one screen point spans.
    pub(super) metres_per_point: f64,
}

impl Viewport {
    /// `None` for a box with no area or no size on screen.
    pub(super) fn new(corners: [Position; 4], width_points: f64) -> Option<Self> {
        let center = [
            corners.iter().map(|corner| corner[0]).sum::<f64>() / 4.0,
            corners.iter().map(|corner| corner[1]).sum::<f64>() / 4.0,
        ];
        let [top_left, top_right, _, bottom_left] = corners;
        let length = |(x, y): (f64, f64)| (x * x + y * y).sqrt();
        // Use one tangent plane for scale and extent. Using a corner's
        // latitude makes the reported scale depend on the map's rotation.
        let start = metres(center, top_left);
        let vector = |to| {
            let end = metres(center, to);
            (end.0 - start.0, end.1 - start.1)
        };
        let across = vector(top_right);
        let down = vector(bottom_left);
        let width_m = length(across);
        let height_m = length(down);
        let metres_per_point = width_m / width_points;
        let valid = [width_m, height_m, metres_per_point]
            .iter()
            .all(|value| value.is_finite() && *value > 0.0);
        let mut ring = corners.to_vec();
        ring.push(top_left);
        valid.then_some(Self {
            ring,
            center,
            width_m,
            height_m,
            across: (across.0 / width_m, across.1 / width_m),
            down: (down.0 / height_m, down.1 / height_m),
            metres_per_point,
        })
    }

    /// How much of the viewport the polygons' extent spans: the larger of
    /// its share of the width and of the height, both measured along the
    /// viewport's own sides.
    pub(super) fn share(&self, polygons: &[Rings]) -> f64 {
        let (across, down) = (self.across, self.down);
        let mut span = [
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ];
        for point in polygons.iter().flatten().flatten() {
            let (x, y) = metres(self.center, *point);
            let along = x * across.0 + y * across.1;
            let below = x * down.0 + y * down.1;
            span[0] = span[0].min(along);
            span[1] = span[1].max(along);
            span[2] = span[2].min(below);
            span[3] = span[3].max(below);
        }
        if span[0] > span[1] {
            return 0.0;
        }
        ((span[1] - span[0]) / self.width_m).max((span[3] - span[2]) / self.height_m)
    }

    /// Whether any part of the polygons is in view.
    pub(super) fn meets(&self, polygons: &[Rings]) -> bool {
        multipolygon_meets_ring(polygons, &self.ring)
    }
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

/// `rect` grown about its centre until each side spans at least `metres`.
pub(super) fn at_least(rect: Rect, metres: f64) -> Rect {
    let center = [(rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0];
    let half_lng = metres / (2.0 * center[1].to_radians().cos() * METRES_PER_DEGREE_LNG);
    let half_lat = metres / (2.0 * METRES_PER_DEGREE_LAT);
    [
        rect[0].min(center[0] - half_lng),
        rect[1].min(center[1] - half_lat),
        rect[2].max(center[0] + half_lng),
        rect[3].max(center[1] + half_lat),
    ]
}

#[cfg(test)]
#[path = "../tests/campus_map__geometry.rs"]
mod tests;
