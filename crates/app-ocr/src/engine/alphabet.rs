use super::{decode_image, geometry, illumination, model_error};
use crate::{MatrixCaptureFrame, MatrixCaptureGlyph, MatrixOcrError, Recognition};
use image::{imageops::FilterType, GrayImage, Rgb, RgbImage};
use imageproc::geometric_transformations::{warp_into, Border, Interpolation, Projection};
use ort::{session::Session, value::Tensor};
use std::sync::{Arc, Mutex, TryLockError};

const SIZE: usize = 96;
const MODEL: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/matrix-alphabet.onnx"));

/// Locate ink without discarding a fixed fraction of a cell. Keep grayscale
/// pixels for inference; the binary mask is only used to separate grid edges.
fn normalize_glyph(gray: &GrayImage) -> Option<GrayImage> {
    use image::Luma;
    use imageproc::region_labelling::{connected_components, Connectivity};
    let lighting = illumination::analyze(gray);
    let visible = lighting.corrected.as_ref().unwrap_or(gray);
    let threshold = imageproc::contrast::otsu_level(visible);
    let mut mask = GrayImage::from_fn(96, 96, |x, y| {
        Luma([if visible.get_pixel(x, y)[0] <= threshold {
            255
        } else {
            0
        }])
    });
    // Separate an actual grid/ink join only when needed. Splitting an already
    // separate grid could release small border fragments into the glyph bounds.
    let initial = connected_components(&mask, Connectivity::Eight, Luma([0]));
    let mut edges = vec![false; initial.as_raw().iter().copied().max().unwrap_or(0) as usize + 1];
    for (x, y, label) in initial.enumerate_pixels() {
        if label[0] != 0 && (x == 0 || y == 0 || x == 95 || y == 95) {
            edges[label[0] as usize] = true;
        }
    }
    let joined = initial
        .enumerate_pixels()
        .filter(|(x, y, label)| {
            (14..82).contains(x) && (14..82).contains(y) && edges[label[0] as usize]
        })
        .count()
        >= 8;
    // This changes only the mask, never the grayscale model pixels.
    let rows: Vec<_> = (0..96)
        .filter(|&y| {
            joined
                && !(6..90).contains(&y)
                && (0..96).filter(|&x| mask.get_pixel(x, y)[0] != 0).count() >= 87
        })
        .collect();
    let columns: Vec<_> = (0..96)
        .filter(|&x| {
            joined
                && !(6..90).contains(&x)
                && (0..96).filter(|&y| mask.get_pixel(x, y)[0] != 0).count() >= 87
        })
        .collect();
    for y in rows {
        for x in 0..96 {
            mask.put_pixel(x, y, Luma([0]));
        }
    }
    for x in columns {
        for y in 0..96 {
            mask.put_pixel(x, y, Luma([0]));
        }
    }
    let labels = if joined {
        connected_components(&mask, Connectivity::Eight, Luma([0]))
    } else {
        initial
    };
    // (left, top, right, bottom, area, central area, pixels on the bottom row)
    let mut components = vec![
        (96u32, 96u32, 0u32, 0u32, 0usize, 0usize, 0usize);
        labels.as_raw().iter().copied().max().unwrap_or(0) as usize + 1
    ];
    for (x, y, label) in labels.enumerate_pixels() {
        if label[0] == 0 {
            continue;
        }
        let component = &mut components[label[0] as usize];
        component.0 = component.0.min(x);
        component.1 = component.1.min(y);
        component.2 = component.2.max(x);
        component.3 = component.3.max(y);
        component.4 += 1;
        if (14..82).contains(&x) && (14..82).contains(&y) {
            component.5 += 1;
        }
        if y == 95 {
            component.6 += 1;
        }
    }
    let (mut left, mut top, mut right, mut bottom) = (96, 96, 0, 0);
    for (x0, y0, x1, y1, area, interior, bottom_contact) in components.into_iter().skip(1) {
        if area < 8 {
            continue;
        }
        // A descender: central ink leaving the cell only through a narrow
        // contact at the bottom. The Q tail of this typeface reaches the row
        // line; on recorded cards 30% of sharp Q cells were rejected here, ten
        // times any other letter. Q tails touch up to about 18 px, while a bar
        // cut by the edge (E losing its lower bar would read F) touches wider,
        // and any contact with another side stays a possible clipped letter.
        let descender = y1 == 95 && x0 > 0 && y0 > 0 && x1 < 95 && bottom_contact <= 18;
        if !descender && (x0 == 0 || y0 == 0 || x1 == 95 || y1 == 95) {
            // An edge joined to central ink may be a clipped letter. Removing
            // that component could manufacture another letter from the rest.
            if interior >= 8 {
                return None;
            }
            continue;
        }
        let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
        let grid = (w >= 67 && h <= 14 && (y0 < 14 || y1 > 81))
            || (h >= 67 && w <= 14 && (x0 < 14 || x1 > 81));
        if grid {
            continue;
        }
        left = left.min(x0);
        top = top.min(y0);
        right = right.max(x1);
        bottom = bottom.max(y1);
    }
    if right < left || bottom < top {
        return None;
    }
    let mut levels = gray.as_raw().clone();
    levels.sort_unstable();
    let paper = levels[levels.len() * 3 / 4];
    let glyph =
        image::imageops::crop_imm(gray, left, top, right - left + 1, bottom - top + 1).to_image();
    // Training spans roughly 59–91 px glyph heights. Keep this common view
    // inside that range, with room for antialiasing around thin serif strokes.
    // The transport patch uses square sampling coordinates. Restore the measured
    // 2:1 body / 10x7 grid metric before fitting the glyph into the model canvas.
    // Keeping all 96 vertical samples also preserves a Q tail near the grid.
    let physical_width = glyph.width() as f64 * 1.4;
    let scale = (76. / glyph.height() as f64).min(80. / physical_width);
    let glyph = image::imageops::resize(
        &glyph,
        (physical_width * scale).round() as u32,
        (glyph.height() as f64 * scale).round() as u32,
        FilterType::Triangle,
    );
    let mut canvas = GrayImage::from_pixel(96, 96, Luma([paper]));
    image::imageops::replace(
        &mut canvas,
        &glyph,
        ((96 - glyph.width()) / 2) as i64,
        ((96 - glyph.height()) / 2) as i64,
    );
    Some(canvas)
}

/// Pixel-center convention and grayscale coefficients match PIL/TorchVision.
fn rectified_rgb(image: &RgbImage, quad: [[f64; 2]; 4]) -> Result<RgbImage, MatrixOcrError> {
    let corners = quad.map(|[x, y]| geometry::Point {
        x: x * image.width() as f64,
        y: y * image.height() as f64,
    });
    let destination = [(0f32, 0f32), (95., 0.), (95., 95.), (0., 95.)];
    let source = destination.map(|(x, y)| {
        let p = geometry::project(
            &corners,
            (x as f64 + 0.5) / SIZE as f64,
            (y as f64 + 0.5) / SIZE as f64,
        );
        ((p.x - 0.5) as f32, (p.y - 0.5) as f32)
    });
    let projection =
        Projection::from_control_points(source, destination).ok_or(MatrixOcrError::GridNotFound)?;
    let mut crop = RgbImage::new(SIZE as u32, SIZE as u32);
    warp_into(
        image,
        projection,
        Interpolation::Bilinear,
        Border::Constant(Rgb([255; 3])),
        &mut crop,
    );
    Ok(crop)
}

pub(super) fn project_glyph(crop: &RgbImage, mode: crate::MatrixGlyphProjection) -> GrayImage {
    use crate::MatrixGlyphProjection;
    // Custom paper-color distance is template preprocessing; warp and image
    // operations remain in image/imageproc. Estimate paper away from grid ink.
    let mut channels: [Vec<u8>; 3] = std::array::from_fn(|_| Vec::new());
    if !matches!(mode, MatrixGlyphProjection::Luma) {
        for y in 9..crop.height().saturating_sub(9) {
            for x in 9..crop.width().saturating_sub(9) {
                for (c, values) in channels.iter_mut().enumerate() {
                    values.push(crop.get_pixel(x, y)[c]);
                }
            }
        }
    }
    let paper: [f64; 3] = channels.map(|mut values| {
        values.sort_unstable();
        values.get(values.len() * 3 / 4).copied().unwrap_or(255) as f64
    });
    let norm = paper.iter().map(|v| v * v).sum::<f64>().sqrt().max(1.);
    let red_paper = (paper[0] - (paper[1] + paper[2]) / 2.).max(4.);
    GrayImage::from_fn(crop.width(), crop.height(), |x, y| {
        let p = crop.get_pixel(x, y);
        let value = match mode {
            // Match training's PIL/TorchVision Rec.601, not image's Rec.709.
            MatrixGlyphProjection::Luma => {
                ((19595 * p[0] as u32 + 38470 * p[1] as u32 + 7471 * p[2] as u32 + 32768) >> 16)
                    as f64
            }
            MatrixGlyphProjection::RedChroma => {
                255. * (p[0] as f64 - (p[1] as f64 + p[2] as f64) / 2.) / red_paper
            }
            MatrixGlyphProjection::PaperDistance => {
                let distance = (0..3)
                    .map(|c| (paper[c] - p[c] as f64).max(0.).powi(2))
                    .sum::<f64>()
                    .sqrt();
                255. * (1. - distance / norm)
            }
        };
        image::Luma([value.round().clamp(0., 255.) as u8])
    })
}
/// Luma cell crop with the same pixel-centre convention as `rectified_rgb`.
fn rectified_luma(image: &GrayImage, quad: [[f64; 2]; 4]) -> Result<GrayImage, MatrixOcrError> {
    let corners = quad.map(|[x, y]| geometry::Point {
        x: x * image.width() as f64,
        y: y * image.height() as f64,
    });
    let destination = [(0f32, 0f32), (95., 0.), (95., 95.), (0., 95.)];
    let source = destination.map(|(x, y)| {
        let p = geometry::project(
            &corners,
            (x as f64 + 0.5) / SIZE as f64,
            (y as f64 + 0.5) / SIZE as f64,
        );
        ((p.x - 0.5) as f32, (p.y - 0.5) as f32)
    });
    let projection =
        Projection::from_control_points(source, destination).ok_or(MatrixOcrError::GridNotFound)?;
    let mut crop = GrayImage::new(SIZE as u32, SIZE as u32);
    warp_into(
        image,
        projection,
        Interpolation::Bilinear,
        Border::Constant(image::Luma([255])),
        &mut crop,
    );
    Ok(crop)
}

/// Where the 70 cells are cut from: an RGB photo (with an optional model
/// input projection for studies) or a camera keyframe's luma plane.
enum CellSource<'a> {
    Rgb(&'a RgbImage, crate::MatrixGlyphProjection),
    Luma(&'a GrayImage),
}

pub fn capture_matrix_frame(bytes: &[u8]) -> Result<MatrixCaptureFrame, MatrixOcrError> {
    capture_matrix_frame_configured(bytes, crate::MatrixCaptureConfig::default())
}

/// Locates the grid in one still image (no tracking) and cuts its cells.
pub fn capture_matrix_frame_configured(
    bytes: &[u8],
    config: crate::MatrixCaptureConfig,
) -> Result<MatrixCaptureFrame, MatrixOcrError> {
    let start = std::time::Instant::now();
    let image = decode_image(bytes)?;
    let decode_ms = start.elapsed().as_secs_f64() * 1000.;
    let stage = std::time::Instant::now();
    let quads = super::locate(&image).ok_or(MatrixOcrError::GridNotFound)?;
    let geometry_ms = stage.elapsed().as_secs_f64() * 1000.;
    capture_quads(
        CellSource::Rgb(&image, config.glyph_projection),
        quads,
        decode_ms,
        geometry_ms,
        start,
    )
}

/// Edge acuity of a cell interior: contrast-stretched (5th-95th percentile)
/// central-difference gradient magnitude, max-pooled over 4×4 blocks, then
/// the median block. Pooling keeps the strokes' edges and ignores flat paper;
/// the median needs edges across the glyph, not one sharp speck. A blurred
/// edge spreads the same intensity step over more pixels, so the value falls
/// with motion and defocus but not with exposure. Compared on recorded cards
/// against the Laplacian energy (`sharpness`), Tenengrad and a structure
/// tensor, this ranked keyframes by readable cells best (Spearman 0.91).
pub(crate) fn edge_acuity(image: &GrayImage) -> f64 {
    let (w, h) = (image.width() as usize, image.height() as usize);
    if w < 10 || h < 10 {
        return 0.;
    }
    let pixels: Vec<f64> = image.pixels().map(|p| p[0] as f64).collect();
    let mut sorted = pixels.clone();
    sorted.sort_by(f64::total_cmp);
    let at = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
    // A cell without an ink step has no edge to measure; stretching its
    // noise would read as a sharp edge.
    let range = at(0.95) - at(0.05);
    if range < 16. {
        return 0.;
    }
    let (gw, gh) = (w - 2, h - 2);
    let (bw, bh) = (gw / 4, gh / 4);
    let mut blocks = [0f64; 16];
    for y in 0..bh * 4 {
        for x in 0..bw * 4 {
            let (px, py) = (x + 1, y + 1);
            let gx = (pixels[py * w + px + 1] - pixels[py * w + px - 1]) / 2.;
            let gy = (pixels[(py + 1) * w + px] - pixels[(py - 1) * w + px]) / 2.;
            let block = &mut blocks[(y / bh) * 4 + x / bw];
            *block = block.max(gx.hypot(gy) / range);
        }
    }
    blocks.sort_by(f64::total_cmp);
    (blocks[7] + blocks[8]) / 2.
}

/// Cuts and measures the 70 cells of a tracked keyframe from its
/// full-resolution luma plane; `body` is the grid the tracker fitted on
/// that same frame (normalized, A1 top-left, clockwise).
pub fn capture_matrix_keyframe(
    luma: &[u8],
    width: u32,
    height: u32,
    body: [[f64; 2]; 4],
) -> Result<MatrixCaptureFrame, MatrixOcrError> {
    let start = std::time::Instant::now();
    if body
        .iter()
        .flatten()
        .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
    {
        return Err(MatrixOcrError::InvalidImage(
            "Invalid keyframe grid.".into(),
        ));
    }
    let image = GrayImage::from_raw(width, height, luma.to_vec())
        .filter(|_| width >= 32 && height >= 32 && (width as u64 * height as u64) <= 40_000_000)
        .ok_or_else(|| MatrixOcrError::InvalidImage("Invalid keyframe plane.".into()))?;
    let quads = super::grid::cells(body, 1., 1.).ok_or(MatrixOcrError::GridNotFound)?;
    capture_quads(CellSource::Luma(&image), quads, 0., 0., start)
}

/// Dataset-only recovery using manually verified body corners. This does not
/// change camera detection or turn a recovered crop into a readable label.
#[cfg(feature = "training-tools")]
pub fn capture_matrix_training_frame(
    bytes: &[u8],
    body: [[f64; 2]; 4],
) -> Result<MatrixCaptureFrame, MatrixOcrError> {
    if body
        .iter()
        .flatten()
        .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
        || (0..4).any(|i| {
            let [a, b, c] = [body[i], body[(i + 1) % 4], body[(i + 2) % 4]];
            (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]) <= 1e-8
        })
    {
        return Err(MatrixOcrError::InvalidImage(
            "Invalid clockwise body corners.".into(),
        ));
    }
    let start = std::time::Instant::now();
    let image = decode_image(bytes)?;
    let decode_ms = start.elapsed().as_secs_f64() * 1000.;
    let corners = body.map(|[x, y]| geometry::Point { x, y });
    let quads = geometry::body_cells(corners)
        .into_iter()
        .map(|quad| quad.map(|p| [p.x, p.y]))
        .collect();
    capture_quads(
        CellSource::Rgb(&image, crate::MatrixGlyphProjection::Luma),
        quads,
        decode_ms,
        0.,
        start,
    )
}

fn capture_quads(
    source: CellSource<'_>,
    quads: Vec<[[f64; 2]; 4]>,
    decode_ms: f64,
    geometry_ms: f64,
    start: std::time::Instant,
) -> Result<MatrixCaptureFrame, MatrixOcrError> {
    let stage = std::time::Instant::now();
    let (width, height) = match source {
        CellSource::Rgb(image, _) => (image.width() as f64, image.height() as f64),
        CellSource::Luma(image) => (image.width() as f64, image.height() as f64),
    };
    let patches = quads
        .iter()
        .map(|&quad| match source {
            CellSource::Rgb(image, projection) => {
                let rgb = rectified_rgb(image, quad)?;
                let gray = project_glyph(&rgb, crate::MatrixGlyphProjection::Luma);
                let light = illumination::analyze(&gray);
                // Physical admission stays in luminance for a controlled ablation.
                let projected = (!matches!(projection, crate::MatrixGlyphProjection::Luma))
                    .then(|| project_glyph(&rgb, projection));
                Ok((gray, light, projected))
            }
            CellSource::Luma(image) => {
                let gray = rectified_luma(image, quad)?;
                let light = illumination::analyze(&gray);
                Ok((gray, light, None))
            }
        })
        .collect::<Result<Vec<_>, MatrixOcrError>>()?;
    let mut paper: Vec<_> = patches.iter().map(|(_, light, _)| light.paper).collect();
    paper.sort_by(f64::total_cmp);
    // The upper quartile preserves well-lit cells on a partly shadowed card,
    // while isolated saturated rows/columns stand out against normal paper.
    let reference = paper.get(paper.len() * 3 / 4).copied().unwrap_or(255.);
    let glyphs = patches
        .into_iter()
        .zip(&quads)
        .map(|((gray, light, projected), &quad)| {
            // Ignore grid edges when measuring texture, but retain the entire
            // cell for recognition: a fixed inset can turn a complete L into I.
            let interior =
                image::imageops::crop_imm(light.corrected.as_ref().unwrap_or(&gray), 9, 9, 78, 78)
                    .to_image();
            let acuity = edge_acuity(&image::imageops::resize(
                &interior,
                48,
                48,
                FilterType::Triangle,
            ));
            let measured = image::imageops::resize(&interior, 32, 32, FilterType::Triangle);
            let mean = measured.pixels().map(|p| p[0] as f64).sum::<f64>() / 1024.;
            let contrast = (measured
                .pixels()
                .map(|p| (p[0] as f64 - mean).powi(2))
                .sum::<f64>()
                / 1024.)
                .sqrt();
            let deviation = |points: Vec<f64>| {
                let mean = points.iter().sum::<f64>() / points.len() as f64;
                (points.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / points.len() as f64)
                    .sqrt()
            };
            let interior = deviation(
                (4..28)
                    .flat_map(|y| (4..28).map(move |x| (x, y)))
                    .map(|(x, y)| measured.get_pixel(x, y)[0] as f64)
                    .collect(),
            );
            let border = deviation(
                (0..32)
                    .flat_map(|y| (0..32).map(move |x| (x, y)))
                    .filter(|&(x, y)| !(4..28).contains(&x) || !(4..28).contains(&y))
                    .map(|(x, y)| measured.get_pixel(x, y)[0] as f64)
                    .collect(),
            );
            let centering = interior / (interior + border).max(1.);
            let laplacian = imageproc::filter::laplacian_filter(&measured);
            let sharpness = (1..31)
                .flat_map(|y| (1..31).map(move |x| (x, y)))
                .map(|(x, y)| (laplacian.get_pixel(x, y)[0] as f64).powi(2))
                .sum::<f64>()
                / 900.;
            let distance =
                |a: [f64; 2], b: [f64; 2]| ((a[0] - b[0]) * width).hypot((a[1] - b[1]) * height);
            let source_size = (0..4)
                .map(|i| distance(quad[i], quad[(i + 1) % 4]))
                .fold(f64::INFINITY, f64::min);
            Ok(MatrixCaptureGlyph {
                gray: projected.unwrap_or(gray).into_raw(),
                contrast,
                sharpness,
                source_size,
                centering,
                glare: light.glare_against(reference),
                acuity,
            })
        })
        .collect::<Result<Vec<_>, MatrixOcrError>>()?;
    Ok(MatrixCaptureFrame {
        quads,
        glyphs,
        timing: crate::MatrixFrameTiming {
            decode_ms,
            geometry_ms,
            preprocess_ms: stage.elapsed().as_secs_f64() * 1000.,
            total_ms: start.elapsed().as_secs_f64() * 1000.,
            ..Default::default()
        },
    })
}

#[derive(Clone, Default)]
pub struct MatrixAlphabetService(Arc<Mutex<Option<Session>>>);

/// The exact model views for one retained cell. Training tools use this same
/// boundary; a rejected shape must not become an invented training label.
pub struct MatrixAlphabetInput {
    pub original: Vec<u8>,
    pub corrected: Option<Vec<u8>>,
}

pub fn prepare_matrix_alphabet_input(
    pixels: &[u8],
) -> Result<Option<MatrixAlphabetInput>, MatrixOcrError> {
    if pixels.len() != SIZE * SIZE {
        return Err(MatrixOcrError::InvalidImage(
            "Invalid grayscale candidate.".into(),
        ));
    }
    let gray = GrayImage::from_raw(SIZE as u32, SIZE as u32, pixels.to_vec())
        .ok_or_else(|| MatrixOcrError::InvalidImage("Invalid grayscale candidate.".into()))?;
    let Some(gray) = normalize_glyph(&gray) else {
        return Ok(None);
    };
    let corrected = illumination::analyze(&gray)
        .corrected
        .map(GrayImage::into_raw);
    Ok(Some(MatrixAlphabetInput {
        original: gray.into_raw(),
        corrected,
    }))
}

impl MatrixAlphabetService {
    pub fn recognize(&self, images: &[Vec<u8>]) -> Result<Vec<Recognition>, MatrixOcrError> {
        if images.len() > 10 || images.iter().any(|image| image.len() != SIZE * SIZE) {
            return Err(MatrixOcrError::InvalidImage(
                "Invalid grayscale candidate batch.".into(),
            ));
        }
        let normalized = images
            .iter()
            .map(|pixels| prepare_matrix_alphabet_input(pixels))
            .collect::<Result<Vec<_>, MatrixOcrError>>()?;
        let images = normalized
            .iter()
            .map(|glyph| {
                glyph
                    .as_ref()
                    .map_or_else(|| vec![255; SIZE * SIZE], |glyph| glyph.original.clone())
            })
            .collect::<Vec<_>>();
        let images = &images;
        let mut raw = self.recognize_raw(images)?;
        let corrections = normalized
            .iter()
            .enumerate()
            .filter_map(|(i, glyph)| glyph.as_ref()?.corrected.clone().map(|pixels| (i, pixels)))
            .collect::<Vec<_>>();
        let (positions, images): (Vec<_>, Vec<_>) = corrections.into_iter().unzip();
        for (i, corrected) in positions.into_iter().zip(self.recognize_raw(&images)?) {
            // Two renderings of one observation remain ONE vote. A confident
            // disagreement is review evidence, never a second independent vote.
            if raw[i].confidence >= 0.8
                && corrected.confidence >= 0.8
                && raw[i].text != corrected.text
            {
                raw[i].confidence = 0.79;
            } else if corrected.confidence > raw[i].confidence {
                raw[i] = corrected;
            }
        }
        for (reading, glyph) in raw.iter_mut().zip(normalized) {
            if glyph.is_none() {
                *reading = Recognition::default();
            }
        }
        Ok(raw)
    }
    fn recognize_raw(&self, images: &[Vec<u8>]) -> Result<Vec<Recognition>, MatrixOcrError> {
        if images.is_empty() {
            return Ok(vec![]);
        }
        if images.len() > 10 || images.iter().any(|image| image.len() != SIZE * SIZE) {
            return Err(MatrixOcrError::InvalidImage(
                "Invalid grayscale candidate batch.".into(),
            ));
        }
        let mut resident = match self.0.try_lock() {
            Ok(resident) => resident,
            Err(TryLockError::WouldBlock) => return Err(MatrixOcrError::Busy),
            Err(TryLockError::Poisoned(poisoned)) => {
                *poisoned.into_inner() = None;
                self.0.clear_poison();
                return Err(MatrixOcrError::Model(
                    "Alphabet scanner recovered after an interrupted scan. Try again.".into(),
                ));
            }
        };
        if resident.is_none() {
            *resident = Some(
                Session::builder()
                    .map_err(model_error)?
                    .with_intra_threads(2)
                    .map_err(model_error)?
                    .with_inter_threads(1)
                    .map_err(model_error)?
                    .commit_from_memory(MODEL)
                    .map_err(model_error)?,
            );
        }
        let session = resident
            .as_mut()
            .ok_or_else(|| MatrixOcrError::Model("Alphabet model did not initialize.".into()))?;
        let input = Tensor::from_array((
            [images.len(), 1, SIZE, SIZE],
            images
                .iter()
                .flatten()
                .map(|p| *p as f32 / 127.5 - 1.)
                .collect::<Vec<_>>(),
        ))
        .map_err(model_error)?;
        let output = session.run(ort::inputs![input]).map_err(model_error)?;
        let Some((_, output)) = output.into_iter().next() else {
            return Err(MatrixOcrError::Model("Missing alphabet output.".into()));
        };
        let (shape, data) = output.try_extract_tensor::<f32>().map_err(model_error)?;
        if shape.as_ref() != [images.len() as i64, 27]
            || data.len() != images.len() * 27
            || data.iter().any(|v| !v.is_finite())
        {
            return Err(MatrixOcrError::Model("Invalid alphabet output.".into()));
        }
        data.as_chunks::<27>()
            .0
            .iter()
            .map(|row| {
                let (winner, &max) = row
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.total_cmp(b.1))
                    .ok_or_else(|| MatrixOcrError::Model("Empty alphabet output.".into()))?;
                let sum = row.iter().map(|v| (*v - max).exp()).sum::<f32>();
                Ok(Recognition {
                    text: if winner == 26 {
                        String::new()
                    } else {
                        ((b'A' + winner as u8) as char).to_string()
                    },
                    confidence: 1. / sum as f64,
                })
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "../tests/engine__alphabet.rs"]
mod tests;
