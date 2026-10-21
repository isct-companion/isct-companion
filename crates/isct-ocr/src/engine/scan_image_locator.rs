//! Grayscale, feature-first Matrix localization for still-image imports.
//!
//! Coarse position and angle come from the alternating left label strip.
//! Header, A1 paper and internal grid features then rank the matching body.
//! Camera capture keeps its existing detector for now.
use super::geometry;
use image::{GrayImage, Luma, RgbImage};
use imageproc::{
    geometric_transformations::{rotate_about_center_no_crop, Border, Interpolation},
    region_labelling::{connected_components, Connectivity},
};

const ROWS: usize = 7;
const COLS: usize = 10;

#[derive(Clone, Copy)]
struct Run {
    y: u32,
    left: f64,
    right: f64,
}

#[derive(Clone, Copy)]
struct Band {
    left: f64,
    right: f64,
    top: u32,
    bottom: u32,
}

#[derive(Clone)]
struct Candidate {
    body: [geometry::Point; 4],
    edges: AnchorEdges,
    score: f64,
}

#[derive(Clone, Copy)]
/// Four grayscale reference lines, kept separate from the selected cell quads.
pub(super) struct AnchorEdges {
    pub(super) header_top: Option<[geometry::Point; 2]>,
    pub(super) labels_left: Option<[geometry::Point; 2]>,
    pub(super) body_top: Option<[geometry::Point; 2]>,
    pub(super) body_left: Option<[geometry::Point; 2]>,
}

/// The selected 70 cells and optional annotation lines are independent outputs.
pub(super) struct LocatedGrid {
    pub(super) cells: Vec<[geometry::Point; 4]>,
    pub(super) edges: AnchorEdges,
}

#[derive(Clone, Copy)]
struct Blob {
    area: u32,
    min_x: u32,
    max_x: u32,
    min_y: u32,
    max_y: u32,
    sum_x: f64,
    sum_y: f64,
}

impl Blob {
    fn empty() -> Self {
        Self {
            area: 0,
            min_x: u32::MAX,
            max_x: 0,
            min_y: u32::MAX,
            max_y: 0,
            sum_x: 0.,
            sum_y: 0.,
        }
    }

    fn center(self) -> geometry::Point {
        geometry::Point {
            x: self.sum_x / self.area as f64,
            y: self.sum_y / self.area as f64,
        }
    }

    fn size(self) -> f64 {
        ((self.max_x - self.min_x + 1) * (self.max_y - self.min_y + 1)) as f64
    }

    fn height_along(self, axis: geometry::Point) -> f64 {
        let width = (self.max_x - self.min_x + 1) as f64;
        let height = (self.max_y - self.min_y + 1) as f64;
        axis.x.abs() * width + axis.y.abs() * height
    }
}

#[derive(Clone, Copy)]
struct Stripe {
    top: geometry::Point,
    first: geometry::Point,
    middle: geometry::Point,
    bottom: geometry::Point,
    dark_heights: [f64; 4],
    score: f64,
}

fn stripe_candidates(image: &GrayImage) -> Vec<Stripe> {
    let dark = GrayImage::from_fn(image.width(), image.height(), |x, y| {
        Luma([u8::from(image.get_pixel(x, y)[0] <= 90) * 255])
    });
    let labels = connected_components(&dark, Connectivity::Four, Luma([0]));
    let count = labels.as_raw().iter().copied().max().unwrap_or(0) as usize + 1;
    let mut components = vec![Blob::empty(); count];
    for (x, y, label) in labels.enumerate_pixels() {
        let id = label[0] as usize;
        if id == 0 {
            continue;
        }
        let blob = &mut components[id];
        blob.area += 1;
        blob.min_x = blob.min_x.min(x);
        blob.max_x = blob.max_x.max(x);
        blob.min_y = blob.min_y.min(y);
        blob.max_y = blob.max_y.max(y);
        blob.sum_x += x as f64;
        blob.sum_y += y as f64;
    }
    let mut blobs = components
        .into_iter()
        .filter(|blob| {
            if blob.area < 12 {
                return false;
            }
            let width = (blob.max_x - blob.min_x + 1) as f64;
            let height = (blob.max_y - blob.min_y + 1) as f64;
            (0.5..=2.0).contains(&(width / height)) && blob.area as f64 / blob.size() >= 0.35
        })
        .collect::<Vec<_>>();
    blobs.sort_by_key(|blob| std::cmp::Reverse(blob.area));
    blobs.truncate(350);
    let mut stripes = Vec::new();
    for (i, first) in blobs.iter().enumerate() {
        for (j, second) in blobs.iter().enumerate() {
            if i == j || !(0.5..=2.0).contains(&(first.area as f64 / second.area as f64)) {
                continue;
            }
            let (a, b) = (first.center(), second.center());
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let distance = dx.hypot(dy);
            let side = ((first.size().sqrt() + second.size().sqrt()) * 0.5).max(1.);
            if !(1.0..=3.3).contains(&(distance / side)) {
                continue;
            }
            let near = |target: geometry::Point| {
                blobs
                    .iter()
                    .enumerate()
                    .filter_map(|(index, blob)| {
                        if index == i
                            || index == j
                            || !(0.5..=2.0).contains(&(first.area as f64 / blob.area as f64))
                        {
                            return None;
                        }
                        let center = blob.center();
                        let error = (center.x - target.x).hypot(center.y - target.y);
                        (error < distance * 0.27).then_some((index, error))
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1))
            };
            let Some((third_index, error3)) = near(geometry::Point {
                x: b.x + dx,
                y: b.y + dy,
            }) else {
                continue;
            };
            let axis = geometry::Point {
                x: dx / distance,
                y: dy / distance,
            };
            // Perspective may change each label's height gradually. Reject
            // abrupt jumps, rather than requiring three identical heights.
            let heights = [
                first.height_along(axis),
                second.height_along(axis),
                blobs[third_index].height_along(axis),
            ];
            let smallest = heights.iter().copied().fold(f64::INFINITY, f64::min);
            let largest = heights.iter().copied().fold(0., f64::max);
            let height_growth_1 = (heights[1] / heights[0]).ln();
            let height_growth_2 = (heights[2] / heights[1]).ln();
            if smallest < 3.
                || largest / smallest > 1.8
                || (height_growth_1 - height_growth_2).abs() > 0.3 + 1. / smallest
            {
                continue;
            }
            let bottom = blobs[third_index].center();
            let next_distance = (bottom.x - b.x).hypot(bottom.y - b.y);
            let first_step = (distance / next_distance).clamp(0.8, 1.25);
            let top = geometry::Point {
                x: a.x - dx * first_step,
                y: a.y - dy * first_step,
            };
            let cross = geometry::Point {
                x: -dy / distance,
                y: dx / distance,
            };
            // Sample across each label so a white numeral in its middle does
            // not hide the dark fill. Relative contrast rejects wood texture
            // with three similarly placed dark flecks but no gray/black rhythm.
            let tone = |center: geometry::Point, dark: bool| {
                let mut values = [-0.3, -0.15, 0., 0.15, 0.3]
                    .into_iter()
                    .filter_map(|shift| {
                        sample(
                            image,
                            geometry::Point {
                                x: center.x + cross.x * side * shift,
                                y: center.y + cross.y * side * shift,
                            },
                        )
                    })
                    .collect::<Vec<_>>();
                if values.len() < 3 {
                    return None;
                }
                values.sort_by(f64::total_cmp);
                Some(
                    values[if dark {
                        values.len() / 4
                    } else {
                        values.len() / 2
                    }],
                )
            };
            let dark_centers = [top, a, b, bottom];
            let Some(dark_tones) = dark_centers
                .map(|center| tone(center, true))
                .into_iter()
                .collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            if dark_tones[0] > 110. {
                continue;
            }
            let mut weakest_contrast = f64::INFINITY;
            for step in 0..3 {
                let before = dark_centers[step];
                let after = dark_centers[step + 1];
                let between = |fraction: f64| geometry::Point {
                    x: before.x + (after.x - before.x) * fraction,
                    y: before.y + (after.y - before.y) * fraction,
                };
                let center = geometry::Point {
                    x: (before.x + after.x) * 0.5,
                    y: (before.y + after.y) * 0.5,
                };
                let Some(gray_tone) = tone(center, false) else {
                    weakest_contrast = 0.;
                    break;
                };
                let dark_ceiling = dark_tones[step].max(dark_tones[step + 1]);
                weakest_contrast = weakest_contrast.min(gray_tone - dark_ceiling);
                // The gray band has to persist across the interval, with dark
                // fill on both sides. Isolated flecks in wood fail this rhythm.
                for fraction in [0.15, 0.35, 0.65, 0.85] {
                    let should_be_dark = fraction < 0.2 || fraction > 0.8;
                    let Some(value) = tone(between(fraction), should_be_dark) else {
                        weakest_contrast = 0.;
                        break;
                    };
                    let contrast = if should_be_dark {
                        gray_tone - value
                    } else {
                        value - dark_ceiling
                    };
                    weakest_contrast = weakest_contrast.min(contrast);
                }
            }
            if weakest_contrast < 20. {
                continue;
            }
            let score = first.area.min(second.area).min(blobs[third_index].area) as f64
                * (weakest_contrast / 100.).clamp(0.5, 2.)
                / (largest / smallest)
                - error3 * side;
            stripes.push(Stripe {
                top,
                first: a,
                middle: b,
                bottom,
                dark_heights: [heights[0], heights[0], heights[1], heights[2]],
                score,
            });
        }
    }
    stripes.sort_by(|a, b| b.score.total_cmp(&a.score));
    stripes.dedup_by(|a, b| {
        (a.top.x - b.top.x).hypot(a.top.y - b.top.y) < 8.
            && (a.bottom.x - b.bottom.x).hypot(a.bottom.y - b.bottom.y) < 8.
    });
    stripes.truncate(8);
    stripes
}

pub(super) fn locate(image: &RgbImage) -> Option<Vec<[geometry::Point; 4]>> {
    locate_with_edges(image).map(|located| {
        let _reference_edges = located.edges;
        located.cells
    })
}

pub(super) fn locate_with_edges(image: &RgbImage) -> Option<LocatedGrid> {
    if image.width() < 32 || image.height() < 32 {
        return None;
    }
    // Keep the grayscale image at source resolution. Reference edges remain
    // separate from the cell quads and never alter the 70-cell selection.
    let gray = grayscale(image);
    // Start from the alternating left label strip. Its fitted axis determines
    // orientation even when the photo is tilted or quarter-turned.
    let stripes = stripe_candidates(&gray);
    if stripes.is_empty() {
        return locate_from_gray_grid(&gray);
    }
    let mut candidates = Vec::new();
    for stripe in stripes {
        let theta = (stripe.bottom.x - stripe.top.x).atan2(stripe.bottom.y - stripe.top.y) as f32;
        let raw = rotate_to_axis(&gray, theta);
        let upright_top = rotate_point(stripe.top, theta, gray.dimensions(), raw.dimensions());
        let upright_first = rotate_point(stripe.first, theta, gray.dimensions(), raw.dimensions());
        let upright_middle =
            rotate_point(stripe.middle, theta, gray.dimensions(), raw.dimensions());
        let upright_bottom =
            rotate_point(stripe.bottom, theta, gray.dimensions(), raw.dimensions());
        {
            let probe = &raw;
            for band in header_bands(probe) {
                if (band.left - upright_top.x).abs() > (upright_bottom.y - upright_top.y).abs() / 4.
                    || (band.bottom as f64 - upright_top.y).abs()
                        > (upright_bottom.y - upright_top.y).abs() / 4.
                {
                    continue;
                }
                let Some((fallback_body, j_edge_contrast, edges)) = segment_body(
                    probe,
                    band,
                    [upright_top, upright_first, upright_middle, upright_bottom],
                    stripe.dark_heights,
                ) else {
                    continue;
                };
                if !stripe_matches_body(
                    &fallback_body,
                    upright_top,
                    upright_first,
                    upright_middle,
                    upright_bottom,
                ) {
                    continue;
                }
                if let Some(mut candidate) = score_candidate(probe, fallback_body, edges) {
                    candidate.score += j_edge_contrast / 100.;
                    candidate.body = candidate.body.map(|point| {
                        unrotate_point(point, theta, gray.dimensions(), raw.dimensions())
                    });
                    candidate.edges = map_edges(candidate.edges, |point| {
                        unrotate_point(point, theta, gray.dimensions(), raw.dimensions())
                    });
                    candidates.push(candidate);
                }
            }
        }
    }

    candidates.sort_by(|a, b| b.score.total_cmp(&a.score));
    let Some(winner) = candidates.into_iter().next() else {
        return locate_from_gray_grid(&gray);
    };
    // The two measured outside edges are retained as independent metadata.
    // Only the matrix body quadrilateral determines the 70 cell selections.
    let cells = refine_grid_cells(&gray, &winner.body);
    Some(LocatedGrid {
        cells,
        edges: winner.edges,
    })
}

/// A grayscale-only compatibility path for clean cards whose left label
/// background is uniform instead of alternating. It measures the header and
/// the eight visible horizontal body seams; the feature-first label detector
/// remains authoritative when those labels are present.
fn locate_from_gray_grid(image: &GrayImage) -> Option<LocatedGrid> {
    let mut candidates = Vec::new();
    for turn in 0..4 {
        let rotated = match turn {
            0 => image.clone(),
            1 => image::imageops::rotate90(image),
            2 => image::imageops::rotate180(image),
            _ => image::imageops::rotate270(image),
        };
        for band in header_bands(&rotated) {
            let Some(threshold) = header_grid_threshold(&rotated, band) else {
                continue;
            };
            let mut lines = horizontal_grid_lines(&rotated, band, threshold);
            let body_edges = match lines.len() {
                7 => {
                    let mut edges = vec![band.bottom as f64];
                    edges.append(&mut lines);
                    edges
                }
                8 => lines,
                _ => continue,
            };
            let pitches = body_edges
                .windows(2)
                .map(|pair| pair[1] - pair[0])
                .collect::<Vec<_>>();
            let smallest = pitches.iter().copied().fold(f64::INFINITY, f64::min);
            let largest = pitches.iter().copied().fold(0., f64::max);
            if smallest < 4. || largest / smallest > 1.8 {
                continue;
            }
            let body = [
                geometry::Point {
                    x: band.left,
                    y: body_edges[0],
                },
                geometry::Point {
                    x: band.right,
                    y: body_edges[0],
                },
                geometry::Point {
                    x: band.right,
                    y: body_edges[ROWS],
                },
                geometry::Point {
                    x: band.left,
                    y: body_edges[ROWS],
                },
            ];
            let edges = AnchorEdges {
                header_top: Some([
                    geometry::Point {
                        x: band.left,
                        y: band.top as f64,
                    },
                    geometry::Point {
                        x: band.right,
                        y: band.top as f64,
                    },
                ]),
                labels_left: None,
                body_top: Some([body[0], body[1]]),
                body_left: Some([body[0], body[3]]),
            };
            let Some(mut candidate) = score_candidate(&rotated, body, edges) else {
                continue;
            };
            let (vertical, horizontal) = grid_support(&rotated, &body);
            if vertical < 0.45 || horizontal < 0.45 {
                continue;
            }
            candidate.score += vertical + horizontal;
            candidates.push((candidate, turn));
        }
    }
    let (candidate, turn) = candidates
        .into_iter()
        .max_by(|a, b| a.0.score.total_cmp(&b.0.score))?;
    let rotated = image_for_turn(image, turn);
    let cells = refine_grid_cells(&rotated, &candidate.body)
        .into_iter()
        .map(|cell| cell.map(|point| unrotate_quarter(point, turn, image.dimensions())))
        .collect();
    Some(LocatedGrid {
        cells,
        edges: map_edges(candidate.edges, |point| {
            unrotate_quarter(point, turn, image.dimensions())
        }),
    })
}

fn image_for_turn(image: &GrayImage, turn: usize) -> GrayImage {
    match turn {
        0 => image.clone(),
        1 => image::imageops::rotate90(image),
        2 => image::imageops::rotate180(image),
        _ => image::imageops::rotate270(image),
    }
}

fn unrotate_quarter(point: geometry::Point, turn: usize, original: (u32, u32)) -> geometry::Point {
    let (width, height) = (original.0 as f64 - 1., original.1 as f64 - 1.);
    match turn {
        0 => point,
        1 => geometry::Point {
            x: point.y,
            y: height - point.x,
        },
        2 => geometry::Point {
            x: width - point.x,
            y: height - point.y,
        },
        _ => geometry::Point {
            x: width - point.y,
            y: point.x,
        },
    }
}

fn header_grid_threshold(image: &GrayImage, band: Band) -> Option<u8> {
    let header_y = (band.top + band.bottom) as f64 * 0.5;
    let paper_y = (band.bottom + 4) as f64;
    let values = (0..10)
        .filter_map(|index| {
            let x = band.left + (band.right - band.left) * (index as f64 + 0.5) / 10.;
            Some((
                sample(image, geometry::Point { x, y: header_y })?,
                sample(image, geometry::Point { x, y: paper_y })?,
            ))
        })
        .collect::<Vec<_>>();
    if values.len() < 6 {
        return None;
    }
    let header = median(values.iter().map(|pair| pair.0).collect());
    let paper = median(values.iter().map(|pair| pair.1).collect());
    (paper - header >= 25.).then_some(((header + paper) * 0.5).clamp(60., 180.) as u8)
}

fn horizontal_grid_lines(image: &GrayImage, band: Band, threshold: u8) -> Vec<f64> {
    let left = band.left.round().max(0.) as u32;
    let right = band.right.round().min((image.width() - 1) as f64) as u32;
    if right <= left + 24 {
        return Vec::new();
    }
    let width = (right - left + 1) as f64;
    let mut lines = Vec::new();
    let mut start = None;
    let mut last = 0;
    for y in band.bottom.saturating_add(1)..image.height() {
        let dark = (left..=right)
            .filter(|&x| image.get_pixel(x, y)[0] <= threshold)
            .count() as f64
            / width;
        if dark >= 0.55 {
            start.get_or_insert(y);
            last = y;
        } else if let Some(first) = start.take() {
            lines.push((first as f64 + last as f64) * 0.5);
        }
    }
    if let Some(first) = start {
        lines.push((first as f64 + last as f64) * 0.5);
    }
    lines
}

fn map_edges(
    edges: AnchorEdges,
    mut map: impl FnMut(geometry::Point) -> geometry::Point,
) -> AnchorEdges {
    AnchorEdges {
        header_top: edges.header_top.map(|line| line.map(&mut map)),
        labels_left: edges.labels_left.map(|line| line.map(&mut map)),
        body_top: edges.body_top.map(|line| line.map(&mut map)),
        body_left: edges.body_left.map(|line| line.map(map)),
    }
}

/// The A-J header and 1-7 label strip are segmented in grayscale. Their
/// measured outer edges, including J's right edge and 7's bottom edge, form
/// the four extended body lines without trying cell-height hypotheses.
fn segment_body(
    image: &GrayImage,
    band: Band,
    stripe_centers: [geometry::Point; 4],
    dark_heights: [f64; 4],
) -> Option<([geometry::Point; 4], f64, AnchorEdges)> {
    let [stripe_top, stripe_first, _, _] = stripe_centers;
    if band.bottom + 8 >= image.height() || band.left < 6. {
        return None;
    }
    let mut header_values = (0..10)
        .filter_map(|i| {
            sample(
                image,
                geometry::Point {
                    x: band.left + (band.right - band.left) * (i as f64 + 0.5) / 10.,
                    y: (band.top + band.bottom) as f64 * 0.5,
                },
            )
        })
        .collect::<Vec<_>>();
    header_values.sort_by(f64::total_cmp);
    let header_ink = *header_values.get(header_values.len() / 4)?;
    let paper_values = (0..10)
        .filter_map(|i| {
            sample(
                image,
                geometry::Point {
                    x: band.left + (band.right - band.left) * (i as f64 + 0.5) / 10.,
                    y: (band.bottom + 4) as f64,
                },
            )
        })
        .collect::<Vec<_>>();
    if paper_values.is_empty() {
        return None;
    }
    let paper = median(paper_values);
    if paper - header_ink < 25. {
        return None;
    }
    let dark_limit = ((header_ink + paper) * 0.5).clamp(70., 130.);
    let near_left = band.left.round() as i32;
    let top_pitch = (stripe_first.y - stripe_top.y) * 0.5;
    let row1 = (stripe_top.y - top_pitch * 0.5).round().max(0.) as u32;
    let row1_ink = |x: i32| {
        if x < 0 || x >= image.width() as i32 {
            return None;
        }
        let values = (1..=3)
            .filter_map(|dy| {
                sample(
                    image,
                    geometry::Point {
                        x: x as f64,
                        y: (row1 + dy) as f64,
                    },
                )
            })
            .collect::<Vec<_>>();
        (!values.is_empty()).then(|| median(values))
    };
    let stripe_x = stripe_top.x.round() as i32;
    let seed = (stripe_x - 4..=stripe_x + 4)
        .filter(|&x| row1_ink(x).is_some_and(|value| value < dark_limit))
        .min_by_key(|&x| (x - stripe_x).abs())?;
    let mut label_left = seed;
    let mut gap = 0;
    for x in (0..seed).rev() {
        if row1_ink(x).is_some_and(|value| value < dark_limit) {
            label_left = x;
            gap = 0;
        } else {
            gap += 1;
            if gap > 3 {
                break;
            }
        }
    }
    if near_left - label_left < 5 {
        return None;
    }
    // Reuse the four dark bands measured by connected components. At small
    // sizes, numerals can make a fresh one-dimensional threshold scan merge
    // adjacent dark and gray rows even when the alternating stripe is clear.
    let pitches = [
        (stripe_centers[1].y - stripe_centers[0].y) * 0.5,
        (stripe_centers[1].y - stripe_centers[0].y) * 0.5,
        (stripe_centers[2].y - stripe_centers[1].y) * 0.5,
        (stripe_centers[3].y - stripe_centers[2].y) * 0.5,
    ];
    if pitches.iter().any(|&pitch| pitch <= 2.) {
        return None;
    }
    let dark_rows: [(u32, u32); 4] = std::array::from_fn(|index| {
        let center = stripe_centers[index].y;
        let half = dark_heights[index].min(pitches[index] * 1.15) * 0.5;
        (
            (center - half).floor().max(0.) as u32,
            (center + half).ceil().min((image.height() - 1) as f64) as u32,
        )
    });
    if dark_rows.windows(2).any(|pair| pair[0].1 >= pair[1].0) {
        return None;
    }
    let left = transition_edge(near_left - 5, near_left + 6, |x| {
        let values = dark_rows
            .iter()
            .flat_map(|&(first, last)| {
                [0.25, 0.5, 0.75].into_iter().filter_map(move |fraction| {
                    sample(
                        image,
                        geometry::Point {
                            x: x as f64,
                            y: first as f64 + (last - first) as f64 * fraction,
                        },
                    )
                })
            })
            .collect::<Vec<_>>();
        (!values.is_empty()).then(|| median(values))
    })?;
    let right = transition_edge(
        band.right.round() as i32 - 5,
        band.right.round() as i32 + 6,
        |x| {
            let values = (1..=7)
                .filter_map(|i| {
                    let y = band.top as f64 + (band.bottom - band.top) as f64 * i as f64 / 8.;
                    sample(image, geometry::Point { x: x as f64, y })
                })
                .collect::<Vec<_>>();
            (!values.is_empty()).then(|| median(values))
        },
    )?;
    // A white header letter can interrupt a dark scanline. J's outer edge is
    // different: the region beyond it stays light across the header height.
    let outside = [4., 8., 12.]
        .into_iter()
        .flat_map(|offset| {
            [0.25, 0.5, 0.75].into_iter().filter_map(move |fraction| {
                sample(
                    image,
                    geometry::Point {
                        x: right + offset,
                        y: band.top as f64 + (band.bottom - band.top) as f64 * fraction,
                    },
                )
            })
        })
        .collect::<Vec<_>>();
    let inside = [-4., -8., -12.]
        .into_iter()
        .flat_map(|offset| {
            [0.25, 0.5, 0.75].into_iter().filter_map(move |fraction| {
                sample(
                    image,
                    geometry::Point {
                        x: right + offset,
                        y: band.top as f64 + (band.bottom - band.top) as f64 * fraction,
                    },
                )
            })
        })
        .collect::<Vec<_>>();
    if outside.len() < 6 || inside.len() < 6 {
        return None;
    }
    let edge_contrast = median(outside) - median(inside);
    if edge_contrast < (paper - header_ink) * 0.5 {
        return None;
    }
    let top = transition_edge(band.bottom as i32 - 5, band.bottom as i32 + 6, |y| {
        let values = (0..COLS)
            .flat_map(|column| {
                [0.2, 0.5, 0.8].into_iter().filter_map(move |within| {
                    sample(
                        image,
                        geometry::Point {
                            x: band.left
                                + (band.right - band.left) * (column as f64 + within) / COLS as f64,
                            y: y as f64,
                        },
                    )
                })
            })
            .collect::<Vec<_>>();
        (!values.is_empty()).then(|| median(values))
    })?;
    let last_label_row = dark_rows[3].1 as i32;
    let bottom = transition_edge(last_label_row - 5, last_label_row + 7, |y| {
        let values = (1..=7)
            .filter_map(|i| {
                let x = label_left as f64 + (near_left - label_left) as f64 * i as f64 / 8.;
                sample(image, geometry::Point { x, y: y as f64 })
            })
            .collect::<Vec<_>>();
        (!values.is_empty()).then(|| median(values))
    })?;
    if right - left < 24. || bottom - top < 18. {
        return None;
    }
    let pitch = (stripe_first.y - stripe_top.y) * 0.5;
    // A row only a few pixels tall cannot supply a stable second edge.
    let measured_right = if pitch < 12. {
        right
    } else {
        row7_right_edge(image, dark_rows[3], left, right, pitch).unwrap_or(right)
    };
    let bottom_right = if (measured_right - right).abs() < (pitch * 0.25).max(2.) {
        right
    } else {
        measured_right
    };
    let mut top_right = right;
    if (bottom_right - right).abs() > pitch {
        let slopes = dark_rows[..3]
            .iter()
            .filter_map(|&row| {
                let (y, x) = upper_right_edge(image, row, left, right, bottom_right, pitch)?;
                (bottom - y > pitch).then_some((x - bottom_right) / (y - bottom))
            })
            .collect::<Vec<_>>();
        if slopes.len() >= 2 {
            let slope = median(slopes);
            let proposed = bottom_right + slope * (top - bottom);
            if proposed > left + pitch * 6. && proposed < bottom_right + pitch * 2. {
                top_right = proposed;
            }
        }
    }
    let (top_left_y, top_right_y) = if (bottom_right - right).abs() > pitch * 0.5 {
        let expected = stripe_top.y - pitch * 0.5;
        (
            top_edge_at_column(image, left, top_right, expected, pitch, false).unwrap_or(top),
            top_edge_at_column(image, left, top_right, expected, pitch, true).unwrap_or(top),
        )
    } else {
        (top, top)
    };
    // Measure header/label outside edges and body top/left edges independently
    // from grayscale transitions; cell pitch is not used to invent these lines.
    let header_top = detect_header_top_edge(image, band, pitch);
    let labels_left = detect_labels_left_edge(
        image,
        dark_rows,
        label_left as f64,
        left,
        top_left_y,
        bottom,
        pitch,
    );
    let fallback_body = [
        geometry::Point {
            x: left,
            y: top_left_y,
        },
        geometry::Point {
            x: top_right,
            y: top_right_y,
        },
        geometry::Point {
            x: bottom_right,
            y: bottom,
        },
        geometry::Point { x: left, y: bottom },
    ];
    let body_top = detect_body_top_edge(image, &fallback_body, pitch);
    let body_left = detect_body_left_edge(image, &fallback_body, pitch);
    Some((
        fallback_body,
        edge_contrast,
        AnchorEdges {
            header_top,
            labels_left,
            body_top,
            body_left,
        },
    ))
}

fn detect_header_top_edge(
    image: &GrayImage,
    band: Band,
    pitch: f64,
) -> Option<[geometry::Point; 2]> {
    let cell = (band.right - band.left) / COLS as f64;
    if cell < 3. || pitch < 4. {
        return None;
    }
    let measure = |first_column: usize| {
        let xs = (first_column..first_column + 2)
            .flat_map(|column| {
                [0.18, 0.34, 0.5, 0.66, 0.82]
                    .into_iter()
                    .map(move |fraction| band.left + cell * (column as f64 + fraction))
            })
            .collect::<Vec<_>>();
        let expected = band.top as f64;
        let profile = |y: i32| {
            let values = xs
                .iter()
                .filter_map(|&x| sample(image, geometry::Point { x, y: y as f64 }))
                .collect::<Vec<_>>();
            (values.len() >= xs.len() * 3 / 4).then(|| 255. - median(values))
        };
        let edge = transition_edge(
            (expected - pitch * 0.65).round() as i32,
            (expected + pitch * 0.65).round() as i32,
            profile,
        )?;
        let outside = profile((edge - pitch * 0.22).round() as i32)?;
        let inside = profile((edge + pitch * 0.22).round() as i32)?;
        (inside - outside >= 16.).then_some((xs.iter().sum::<f64>() / xs.len() as f64, edge))
    };
    let left = measure(0)?;
    let right = measure(COLS - 2)?;
    let slope = (right.1 - left.1) / (right.0 - left.0);
    Some([
        geometry::Point {
            x: band.left,
            y: left.1 + slope * (band.left - left.0),
        },
        geometry::Point {
            x: band.right,
            y: right.1 + slope * (band.right - right.0),
        },
    ])
}

fn detect_labels_left_edge(
    image: &GrayImage,
    dark_rows: [(u32, u32); 4],
    seed: f64,
    body_left: f64,
    body_top: f64,
    body_bottom: f64,
    pitch: f64,
) -> Option<[geometry::Point; 2]> {
    let points = dark_rows
        .into_iter()
        .filter_map(|(first, last)| {
            let height = (last - first).max(1) as f64;
            let ys = [0.25, 0.4, 0.6, 0.75].map(|fraction| first as f64 + height * fraction);
            let profile = |x: i32| {
                let values = ys
                    .iter()
                    .filter_map(|&y| sample(image, geometry::Point { x: x as f64, y }))
                    .collect::<Vec<_>>();
                (values.len() >= 3).then(|| 255. - median(values))
            };
            let edge = transition_edge(
                (seed - pitch * 0.9).round() as i32,
                (seed + pitch * 0.35).round() as i32,
                profile,
            )?;
            let outside = profile((edge - pitch * 0.2).round() as i32)?;
            let inside = profile((edge + pitch * 0.2).round() as i32)?;
            if inside - outside < 18. {
                return None;
            }
            Some(geometry::Point {
                x: (first + last) as f64 * 0.5,
                y: edge,
            })
        })
        .collect::<Vec<_>>();
    if points.len() < 3 {
        return None;
    }
    // Fit x as a function of y so mild perspective skew across the label strip
    // is measured from several black rows instead of copied from row 1.
    let fit_points = points
        .iter()
        .map(|point| geometry::Point {
            x: point.x,
            y: point.y,
        })
        .collect::<Vec<_>>();
    let edge_x = geometry::fit(&fit_points);
    let residual = median(
        points
            .iter()
            .map(|point| (edge_x(point.x) - point.y).abs())
            .collect(),
    );
    if residual > (pitch * 0.22).max(2.) {
        return None;
    }
    let top_x = edge_x(body_top);
    let bottom_x = edge_x(body_bottom);
    if top_x < 2.
        || bottom_x < 2.
        || top_x >= body_left - pitch * 0.2
        || bottom_x >= body_left - pitch * 0.2
    {
        return None;
    }
    Some([
        geometry::Point {
            x: top_x,
            y: body_top,
        },
        geometry::Point {
            x: bottom_x,
            y: body_bottom,
        },
    ])
}

/// Measure the matrix body's top boundary independently from the dark header.
/// Per-column gray transitions are fit into one reference line.
fn detect_body_top_edge(
    image: &GrayImage,
    body: &[geometry::Point; 4],
    pitch: f64,
) -> Option<[geometry::Point; 2]> {
    let cell_pitch = point_distance(body[0], body[1]) / COLS as f64;
    if cell_pitch < 3. || pitch < 4. {
        return None;
    }
    let mut points = Vec::with_capacity(COLS);
    for column in 0..COLS {
        let center_fraction = (column as f64 + 0.5) / COLS as f64;
        let center = between(body[0], body[1], center_fraction);
        let xs = [0.18, 0.38, 0.62, 0.82]
            .map(|offset| between(body[0], body[1], (column as f64 + offset) / COLS as f64).x);
        let profile = |y: i32| {
            let values = xs
                .iter()
                .filter_map(|&x| sample(image, geometry::Point { x, y: y as f64 }))
                .collect::<Vec<_>>();
            (values.len() >= 3).then(|| median(values))
        };
        let radius = (pitch * 0.42).round().max(3.) as i32;
        let Some(edge) = transition_edge(
            (center.y - radius as f64).round() as i32,
            (center.y + radius as f64).round() as i32,
            profile,
        ) else {
            continue;
        };
        points.push(geometry::Point {
            x: center.x,
            y: edge,
        });
    }
    fit_horizontal_edge(&points, body[0].x, body[1].x, (pitch * 0.28).max(2.))
}

/// Measure the matrix body's left boundary from the alternating label strip
/// into the light cell paper, then fit the measured transitions vertically.
fn detect_body_left_edge(
    image: &GrayImage,
    body: &[geometry::Point; 4],
    pitch: f64,
) -> Option<[geometry::Point; 2]> {
    let cell_width = point_distance(body[0], body[1]) / COLS as f64;
    let row_height = point_distance(body[0], body[3]) / ROWS as f64;
    if cell_width < 3. || row_height < 3. || pitch < 4. {
        return None;
    }
    let mut points = Vec::with_capacity(ROWS);
    for row in 0..ROWS {
        let center_fraction = (row as f64 + 0.5) / ROWS as f64;
        let center = between(body[0], body[3], center_fraction);
        let ys = [-0.16, 0., 0.16].map(|offset| center.y + row_height * offset);
        let profile = |x: i32| {
            let values = ys
                .iter()
                .filter_map(|&y| sample(image, geometry::Point { x: x as f64, y }))
                .collect::<Vec<_>>();
            (values.len() >= 2).then(|| median(values))
        };
        let radius = (cell_width * 0.42).round().max(3.) as i32;
        let Some(edge) = transition_edge(
            (center.x - radius as f64).round() as i32,
            (center.x + radius as f64).round() as i32,
            profile,
        ) else {
            continue;
        };
        points.push(geometry::Point {
            x: center.y,
            y: edge,
        });
    }
    fit_vertical_edge(&points, body[0].y, body[3].y, (cell_width * 0.28).max(2.))
}

fn fit_horizontal_edge(
    points: &[geometry::Point],
    left: f64,
    right: f64,
    max_residual: f64,
) -> Option<[geometry::Point; 2]> {
    if points.len() < 5 || (right - left).abs() < 10. {
        return None;
    }
    let line = geometry::fit(points);
    let residual = median(
        points
            .iter()
            .map(|point| (line(point.x) - point.y).abs())
            .collect(),
    );
    if residual > max_residual {
        return None;
    }
    Some([
        geometry::Point {
            x: left,
            y: line(left),
        },
        geometry::Point {
            x: right,
            y: line(right),
        },
    ])
}

fn fit_vertical_edge(
    points: &[geometry::Point],
    top: f64,
    bottom: f64,
    max_residual: f64,
) -> Option<[geometry::Point; 2]> {
    if points.len() < 4 || (bottom - top).abs() < 10. {
        return None;
    }
    let swapped = points
        .iter()
        .map(|point| geometry::Point {
            x: point.x,
            y: point.y,
        })
        .collect::<Vec<_>>();
    let line = geometry::fit(&swapped);
    let residual = median(
        points
            .iter()
            .map(|point| (line(point.x) - point.y).abs())
            .collect(),
    );
    if residual > max_residual {
        return None;
    }
    Some([
        geometry::Point {
            x: line(top),
            y: top,
        },
        geometry::Point {
            x: line(bottom),
            y: bottom,
        },
    ])
}

fn between(a: geometry::Point, b: geometry::Point, fraction: f64) -> geometry::Point {
    geometry::Point {
        x: a.x + (b.x - a.x) * fraction,
        y: a.y + (b.y - a.y) * fraction,
    }
}

fn point_distance(a: geometry::Point, b: geometry::Point) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

fn top_edge_at_column(
    image: &GrayImage,
    left: f64,
    right: f64,
    expected: f64,
    pitch: f64,
    last: bool,
) -> Option<f64> {
    let cell = (right - left) / COLS as f64;
    if cell < 3. {
        return None;
    }
    let column_left = if last { right - cell } else { left };
    transition_edge(
        (expected - pitch).round() as i32,
        (expected + pitch).round() as i32,
        |y| {
            let values = [0.15, 0.35, 0.65, 0.85]
                .into_iter()
                .filter_map(|fraction| {
                    sample(
                        image,
                        geometry::Point {
                            x: column_left + cell * fraction,
                            y: y as f64,
                        },
                    )
                })
                .collect::<Vec<_>>();
            (values.len() >= 3).then(|| median(values))
        },
    )
}

fn upper_right_edge(
    image: &GrayImage,
    row: (u32, u32),
    left: f64,
    header_right: f64,
    bottom_right: f64,
    pitch: f64,
) -> Option<(f64, f64)> {
    let run = (pitch * 0.5).round().clamp(5., 16.) as i32;
    let body_start = (left + pitch).round() as i32;
    let body_end = (header_right - pitch).round() as i32;
    let search_start = (header_right - pitch).round().max(0.) as i32;
    let search_end = (bottom_right + pitch * 2.)
        .round()
        .min((image.width() as i32 - run - 1) as f64) as i32;
    let y_start = (row.0 as f64 - pitch).floor().max(2.) as i32;
    let y_end = (row.1 as f64 + pitch * 0.25)
        .ceil()
        .min((image.height() - 3) as f64) as i32;
    let mut best = None;
    for y in (y_start..=y_end).step_by(2) {
        let profile = |x: i32| {
            let values = [-1, 0, 1]
                .into_iter()
                .filter_map(|dy| {
                    sample(
                        image,
                        geometry::Point {
                            x: x as f64,
                            y: (y + dy) as f64,
                        },
                    )
                })
                .collect::<Vec<_>>();
            (values.len() == 3).then(|| median(values))
        };
        let body_values = (body_start..body_end)
            .step_by(3)
            .filter_map(profile)
            .collect::<Vec<_>>();
        if body_values.len() < 8 {
            continue;
        }
        let mut body_values = body_values;
        body_values.sort_by(f64::total_cmp);
        let body_upper = body_values[body_values.len() * 3 / 4];
        let Some(outside_tone) = (search_start..=search_end - run)
            .step_by(2)
            .filter_map(|x| {
                let values = (0..run)
                    .step_by(2)
                    .filter_map(|offset| profile(x + offset))
                    .collect::<Vec<_>>();
                (values.len() >= 3).then(|| median(values))
            })
            .max_by(f64::total_cmp)
        else {
            continue;
        };
        let contrast = outside_tone - body_upper;
        if contrast < 15. {
            continue;
        }
        let light_limit = outside_tone - (contrast * 0.4).min(15.);
        let Some(plateau_start) = (search_start..=search_end).find(|&x| {
            (0..run).all(|offset| profile(x + offset).is_some_and(|value| value > light_limit))
        }) else {
            continue;
        };
        let edge = transition_edge(plateau_start - 5, plateau_start + 1, profile)
            .unwrap_or(plateau_start as f64 - 0.5);
        if best.is_none_or(|(score, _, _)| contrast > score) {
            best = Some((contrast, y as f64, edge));
        }
    }
    best.map(|(_, y, edge)| (y, edge))
}

fn row7_right_edge(
    image: &GrayImage,
    row: (u32, u32),
    left: f64,
    header_right: f64,
    pitch: f64,
) -> Option<f64> {
    let profile = |x: i32| {
        let values = [0.3, 0.45, 0.6, 0.75]
            .into_iter()
            .filter_map(|fraction| {
                sample(
                    image,
                    geometry::Point {
                        x: x as f64,
                        y: row.0 as f64 + (row.1 - row.0) as f64 * fraction,
                    },
                )
            })
            .collect::<Vec<_>>();
        (values.len() >= 3).then(|| median(values))
    };
    let body_start = (left + pitch).round() as i32;
    let body_end = (header_right - pitch).round() as i32;
    let body_values = (body_start..body_end)
        .step_by(3)
        .filter_map(profile)
        .collect::<Vec<_>>();
    if body_values.len() < 8 {
        return None;
    }
    let light_limit = median(body_values) + 15.;
    let run = (pitch * 0.5).round().clamp(5., 16.) as i32;
    let start = (header_right - pitch).round().max(0.) as i32;
    let end = (header_right + pitch * 10.)
        .round()
        .min((image.width() as i32 - run - 1) as f64) as i32;
    let plateau_start = (start..=end).find(|&x| {
        (0..run).all(|offset| profile(x + offset).is_some_and(|value| value > light_limit))
    })?;
    transition_edge(plateau_start - 5, plateau_start + 1, profile)
        .or(Some(plateau_start as f64 - 0.5))
}

fn transition_edge(
    mut first: i32,
    mut last: i32,
    profile: impl Fn(i32) -> Option<f64>,
) -> Option<f64> {
    first = first.max(2);
    last = last.max(first).min(i32::MAX - 2);
    let peak = (first..=last)
        .filter_map(|position| Some((position, profile(position + 2)? - profile(position - 2)?)))
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    if peak.1 < 10. {
        return None;
    }
    let mut weighted = 0.;
    let mut weight = 0.;
    for position in peak.0 - 2..peak.0 + 2 {
        let rise = (profile(position + 1)? - profile(position)?).max(0.);
        weighted += (position as f64 + 0.5) * rise;
        weight += rise;
    }
    (weight > 0.).then_some(weighted / weight)
}

fn stripe_matches_body(
    body: &[geometry::Point; 4],
    top: geometry::Point,
    first: geometry::Point,
    middle: geometry::Point,
    bottom: geometry::Point,
) -> bool {
    let top_pitch = (first.y - top.y) / 2.;
    let bottom_pitch = (bottom.y - middle.y) / 2.;
    let row_pitch = top_pitch.min(bottom_pitch);
    // Ten header columns must span several measured label-row pitches. This
    // rejects a small dark patch beside the strip without estimating height.
    let body_width = (body[1].x - body[0].x).abs();
    row_pitch > 2.
        && body_width > row_pitch * 7.
        && (body[0].x - top.x).abs() < top_pitch * 1.2
        && (top.y - body[0].y - top_pitch * 0.5).abs() < top_pitch * 0.7
        && (body[3].y - bottom.y - bottom_pitch * 0.5).abs() < bottom_pitch * 0.7
}

fn rotate_to_axis(image: &GrayImage, theta: f32) -> GrayImage {
    if theta.abs() < 0.005 {
        return image.clone();
    }
    rotate_about_center_no_crop(
        image,
        theta,
        Interpolation::Bilinear,
        Border::Constant(Luma([255])),
    )
}

fn rotate_point(
    point: geometry::Point,
    theta: f32,
    source: (u32, u32),
    destination: (u32, u32),
) -> geometry::Point {
    let (sine, cosine) = (theta as f64).sin_cos();
    let x = point.x - source.0 as f64 * 0.5;
    let y = point.y - source.1 as f64 * 0.5;
    geometry::Point {
        x: cosine * x - sine * y + destination.0 as f64 * 0.5,
        y: sine * x + cosine * y + destination.1 as f64 * 0.5,
    }
}

fn unrotate_point(
    point: geometry::Point,
    theta: f32,
    source: (u32, u32),
    destination: (u32, u32),
) -> geometry::Point {
    rotate_point(point, -theta, destination, source)
}

fn grayscale(image: &RgbImage) -> GrayImage {
    GrayImage::from_fn(image.width(), image.height(), |x, y| {
        let pixel = image.get_pixel(x, y);
        Luma([
            ((19595 * pixel[0] as u32 + 38470 * pixel[1] as u32 + 7471 * pixel[2] as u32 + 32768)
                >> 16) as u8,
        ])
    })
}

fn thresholds(image: &GrayImage) -> Vec<u8> {
    let mut histogram = [0usize; 256];
    for &pixel in image.as_raw() {
        histogram[pixel as usize] += 1;
    }
    let total = image.as_raw().len();
    let quantile = |percent: usize| {
        let target = total.saturating_mul(percent) / 100;
        let mut accumulated = 0;
        histogram
            .iter()
            .position(|count| {
                accumulated += count;
                accumulated >= target
            })
            .unwrap_or(128) as u8
    };
    let mut values = vec![
        imageproc::contrast::otsu_level(image),
        quantile(15),
        quantile(25),
        quantile(35),
        quantile(45),
        60,
        90,
        120,
        150,
    ];
    values.sort_unstable();
    values.dedup_by(|a, b| (*a).abs_diff(*b) < 8);
    values
}

fn header_bands(image: &GrayImage) -> Vec<Band> {
    let (width, height) = image.dimensions();
    let min_span = 24f64.max(width as f64 / 180.);
    let bridge = (width / 140).max(2);
    let mut result: Vec<Band> = Vec::new();
    for threshold in thresholds(image) {
        let mut groups: Vec<Vec<Run>> = Vec::new();
        for y in 0..height {
            let mut runs = Vec::new();
            let (mut start, mut last_dark) = (None, None);
            for x in 0..width {
                let dark = image.get_pixel(x, y)[0] <= threshold;
                if dark {
                    start.get_or_insert(x);
                    last_dark = Some(x);
                } else if let (Some(first), Some(last)) = (start, last_dark) {
                    if x.saturating_sub(last) > bridge {
                        if (last - first) as f64 >= min_span {
                            runs.push(Run {
                                y,
                                left: first as f64,
                                right: last as f64,
                            });
                        }
                        start = None;
                        last_dark = None;
                    }
                }
            }
            if let (Some(first), Some(last)) = (start, last_dark) {
                if (last - first) as f64 >= min_span {
                    runs.push(Run {
                        y,
                        left: first as f64,
                        right: last as f64,
                    });
                }
            }
            for run in runs {
                let span = run.right - run.left;
                let tolerance = (span * 0.12).max(3.);
                let max_gap = (span / 16.).ceil().clamp(3., 32.) as u32;
                let matching = groups.iter_mut().rev().find(|group| {
                    let Some(previous) = group.last() else {
                        return false;
                    };
                    run.y > previous.y
                        && run.y - previous.y <= max_gap
                        && (run.left - previous.left).abs() <= tolerance
                        && (run.right - previous.right).abs() <= tolerance
                });
                if let Some(group) = matching {
                    group.push(run);
                } else {
                    groups.push(vec![run]);
                }
            }
        }
        for group in groups {
            let Some(first) = group.first() else {
                continue;
            };
            let Some(last) = group.last() else {
                continue;
            };
            let left = median(group.iter().map(|run| run.left).collect());
            let right = median(group.iter().map(|run| run.right).collect());
            let span = right - left;
            let band_height = (last.y - first.y + 1) as f64;
            if span < min_span || band_height < 2. || !(5.0..=30.0).contains(&(span / band_height))
            {
                continue;
            }
            let band = Band {
                left,
                right,
                top: first.y,
                bottom: last.y,
            };
            if result.iter().any(|known| {
                (known.left - band.left).abs() < span * 0.12
                    && (known.right - band.right).abs() < span * 0.12
                    && known.top.abs_diff(band.top) < (span * 0.12).max(3.) as u32
            }) {
                continue;
            }
            result.push(band);
        }
    }
    result
}

fn score_candidate(
    gray: &GrayImage,
    body: [geometry::Point; 4],
    edges: AnchorEdges,
) -> Option<Candidate> {
    let (header, labels, a1, label_rows) = anchor_levels(gray, &body)?;
    let header_contrast = a1 - header;
    let label_contrast = a1 - labels;
    if header_contrast < 14. || label_contrast < 14. {
        return None;
    }
    // The stripe and header already establish the frame. Thin grid lines may
    // disappear at low resolution, so they rank candidates without vetoing one.
    let (vertical_grid, horizontal_grid) = grid_support(gray, &body);
    let alternating = alternating_rows(&label_rows);
    let grid = (vertical_grid + horizontal_grid) * 0.5;
    let score =
        (header_contrast.min(100.) + label_contrast.min(100.)) / 100. + grid + alternating * 0.25;
    Some(Candidate { body, edges, score })
}

fn anchor_levels(
    image: &GrayImage,
    body: &[geometry::Point; 4],
) -> Option<(f64, f64, f64, Vec<f64>)> {
    let mut header = Vec::with_capacity(COLS * 2);
    for column in 0..COLS {
        let center = (column as f64 + 0.5) / COLS as f64;
        for offset in [-0.2, 0.2] {
            header.push(sample(
                image,
                geometry::project(body, center + offset / COLS as f64, -0.5 / ROWS as f64),
            )?);
        }
    }
    let mut labels = Vec::with_capacity(ROWS * 2);
    let mut label_rows = Vec::with_capacity(ROWS);
    for row in 0..ROWS {
        let center = (row as f64 + 0.5) / ROWS as f64;
        let mut samples = Vec::with_capacity(2);
        for offset in [-0.2, 0.2] {
            samples.push(sample(
                image,
                geometry::project(body, -0.5 / COLS as f64, center + offset / ROWS as f64),
            )?);
        }
        label_rows.push((samples[0] + samples[1]) / 2.);
        labels.extend(samples);
    }
    // Four off-center samples measure the bright cell paper around its glyph.
    let a1_samples = [0.18, 0.82]
        .into_iter()
        .flat_map(|u| [0.18, 0.82].into_iter().map(move |v| (u, v)))
        .map(|(u, v)| {
            sample(
                image,
                geometry::project(body, u / COLS as f64, v / ROWS as f64),
            )
        })
        .collect::<Option<Vec<_>>>()?;
    Some((
        median(header),
        median(labels),
        median(a1_samples),
        label_rows,
    ))
}

fn alternating_rows(values: &[f64]) -> f64 {
    if values.len() < 3 {
        return 0.;
    }
    let signs: Vec<_> = values
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).signum())
        .collect();
    let changes = signs
        .windows(2)
        .filter(|pair| pair[0] * pair[1] < 0.)
        .count();
    changes as f64 / signs.len().saturating_sub(1).max(1) as f64
}

fn grid_support(image: &GrayImage, body: &[geometry::Point; 4]) -> (f64, f64) {
    let mut vertical = 0;
    for column in 1..COLS {
        let mut hits = 0;
        let mut total = 0;
        for row in 0..ROWS {
            for offset in [0.22, 0.5, 0.78] {
                let v = (row as f64 + offset) / ROWS as f64;
                let center = column as f64 / COLS as f64;
                if line_is_dark(image, body, center, v, true) {
                    hits += 1;
                }
                total += 1;
            }
        }
        if hits as f64 / total as f64 >= 0.35 {
            vertical += 1;
        }
    }
    let mut horizontal = 0;
    for row in 1..ROWS {
        let mut hits = 0;
        let mut total = 0;
        for column in 0..COLS {
            for offset in [0.22, 0.5, 0.78] {
                let u = (column as f64 + offset) / COLS as f64;
                let center = row as f64 / ROWS as f64;
                if line_is_dark(image, body, u, center, false) {
                    hits += 1;
                }
                total += 1;
            }
        }
        if hits as f64 / total as f64 >= 0.35 {
            horizontal += 1;
        }
    }
    (
        vertical as f64 / (COLS - 1) as f64,
        horizontal as f64 / (ROWS - 1) as f64,
    )
}

fn refine_grid_cells(image: &GrayImage, body: &[geometry::Point; 4]) -> Vec<[geometry::Point; 4]> {
    let rows = label_row_offsets(image, body);
    let points: Vec<_> = (0..=ROWS)
        .map(|row| {
            (0..=COLS)
                .map(|column| {
                    geometry::project(
                        body,
                        column as f64 / COLS as f64,
                        row as f64 / ROWS as f64 + rows[row],
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let mut cells = Vec::with_capacity(ROWS * COLS);
    for row in 0..ROWS {
        for column in 0..COLS {
            cells.push([
                points[row][column],
                points[row][column + 1],
                points[row + 1][column + 1],
                points[row + 1][column],
            ]);
        }
    }
    cells
}

/// Row seams follow the alternating grayscale bands in the left label strip.
/// Sampling beside the numeral strokes makes row positioning independent of
/// internal horizontal grid-line contrast.
fn label_row_offsets(image: &GrayImage, body: &[geometry::Point; 4]) -> [f64; ROWS + 1] {
    let mut offsets = [0.; ROWS + 1];
    for (line, offset) in offsets.iter_mut().enumerate().take(ROWS).skip(1) {
        let expected_direction = if line % 2 == 1 { 1. } else { -1. };
        let nominal = line as f64 / ROWS as f64;
        let delta = 0.14 / ROWS as f64;
        let mut best_score = f64::NEG_INFINITY;
        let mut best_offset = 0.;
        for step in -24..=24 {
            let candidate_offset = step as f64 * 0.01 / ROWS as f64;
            let seam = nominal + candidate_offset;
            let before = label_strip_tone(image, body, seam - delta);
            let after = label_strip_tone(image, body, seam + delta);
            let (Some(before), Some(after)) = (before, after) else {
                continue;
            };
            let score = expected_direction * (after - before);
            if score > best_score {
                best_score = score;
                best_offset = candidate_offset;
            }
        }
        // Keep the body geometry seam if the label strip has no clear transition.
        if best_score >= 8. {
            *offset = best_offset;
        }
    }
    offsets
}

fn label_strip_tone(image: &GrayImage, body: &[geometry::Point; 4], v: f64) -> Option<f64> {
    let values = [0.08, 0.18, 0.28, 0.72, 0.82, 0.92]
        .into_iter()
        .filter_map(|fraction| sample(image, geometry::project(body, -fraction / COLS as f64, v)))
        .collect::<Vec<_>>();
    (values.len() >= 4).then(|| median(values))
}

fn line_is_dark(
    image: &GrayImage,
    body: &[geometry::Point; 4],
    u: f64,
    v: f64,
    vertical: bool,
) -> bool {
    let pitch = if vertical {
        1. / COLS as f64
    } else {
        1. / ROWS as f64
    };
    let (left, right) = if vertical {
        (u - pitch * 0.12, u + pitch * 0.12)
    } else {
        (v - pitch * 0.12, v + pitch * 0.12)
    };
    let center = geometry::project(body, u, v);
    let a = if vertical {
        geometry::project(body, left, v)
    } else {
        geometry::project(body, u, left)
    };
    let b = if vertical {
        geometry::project(body, right, v)
    } else {
        geometry::project(body, u, right)
    };
    let (Some(ink), Some(side_a), Some(side_b)) =
        (sample(image, center), sample(image, a), sample(image, b))
    else {
        return false;
    };
    ink + 8. < (side_a + side_b) * 0.5
}

fn sample(image: &GrayImage, point: geometry::Point) -> Option<f64> {
    if !point.x.is_finite()
        || !point.y.is_finite()
        || point.x < 0.
        || point.y < 0.
        || point.x >= image.width() as f64
        || point.y >= image.height() as f64
    {
        return None;
    }
    let x = point.x.round().clamp(0., (image.width() - 1) as f64) as u32;
    let y = point.y.round().clamp(0., (image.height() - 1) as f64) as u32;
    Some(image.get_pixel(x, y)[0] as f64)
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}
