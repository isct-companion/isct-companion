//! Where pages sit in a continuous column and which tiles a viewport needs.
//!
//! Coordinates: page sizes are PDF points; everything the view places is CSS
//! pixels from the top-left of the scrolled content; tiles are device pixels
//! at a level. A level `L` draws a page at `2^(L/2)` device pixels per point,
//! so nearby zooms share tiles and the view scales them by at most √2.
use serde::{Deserialize, Serialize};

/// Device pixels on a tile's side.
pub const TILE: u32 = 512;
/// CSS pixels between pages and around the column.
pub const GAP: f64 = 12.0;
/// The preview of each page is about this many device pixels wide.
const PREVIEW_WIDTH: f64 = 640.0;
/// Levels a tile may use: 1/16 to 16 device pixels per point.
const LEVELS: std::ops::RangeInclusive<i8> = -8..=8;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Viewport {
    /// The scroll area's inner size, CSS pixels.
    pub width: f64,
    pub height: f64,
    pub scroll_top: f64,
    /// 1 fits the widest page to the width.
    pub zoom: f64,
    pub device_pixel_ratio: f64,
}

/// One tile of one page at one level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TileKey {
    pub page: u32,
    pub level: i8,
    pub col: u32,
    pub row: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlacedTile {
    pub key: TileKey,
    /// CSS pixels within its page.
    pub rect: Rect,
    /// A preview tile, drawn under the sharp ones.
    pub preview: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlacedPage {
    pub index: u32,
    /// CSS pixels within the content.
    pub rect: Rect,
    pub tiles: Vec<PlacedTile>,
}

/// What the view draws for one viewport. `wanted` lists every tile in the
/// order to render them; the engine drops queued tiles not in it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub content_width: f64,
    pub content_height: f64,
    /// CSS pixels per point.
    pub scale: f64,
    /// The page most of the viewport shows, 0-based.
    pub current_page: u32,
    pub pages: Vec<PlacedPage>,
    pub wanted: Vec<TileKey>,
}

#[derive(Clone, Debug)]
pub struct Layout {
    pages: Vec<PageSize>,
    widest: f64,
}

impl Layout {
    pub fn new(pages: Vec<PageSize>) -> Self {
        let widest = pages
            .iter()
            .map(|page| f64::from(page.width))
            .fold(1.0, f64::max);
        Self { pages, widest }
    }

    pub fn pages(&self) -> &[PageSize] {
        &self.pages
    }

    /// CSS pixels per point for a viewport.
    pub fn scale(&self, viewport: &Viewport) -> f64 {
        let fit = ((viewport.width - 2.0 * GAP).max(1.0)) / self.widest;
        fit * viewport.zoom.clamp(0.25, 8.0)
    }

    /// Top of each page and the content height, CSS pixels.
    fn tops(&self, scale: f64) -> (Vec<f64>, f64) {
        let mut top = GAP;
        let tops = self
            .pages
            .iter()
            .map(|page| {
                let here = top;
                top += f64::from(page.height) * scale + GAP;
                here
            })
            .collect();
        (tops, top)
    }

    /// The scroll position that puts the top of `page` at the top of the
    /// viewport, for page jumps and for keeping the place across a zoom.
    pub fn page_top(&self, viewport: &Viewport, page: u32) -> f64 {
        let (tops, _) = self.tops(self.scale(viewport));
        tops.get(page as usize).copied().unwrap_or(0.0) - GAP
    }

    pub fn plan(&self, viewport: &Viewport) -> Plan {
        let scale = self.scale(viewport);
        let (tops, content_height) = self.tops(scale);
        let content_width = (self.widest * scale + 2.0 * GAP).max(viewport.width);
        let dpr = viewport.device_pixel_ratio.clamp(1.0, 4.0);
        let sharp = level_for(scale * dpr);
        let view_top = viewport.scroll_top;
        let view_bottom = viewport.scroll_top + viewport.height;
        // Previews a viewport ahead and behind, sharp tiles half a viewport.
        let preview_band = (view_top - viewport.height, view_bottom + viewport.height);
        let sharp_band = (
            view_top - viewport.height / 2.0,
            view_bottom + viewport.height / 2.0,
        );
        let middle = (view_top + view_bottom) / 2.0;

        let mut current = (0u32, f64::MIN);
        // A hidden reader has no size: it draws and wants nothing, so its
        // tiles are released and its queued renders dropped.
        if viewport.width <= 0.0 || viewport.height <= 0.0 {
            return Plan {
                content_width,
                content_height,
                scale,
                current_page: 0,
                pages: Vec::new(),
                wanted: Vec::new(),
            };
        }
        let mut pages = Vec::new();
        let mut previews = Vec::new();
        let mut sharps = Vec::new();
        for (index, (page, &top)) in self.pages.iter().zip(&tops).enumerate() {
            let width = f64::from(page.width) * scale;
            let height = f64::from(page.height) * scale;
            let bottom = top + height;
            let shown = overlap(top, bottom, view_top, view_bottom);
            if shown > current.1 {
                current = (index as u32, shown);
            }
            if bottom < preview_band.0 || top > preview_band.1 {
                continue;
            }
            let left = (content_width - width) / 2.0;
            let index = index as u32;
            let preview = preview_level(page).min(sharp);
            let mut tiles = grid(page, index, preview, scale, None, true);
            let distance = (top + height / 2.0 - middle).abs();
            previews.extend(tiles.iter().map(|tile| (distance, tile.key)));
            if sharp > preview && bottom >= sharp_band.0 && top <= sharp_band.1 {
                // Only the part of the page inside the band, in page pixels.
                let from = (sharp_band.0 - top).max(0.0);
                let to = (sharp_band.1 - top).min(height);
                let sharp_tiles = grid(page, index, sharp, scale, Some((from, to)), false);
                for tile in &sharp_tiles {
                    let centre = top + tile.rect.top + tile.rect.height / 2.0;
                    sharps.push(((centre - middle).abs(), tile.key));
                }
                tiles.extend(sharp_tiles);
            }
            pages.push(PlacedPage {
                index,
                rect: Rect {
                    left,
                    top,
                    width,
                    height,
                },
                tiles,
            });
        }
        // Every preview first, nearest the middle first, so a fast scroll
        // always lands on something; then the sharp tiles the same way.
        previews.sort_by(|a, b| a.0.total_cmp(&b.0));
        sharps.sort_by(|a, b| a.0.total_cmp(&b.0));
        let wanted = previews
            .into_iter()
            .chain(sharps)
            .map(|(_, key)| key)
            .collect();
        Plan {
            content_width,
            content_height,
            scale,
            current_page: current.0,
            pages,
            wanted,
        }
    }
}

fn overlap(a0: f64, a1: f64, b0: f64, b1: f64) -> f64 {
    (a1.min(b1) - a0.max(b0)).max(0.0)
}

/// The smallest level whose resolution is at least `device_per_point`.
pub fn level_for(device_per_point: f64) -> i8 {
    let level = (2.0 * device_per_point.max(1e-3).log2()).ceil();
    (level as i8).clamp(*LEVELS.start(), *LEVELS.end())
}

/// The level of a page's preview tiles; the engine keeps these on disk.
pub fn preview_level(page: &PageSize) -> i8 {
    level_for(PREVIEW_WIDTH / f64::from(page.width).max(1.0))
}

/// Device pixels per point at a level.
pub fn level_scale(level: i8) -> f64 {
    2f64.powf(f64::from(level) / 2.0)
}

/// A page's size in device pixels at a level.
pub fn page_pixels(page: &PageSize, level: i8) -> (u32, u32) {
    let scale = level_scale(level);
    (
        ((f64::from(page.width) * scale).ceil() as u32).max(1),
        ((f64::from(page.height) * scale).ceil() as u32).max(1),
    )
}

/// The tiles of `page` at `level`, limited to the rows that cross `band`
/// (CSS pixels from the page top) when given.
fn grid(
    page: &PageSize,
    index: u32,
    level: i8,
    scale: f64,
    band: Option<(f64, f64)>,
    preview: bool,
) -> Vec<PlacedTile> {
    let (width, height) = page_pixels(page, level);
    // CSS pixels per device pixel of this level.
    let css = scale / level_scale(level);
    let cols = width.div_ceil(TILE);
    let rows = height.div_ceil(TILE);
    let row_range = match band {
        Some((from, to)) => {
            let first = (from / css / f64::from(TILE)).floor().max(0.0) as u32;
            let last = ((to / css / f64::from(TILE)).ceil() as u32).min(rows);
            first..last
        }
        None => 0..rows,
    };
    let mut tiles = Vec::new();
    for row in row_range {
        for col in 0..cols {
            let x = col * TILE;
            let y = row * TILE;
            tiles.push(PlacedTile {
                key: TileKey {
                    page: index,
                    level,
                    col,
                    row,
                },
                rect: Rect {
                    left: f64::from(x) * css,
                    top: f64::from(y) * css,
                    width: f64::from(TILE.min(width - x)) * css,
                    height: f64::from(TILE.min(height - y)) * css,
                },
                preview,
            });
        }
    }
    tiles
}

#[cfg(test)]
#[path = "tests/layout.rs"]
mod tests;
