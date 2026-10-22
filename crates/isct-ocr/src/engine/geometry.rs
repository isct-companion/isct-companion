use image::RgbImage;

#[derive(Clone, Copy, Debug)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

pub(super) fn fit(points: &[Point]) -> impl Fn(f64) -> f64 {
    let mut slopes = Vec::new();
    for (index, first) in points.iter().enumerate() {
        for second in &points[index + 1..] {
            if (second.x - first.x).abs() > 10. {
                slopes.push((second.y - first.y) / (second.x - first.x));
            }
        }
    }
    let slope = if slopes.is_empty() {
        0.
    } else {
        median(slopes)
    };
    let offset = median(
        points
            .iter()
            .map(|point| point.y - slope * point.x)
            .collect(),
    );
    move |x| offset + slope * x
}

// Match JavaScript Math.round, including negative half values.
fn round(value: f64) -> f64 {
    (value + 0.5).floor()
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

#[cfg(any(test, feature = "training-tools"))]
pub(super) fn body_cells(body: [Point; 4]) -> Vec<[Point; 4]> {
    (0..7)
        .flat_map(|row| {
            (0..10).map(move |column| {
                let left = column as f64 / 10.;
                let right = (column + 1) as f64 / 10.;
                let top = row as f64 / 7.;
                let bottom = (row + 1) as f64 / 7.;
                [
                    project(&body, left, top),
                    project(&body, right, top),
                    project(&body, right, bottom),
                    project(&body, left, bottom),
                ]
            })
        })
        .collect()
}

pub fn crop(image: &RgbImage, corners: &[Point; 4]) -> RgbImage {
    crop_at(image, corners, 64)
}

pub(super) fn crop_at(image: &RgbImage, corners: &[Point; 4], size: u32) -> RgbImage {
    image::ImageBuffer::from_fn(size, size, |x, y| {
        let point = project(
            corners,
            0.09 + x as f64 / size.saturating_sub(1).max(1) as f64 * 0.82,
            0.09 + y as f64 / size.saturating_sub(1).max(1) as f64 * 0.82,
        );
        *image.get_pixel(
            round(point.x).clamp(0., (image.width() - 1) as f64) as u32,
            round(point.y).clamp(0., (image.height() - 1) as f64) as u32,
        )
    })
}
