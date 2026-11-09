#[derive(Clone, Copy, Debug)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

pub fn project(corners: &[Point; 4], u: f64, v: f64) -> Point {
    let [a, b, c, d] = *corners;
    let (dx, dy) = (a.x - b.x + c.x - d.x, a.y - b.y + c.y - d.y);
    let (bx, by, cx, cy) = (b.x - c.x, b.y - c.y, d.x - c.x, d.y - c.y);
    let determinant = bx * cy - cx * by;
    let (g, h) = if determinant.abs() < 1e-8 {
        (0., 0.)
    } else {
        (
            (dx * cy - cx * dy) / determinant,
            (bx * dy - dx * by) / determinant,
        )
    };
    let z = g * u + h * v + 1.;
    Point {
        x: ((b.x - a.x + g * b.x) * u + (d.x - a.x + h * d.x) * v + a.x) / z,
        y: ((b.y - a.y + g * b.y) * u + (d.y - a.y + h * d.y) * v + a.y) / z,
    }
}

#[cfg(test)]
pub(super) fn body_cells(corners: [Point; 4]) -> Vec<[Point; 4]> {
    (0..70)
        .map(|index| {
            let (column, row) = ((index % 10) as f64, (index / 10) as f64);
            [
                (column / 10., row / 7.),
                ((column + 1.) / 10., row / 7.),
                ((column + 1.) / 10., (row + 1.) / 7.),
                (column / 10., (row + 1.) / 7.),
            ]
            .map(|(u, v)| project(&corners, u, v))
        })
        .collect()
}
