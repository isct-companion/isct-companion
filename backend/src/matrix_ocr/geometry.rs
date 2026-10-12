use image::RgbImage;

#[derive(Clone, Copy, Debug)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}
fn fit(points: &[Point]) -> impl Fn(f64) -> f64 {
    let mut slopes = vec![];
    for (i, a) in points.iter().enumerate() {
        for b in &points[i + 1..] {
            if (b.x - a.x).abs() > 10. {
                slopes.push((b.y - a.y) / (b.x - a.x));
            }
        }
    }
    let slope = if slopes.is_empty() {
        0.
    } else {
        median(slopes)
    };
    let offset = median(points.iter().map(|p| p.y - slope * p.x).collect());
    move |x| offset + slope * x
}
// Match JavaScript Math.round, including negative half values.
fn round(v: f64) -> f64 {
    (v + 0.5).floor()
}
fn dark(image: &RgbImage, x: f64, y: f64, threshold: u8) -> bool {
    let (x, y) = (round(x) as i64, round(y) as i64);
    x >= 0
        && y >= 0
        && x < image.width() as i64
        && y < image.height() as i64
        && image
            .get_pixel(x as u32, y as u32)
            .0
            .iter()
            .all(|v| *v < threshold)
}
#[derive(Clone, Copy)]
struct Run {
    y: f64,
    left: f64,
    right: f64,
}
/// White header letters can interrupt a wide horizontal run for several rows.
/// Bridge only a mostly dark band, never the light body between grid lines.
fn joins_header(image: &RgbImage, previous: Run, next: Run) -> bool {
    let dy = next.y - previous.y;
    if dy <= 4. {
        return true;
    }
    let span = (previous.right - previous.left).min(next.right - next.left);
    if dy > span / 10. || dy <= 0. {
        return false;
    }
    let left = previous.left.max(next.left) as u32;
    let right = previous.right.min(next.right) as u32;
    let mut hits = 0;
    let mut total = 0;
    for y in previous.y as u32 + 1..next.y as u32 {
        for x in left..=right {
            total += 1;
            if dark(image, x as f64, y as f64, 125) {
                hits += 1;
            }
        }
    }
    total > 0 && hits as f64 / total as f64 > 0.5
}

fn template(image: &RgbImage, recover_header_gaps: bool) -> Option<[Point; 4]> {
    let (w, h) = (image.width() as f64, image.height() as f64);
    let gap = round(w / 70.).max(3.) as i32;
    let mut runs = vec![];
    for y in 0..image.height() {
        let (mut left, mut last) = (-1, -1);
        for x in 0..=image.width() as i32 + gap {
            if (x as f64) < w && dark(image, x as f64, y as f64, 125) {
                if left < 0 {
                    left = x;
                }
                last = x;
            } else if left >= 0 && x - last > gap {
                if (last - left) as f64 > w * 0.2 {
                    runs.push(Run {
                        y: y as f64,
                        left: left as f64,
                        right: last as f64,
                    });
                }
                left = -1;
            }
        }
    }
    let mut groups: Vec<Vec<Run>> = vec![];
    for run in runs {
        if let Some(g) = groups.iter_mut().find(|g| {
            let last = g.last().unwrap();
            run.y >= last.y
                && (run.left - last.left).abs() < w * 0.06
                && (run.right - last.right).abs() < w * 0.06
                && (run.y - last.y <= 4.
                    || (recover_header_gaps && joins_header(image, *last, run)))
        }) {
            g.push(run);
        } else {
            groups.push(vec![run]);
        }
    }
    groups.retain(|g| {
        let span = median(g.iter().map(|r| r.right - r.left).collect());
        let height = g.last().unwrap().y - g[0].y + 1.;
        span / height > 7. && span / height < 28. && height >= 6.
    });
    let weight =
        |g: &Vec<Run>| g.len() as f64 * median(g.iter().map(|r| r.right - r.left).collect());
    groups.sort_by(|a, b| weight(b).total_cmp(&weight(a)));
    for group in groups {
        let left = median(group.iter().map(|r| r.left).collect());
        let right = median(group.iter().map(|r| r.right).collect());
        let cw = (right - left) / 10.;
        let mid = median(group.iter().map(|r| r.y).collect());
        let (mut tops, mut bottoms) = (vec![], vec![]);
        let mut x = left + cw * 0.1;
        while x < right - cw * 0.1 {
            let (mut top, mut bottom) = (mid, mid);
            while top > group[0].y - cw * 0.2 && dark(image, x, top, 125) {
                top -= 1.;
            }
            while bottom < group.last().unwrap().y + cw * 0.2 && dark(image, x, bottom, 125) {
                bottom += 1.;
            }
            if bottom - top > cw * 0.3 {
                tops.push(Point { x, y: top + 1. });
                bottoms.push(Point { x, y: bottom });
            }
            x += 2f64.max(cw * 0.17);
        }
        if bottoms.len() < 10 {
            continue;
        }
        let top_line = fit(&tops);
        let start_line = fit(&bottoms);
        let header_height = start_line((left + right) / 2.) - top_line((left + right) / 2.);
        if header_height < cw * 0.4 || header_height > cw * 1.1 {
            continue;
        }
        let mut ends = vec![];
        for fraction in [0.15, 0.25, 0.75, 0.85] {
            let x = left - cw * fraction;
            let start = start_line(x) + header_height * 0.3;
            let (mut last, mut missing, mut hits) = (start, 0., 0.);
            let mut y = round(start);
            while y < h.min(start + header_height * 9.) {
                if dark(image, x, y, 190) {
                    last = y;
                    missing = 0.;
                    hits += 1.;
                } else {
                    missing += 1.;
                }
                if missing > header_height * 0.45 {
                    break;
                }
                y += 1.;
            }
            let extent = last - start_line(x);
            if extent > header_height * 5. && extent < header_height * 8.5 && hits / extent > 0.65 {
                ends.push(Point { x, y: last });
            }
        }
        if ends.len() < 2 {
            continue;
        }
        let bottom_left = median(ends.iter().map(|p| p.y).collect());
        let body_height = bottom_left - start_line(left);
        let mut edges = vec![];
        let mut y = start_line(right) + body_height * 0.15;
        while y < bottom_left - body_height * 0.05 {
            let mut x = round(right + cw * 0.35);
            while x > right - cw * 0.25 {
                if dark(image, x, y, 155) {
                    edges.push(Point { x: y, y: x });
                    break;
                }
                x -= 1.;
            }
            y += 2f64.max(body_height / 30.);
        }
        if edges.len() < 10 {
            continue;
        }
        let right_edge = fit(&edges);
        let mut bottom_edges = vec![];
        let mut x = left + cw * 0.3;
        while x < right - cw * 0.3 {
            let mut y = round(bottom_left + header_height * 0.3);
            while y > bottom_left - header_height * 0.3 {
                if dark(image, x, y, 165) {
                    bottom_edges.push(Point { x, y });
                    break;
                }
                y -= 1.;
            }
            x += cw * 0.4;
        }
        let bottom_line = if bottom_edges.len() >= 10 {
            Some(fit(&bottom_edges))
        } else {
            None
        };
        let brx = right_edge(bottom_line.as_ref().map_or(bottom_left, |f| f(right)));
        let slope = (start_line(right) - start_line(left)) / (right - left);
        let corners = [
            Point {
                x: left,
                y: start_line(left),
            },
            Point {
                x: right,
                y: start_line(right),
            },
            Point {
                x: brx,
                y: bottom_line
                    .as_ref()
                    .map_or(bottom_left + slope * (brx - left), |f| f(brx)),
            },
            Point {
                x: left,
                y: bottom_line.as_ref().map_or(bottom_left, |f| f(left)),
            },
        ];
        if corners
            .iter()
            .all(|p| p.x >= 0. && p.y >= 0. && p.x < w && p.y < h)
        {
            return Some(corners);
        }
    }
    None
}

#[derive(Clone, Copy)]
struct Line {
    position: f64,
    slope: f64,
    support: f64,
}
fn detect_lines(image: &RgbImage, vertical: bool, count: usize) -> Option<Vec<Line>> {
    let (length, depth) = if vertical {
        (image.width() as usize, image.height() as usize)
    } else {
        (image.height() as usize, image.width() as usize)
    };
    let mut best: Vec<_> = (0..length)
        .map(|p| Line {
            position: p as f64,
            slope: 0.,
            support: 0.,
        })
        .collect();
    let mut edges = best.clone();
    for step in -16..=16 {
        let slope = step as f64 / 100.;
        let mut scores = vec![0.; length];
        let mut edge_scores = vec![0.; length];
        for cross in 0..depth {
            let shift = round(slope * (cross as f64 - depth as f64 / 2.)) as i32;
            for axis in 0..length {
                let (x, y) = if vertical {
                    (axis, cross)
                } else {
                    (cross, axis)
                };
                if *image.get_pixel(x as u32, y as u32).0.iter().max().unwrap() > 125 {
                    continue;
                }
                let position = axis as i32 - shift;
                if position < 0 || position >= length as i32 {
                    continue;
                }
                scores[position as usize] += 1.;
                if axis + 3 < length {
                    let (nx, ny) = if vertical { (x + 3, y) } else { (x, y + 3) };
                    if *image
                        .get_pixel(nx as u32, ny as u32)
                        .0
                        .iter()
                        .max()
                        .unwrap()
                        > 160
                    {
                        edge_scores[position as usize] += 1.;
                    }
                }
            }
        }
        for position in 0..length {
            let support = scores[position] / depth as f64;
            let edge_support = edge_scores[position] / depth as f64;
            if support > best[position].support {
                best[position] = Line {
                    position: position as f64,
                    slope,
                    support,
                };
            }
            if edge_support > edges[position].support {
                edges[position] = Line {
                    position: position as f64,
                    slope,
                    support: edge_support,
                };
            }
        }
    }
    let mut groups: Vec<Vec<Line>> = vec![];
    for line in best {
        if line.support < 0.38 {
            continue;
        }
        if let Some(last) = groups
            .last_mut()
            .filter(|g| line.position <= g.last().unwrap().position + 2.)
        {
            last.push(line);
        } else {
            groups.push(vec![line]);
        }
    }
    let mut candidates = vec![];
    for group in groups {
        let broad = group.last().unwrap().position - group[0].position
            >= length as f64 / (count * 2) as f64;
        let mut candidate = if broad {
            edges[group[0].position as usize]
        } else {
            group[0]
        };
        for line in group {
            let l = if broad {
                edges[line.position as usize]
            } else {
                line
            };
            if l.support > candidate.support {
                candidate = l;
            }
        }
        if !broad || candidate.support >= 0.38 {
            candidates.push(candidate);
        }
    }
    let (mut result, mut best_score) = (None, f64::NEG_INFINITY);
    for lines in candidates.windows(count) {
        let gap = (lines[count - 1].position - lines[0].position) / (count - 1) as f64;
        if gap < length as f64 / (count * 2) as f64 {
            continue;
        }
        let error = lines
            .iter()
            .enumerate()
            .map(|(i, l)| (l.position - lines[0].position - i as f64 * gap).abs())
            .fold(0., f64::max)
            / gap;
        if error > 0.18 {
            continue;
        }
        let score = lines.iter().map(|l| l.support).sum::<f64>() / count as f64 - error;
        if score > best_score {
            best_score = score;
            result = Some(lines.to_vec());
        }
    }
    result
}
fn intersection(v: Line, h: Line, width: f64, height: f64) -> Point {
    let vx = v.position - v.slope * height / 2.;
    let hy = h.position - h.slope * width / 2.;
    let x = (vx + v.slope * hy) / (1. - v.slope * h.slope);
    Point {
        x,
        y: hy + h.slope * x,
    }
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
pub fn locate(image: &RgbImage) -> Option<Vec<[Point; 4]>> {
    // Verify the complete projective grid before accepting the older header
    // estimate, which can look valid while shifting crops on a skewed card.
    if let Some(cells) = super::contours::locate(image) {
        return Some(cells);
    }
    if let Some(corners) = template(image, false).or_else(|| template(image, true)) {
        return Some(
            (0..70)
                .map(|i| {
                    let (c, r) = ((i % 10) as f64, (i / 10) as f64);
                    [
                        (c / 10., r / 7.),
                        ((c + 1.) / 10., r / 7.),
                        ((c + 1.) / 10., (r + 1.) / 7.),
                        (c / 10., (r + 1.) / 7.),
                    ]
                    .map(|(u, v)| project(&corners, u, v))
                })
                .collect(),
        );
    }
    let vertical = detect_lines(image, true, 11)?;
    let horizontal = detect_lines(image, false, 8)?;
    let (w, h) = (image.width() as f64, image.height() as f64);
    let corner = |c: usize, r: usize| intersection(vertical[c], horizontal[r], w, h);
    for r in 0..8 {
        for c in 0..11 {
            let p = corner(c, r);
            if p.x < 0.
                || p.y < 0.
                || p.x >= w
                || p.y >= h
                || (c > 0 && p.x - corner(c - 1, r).x < 4.)
                || (r > 0 && p.y - corner(c, r - 1).y < 4.)
            {
                return None;
            }
        }
    }
    Some(
        (0..70)
            .map(|i| {
                let (c, r) = (i % 10, i / 10);
                [
                    corner(c, r),
                    corner(c + 1, r),
                    corner(c + 1, r + 1),
                    corner(c, r + 1),
                ]
            })
            .collect(),
    )
}
pub fn crop(image: &RgbImage, corners: &[Point; 4]) -> RgbImage {
    image::ImageBuffer::from_fn(64, 64, |x, y| {
        let p = project(
            corners,
            0.09 + x as f64 / 63. * 0.82,
            0.09 + y as f64 / 63. * 0.82,
        );
        *image.get_pixel(
            round(p.x).clamp(0., (image.width() - 1) as f64) as u32,
            round(p.y).clamp(0., (image.height() - 1) as f64) as u32,
        )
    })
}
