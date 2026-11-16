use super::*;

fn a4() -> PageSize {
    PageSize {
        width: 595.0,
        height: 842.0,
    }
}

fn viewport(scroll_top: f64, zoom: f64) -> Viewport {
    Viewport {
        width: 1024.0,
        height: 800.0,
        scroll_top,
        zoom,
        device_pixel_ratio: 2.0,
    }
}

/// Levels step by √2 and always cover the resolution asked for.
#[test]
fn level_covers_the_requested_resolution() {
    for wanted in [0.3, 1.0, 1.41, 1.42, 2.0, 3.3, 7.9] {
        let level = level_for(wanted);
        assert!(level_scale(level) >= wanted - 1e-9, "{wanted}");
        assert!(level_scale(level - 1) < wanted, "{wanted}");
    }
    assert_eq!(level_for(1e9), 8);
}

/// A 200-page scan plans only the pages near the viewport; previews come
/// before sharp tiles, the page under the middle first; every sharp tile
/// crosses the band around the viewport; tiles cover their page exactly.
#[test]
fn plans_only_what_the_viewport_needs() {
    let layout = Layout::new(vec![a4(); 200]);
    let at = layout.page_top(&viewport(0.0, 1.0), 100);
    let plan = layout.plan(&viewport(at, 1.0));

    assert_eq!(plan.current_page, 100);
    let indexes: Vec<_> = plan.pages.iter().map(|page| page.index).collect();
    assert!(indexes.len() <= 4, "{indexes:?}");
    assert!(indexes.contains(&99) && indexes.contains(&101));

    let first_sharp = plan
        .wanted
        .iter()
        .position(|key| key.level > plan.wanted[0].level)
        .expect("sharp tiles");
    assert!(plan.wanted[..first_sharp]
        .iter()
        .all(|key| key.level == plan.wanted[0].level));
    assert_eq!(plan.wanted[0].page, 100);
    assert_eq!(plan.wanted[first_sharp].page, 100);

    let page = plan
        .pages
        .iter()
        .find(|page| page.index == 100)
        .expect("page");
    let previews: Vec<_> = page.tiles.iter().filter(|tile| tile.preview).collect();
    let covered: f64 = previews.iter().map(|t| t.rect.width * t.rect.height).sum();
    // Tiles round the page up to whole device pixels: under 1% more.
    let area = page.rect.width * page.rect.height;
    assert!(
        covered >= area && covered < area * 1.01,
        "{covered} vs {area}"
    );
    for tile in page.tiles.iter().filter(|tile| !tile.preview) {
        let top = page.rect.top + tile.rect.top;
        assert!(top + tile.rect.height >= at - 400.0 && top <= at + 800.0 + 400.0);
    }
}

/// Zooming in only raises the level of the sharp tiles; the preview level
/// stays, so its tiles are reused across zooms.
#[test]
fn zoom_changes_sharp_level_not_preview() {
    let layout = Layout::new(vec![a4(); 3]);
    let level = |plan: &Plan, preview: bool| {
        plan.pages[0]
            .tiles
            .iter()
            .find(|tile| tile.preview == preview)
            .map(|tile| tile.key.level)
    };
    let near = layout.plan(&viewport(0.0, 1.0));
    let far = layout.plan(&viewport(0.0, 3.0));
    assert_eq!(level(&near, true), level(&far, true));
    assert!(level(&far, false) > level(&near, false));
    assert!(far.content_height > near.content_height * 2.9);
}

/// Each page keeps its own proportions in a mixed portrait and landscape
/// deck, and a hidden (zero-size) viewport plans no pages and wants no tiles.
#[test]
fn mixed_pages_keep_their_shape_and_hidden_views_want_nothing() {
    let landscape = PageSize {
        width: 842.0,
        height: 595.0,
    };
    let layout = Layout::new(vec![a4(), landscape, a4()]);
    let mut tall = viewport(0.0, 1.0);
    tall.height = 3000.0;
    let plan = layout.plan(&tall);
    let ratios: Vec<_> = plan
        .pages
        .iter()
        .map(|page| format!("{:.3}", page.rect.width / page.rect.height))
        .collect();
    assert_eq!(ratios, ["0.707", "1.415", "0.707"]);

    let mut hidden = viewport(0.0, 1.0);
    hidden.height = 0.0;
    let plan = layout.plan(&hidden);
    assert!(plan.pages.is_empty() && plan.wanted.is_empty());
}
