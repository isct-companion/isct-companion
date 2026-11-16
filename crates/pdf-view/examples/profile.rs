//! Times the engine on real PDFs: `cargo run -p pdf-view --release --example
//! profile -- <file.pdf>...`. Prints, per file, the first tile of page 1 (the
//! page's image decode), the rest of that page's tiles, and the same page a
//! zoom step up, which should reuse the decoded page.
use pdf_view::layout::{Layout, Viewport};
use pdf_view::{Engine, LIBRARY_NAME};
use std::sync::mpsc::channel;
use std::time::Instant;

fn main() {
    let exe = std::env::current_exe().expect("example path");
    let library = exe.parent().expect("example folder").join(LIBRARY_NAME);
    let cache = std::env::temp_dir().join("pdf-view-profile");
    let _ = std::fs::remove_dir_all(&cache);
    let engine = Engine::start(&library, cache).expect("PDFium");
    for path in std::env::args().skip(1) {
        let (tx, rx) = channel();
        engine.open(path.clone().into(), Box::new(move |r| drop(tx.send(r))));
        let opened = rx.recv().expect("reply").expect("open");
        let layout = Layout::new(opened.pages.clone());
        let mut timings = Vec::new();
        for zoom in [1.0, 1.5] {
            let viewport = Viewport {
                width: 1100.0,
                height: 900.0,
                scroll_top: 0.0,
                zoom,
                device_pixel_ratio: 2.0,
            };
            let plan = layout.plan(&viewport);
            let keys: Vec<_> = plan
                .wanted
                .iter()
                .filter(|k| k.page == 0)
                .copied()
                .collect();
            engine.want(opened.id, keys.clone());
            let mut each = Vec::new();
            for key in keys {
                let (tx, rx) = channel();
                let started = Instant::now();
                engine.tile(opened.id, key, Box::new(move |r| drop(tx.send(r))));
                rx.recv().expect("reply").expect("tile");
                each.push(started.elapsed().as_millis());
            }
            timings.push(each);
        }
        println!(
            "{:>6.1} MB {:>3} pages | zoom 1: {:?} ms | zoom 1.5: {:?} ms | {}",
            std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) as f64 / 1e6,
            opened.pages.len(),
            timings[0],
            timings[1],
            path.rsplit('/')
                .next()
                .unwrap_or(&path)
                .chars()
                .take(8)
                .collect::<String>()
        );
        engine.close(opened.id);
    }
}
