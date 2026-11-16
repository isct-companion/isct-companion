//! One thread owns PDFium and renders tiles in the order the view wants
//! them. PDFium is not thread-safe, so every call goes through this thread.
//!
//! Scanned exams are pages of 7–35 megapixel images. PDFium keeps a loaded
//! page's decoded images, so the thread keeps the last few pages loaded:
//! the first tile of a page pays the decode, the rest of its tiles and any
//! zoom of it reuse it. Encoded tiles stay in memory, and preview tiles also
//! on disk, so reopening a document shows it at once.
use crate::layout::{self, PageSize, Rect, TileKey, TILE};
use crate::PdfError;
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

pub type DocId = u64;
/// Answers a request on the render thread; keep it short.
pub type Reply<T> = Box<dyn FnOnce(Result<T, PdfError>) + Send>;

/// Loaded pages kept per document, each with its decoded images.
const LOADED_PAGES: usize = 4;
/// Encoded tiles kept in memory, all documents together.
const MEMORY_BUDGET: usize = 192 << 20;
/// Preview tiles kept on disk, all documents together.
const DISK_BUDGET: u64 = 256 << 20;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Opened {
    pub id: DocId,
    pub pages: Vec<PageSize>,
}

/// A run of text on one line, for the selection layer. Points from the
/// page's top-left corner.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextRun {
    pub text: String,
    pub rect: Rect,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchHit {
    pub page: u32,
    /// Points from the page's top-left corner; a hit across lines has several.
    pub rects: Vec<Rect>,
}

enum Command {
    Open {
        path: PathBuf,
        reply: Reply<Opened>,
    },
    Close(DocId),
    Tile {
        doc: DocId,
        key: TileKey,
        reply: Reply<Arc<Vec<u8>>>,
    },
    Want {
        doc: DocId,
        keys: Vec<TileKey>,
    },
    Text {
        doc: DocId,
        page: u32,
        reply: Reply<Vec<TextRun>>,
    },
    Search {
        doc: DocId,
        query: String,
        reply: Reply<Vec<SearchHit>>,
    },
}

/// The handle every caller shares; dropping the last one stops the thread.
#[derive(Clone)]
pub struct Engine {
    commands: Sender<Command>,
}

impl Engine {
    /// Loads PDFium from `library` and starts the render thread. `cache` is
    /// the folder for preview tiles.
    pub fn start(library: &Path, cache: PathBuf) -> Result<Self, PdfError> {
        let (commands, receiver) = channel();
        let (ready, started) = channel();
        let library = library.to_owned();
        std::thread::Builder::new()
            .name("pdf-view".into())
            .spawn(move || match Pdfium::bind_to_library(&library) {
                Ok(bindings) => {
                    // The thread lives as long as the app; PDFium's own
                    // lifetime ties every document to this.
                    let pdfium: &'static Pdfium = Box::leak(Box::new(Pdfium::new(bindings)));
                    let _ = ready.send(Ok(()));
                    Worker::new(pdfium, cache).run(receiver);
                }
                Err(error) => {
                    let _ = ready.send(Err(PdfError::Library(error.to_string())));
                }
            })
            .map_err(|e| PdfError::Library(e.to_string()))?;
        started
            .recv()
            .map_err(|_| PdfError::Library("render thread stopped".into()))??;
        Ok(Self { commands })
    }

    fn send(&self, command: Command) -> Result<(), PdfError> {
        self.commands.send(command).map_err(|_| PdfError::Stopped)
    }

    pub fn open(&self, path: PathBuf, reply: Reply<Opened>) {
        if let Err(Command::Open { reply, .. }) = self
            .commands
            .send(Command::Open { path, reply })
            .map_err(|e| e.0)
        {
            reply(Err(PdfError::Stopped));
        }
    }

    pub fn close(&self, doc: DocId) {
        let _ = self.send(Command::Close(doc));
    }

    pub fn tile(&self, doc: DocId, key: TileKey, reply: Reply<Arc<Vec<u8>>>) {
        if let Err(Command::Tile { reply, .. }) = self
            .commands
            .send(Command::Tile { doc, key, reply })
            .map_err(|e| e.0)
        {
            reply(Err(PdfError::Stopped));
        }
    }

    /// The tiles the view needs now, in render order (`Plan::wanted`).
    /// Queued tiles of `doc` not in it are dropped.
    pub fn want(&self, doc: DocId, keys: Vec<TileKey>) {
        let _ = self.send(Command::Want { doc, keys });
    }

    pub fn text(&self, doc: DocId, page: u32, reply: Reply<Vec<TextRun>>) {
        if let Err(Command::Text { reply, .. }) = self
            .commands
            .send(Command::Text { doc, page, reply })
            .map_err(|e| e.0)
        {
            reply(Err(PdfError::Stopped));
        }
    }

    pub fn search(&self, doc: DocId, query: String, reply: Reply<Vec<SearchHit>>) {
        if let Err(Command::Search { reply, .. }) = self
            .commands
            .send(Command::Search { doc, query, reply })
            .map_err(|e| e.0)
        {
            reply(Err(PdfError::Stopped));
        }
    }
}

struct Document {
    // Pages first: they are closed before their document.
    loaded: VecDeque<(u32, PdfPage<'static>)>,
    document: PdfDocument<'static>,
    sizes: Vec<PageSize>,
    /// Names this file's preview tiles on disk.
    fingerprint: String,
    /// Render order of the tiles the view wants.
    wanted: HashMap<TileKey, usize>,
}

/// A tile across documents: the file's fingerprint and the key.
type TileName = (String, TileKey);

struct Pending {
    doc: DocId,
    key: TileKey,
    reply: Reply<Arc<Vec<u8>>>,
}

struct Worker {
    pdfium: &'static Pdfium,
    cache: PathBuf,
    next: DocId,
    documents: HashMap<DocId, Document>,
    pending: Vec<Pending>,
    memory: HashMap<TileName, (Arc<Vec<u8>>, u64)>,
    memory_bytes: usize,
    tick: u64,
}

impl Worker {
    fn new(pdfium: &'static Pdfium, cache: PathBuf) -> Self {
        prune(&cache, DISK_BUDGET);
        Self {
            pdfium,
            cache,
            next: 1,
            documents: HashMap::new(),
            pending: Vec::new(),
            memory: HashMap::new(),
            memory_bytes: 0,
            tick: 0,
        }
    }

    fn run(mut self, commands: Receiver<Command>) {
        loop {
            if self.pending.is_empty() {
                match commands.recv() {
                    Ok(command) => self.handle(command),
                    Err(_) => return,
                }
            }
            while let Ok(command) = commands.try_recv() {
                self.handle(command);
            }
            if let Some(job) = self.take_next() {
                let result = self.render(job.doc, job.key);
                (job.reply)(result);
            }
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Open { path, reply } => reply(self.open(&path)),
            Command::Close(doc) => {
                if let Some(mut document) = self.documents.remove(&doc) {
                    document.loaded.clear();
                }
                self.cancel(|job| job.doc == doc);
            }
            Command::Tile { doc, key, reply } => {
                let cached = self
                    .documents
                    .get(&doc)
                    .map(|document| (document.fingerprint.clone(), document.sizes.clone()));
                let Some((fingerprint, sizes)) = cached else {
                    return reply(Err(PdfError::Closed));
                };
                if let Some(bytes) = self.cached(&fingerprint, &sizes, key) {
                    reply(Ok(bytes));
                } else {
                    self.pending.push(Pending { doc, key, reply });
                }
            }
            Command::Want { doc, keys } => {
                if let Some(document) = self.documents.get_mut(&doc) {
                    document.wanted = keys.into_iter().enumerate().map(|(i, k)| (k, i)).collect();
                    let wanted = document.wanted.clone();
                    self.cancel(|job| job.doc == doc && !wanted.contains_key(&job.key));
                }
            }
            Command::Text { doc, page, reply } => reply(self.text(doc, page)),
            Command::Search { doc, query, reply } => reply(self.search(doc, &query)),
        }
    }

    fn cancel(&mut self, matches: impl Fn(&Pending) -> bool) {
        let (dropped, kept) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition::<Vec<_>, _>(|job| matches(job));
        self.pending = kept;
        for job in dropped {
            (job.reply)(Err(PdfError::Cancelled));
        }
    }

    /// The queued tile the view wants soonest; unranked tiles go last, in
    /// arrival order.
    fn take_next(&mut self) -> Option<Pending> {
        let rank = |job: &Pending| {
            self.documents
                .get(&job.doc)
                .and_then(|document| document.wanted.get(&job.key).copied())
                .unwrap_or(usize::MAX)
        };
        let best = (0..self.pending.len()).min_by_key(|&i| rank(&self.pending[i]))?;
        Some(self.pending.remove(best))
    }

    fn open(&mut self, path: &Path) -> Result<Opened, PdfError> {
        let document = self
            .pdfium
            .load_pdf_from_file(path, None)
            .map_err(|e| PdfError::Open(e.to_string()))?;
        let sizes = document
            .pages()
            .page_sizes()
            .map_err(|e| PdfError::Open(e.to_string()))?
            .into_iter()
            .map(|rect| PageSize {
                width: rect.width().value,
                height: rect.height().value,
            })
            .collect::<Vec<_>>();
        let id = self.next;
        self.next += 1;
        self.documents.insert(
            id,
            Document {
                loaded: VecDeque::new(),
                document,
                sizes: sizes.clone(),
                fingerprint: fingerprint(path),
                wanted: HashMap::new(),
            },
        );
        Ok(Opened { id, pages: sizes })
    }

    fn cached(
        &mut self,
        fingerprint: &str,
        sizes: &[PageSize],
        key: TileKey,
    ) -> Option<Arc<Vec<u8>>> {
        self.tick += 1;
        let entry = (fingerprint.to_owned(), key);
        if let Some((bytes, used)) = self.memory.get_mut(&entry) {
            *used = self.tick;
            return Some(bytes.clone());
        }
        let page = sizes.get(key.page as usize)?;
        if key.level > layout::preview_level(page) {
            return None;
        }
        let bytes = Arc::new(std::fs::read(self.disk_path(fingerprint, key)).ok()?);
        self.remember(entry, bytes.clone());
        Some(bytes)
    }

    fn remember(&mut self, entry: TileName, bytes: Arc<Vec<u8>>) {
        self.memory_bytes += bytes.len();
        if let Some((old, _)) = self.memory.insert(entry, (bytes, self.tick)) {
            self.memory_bytes -= old.len();
        }
        while self.memory_bytes > MEMORY_BUDGET {
            let Some(oldest) = self
                .memory
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(entry, _)| entry.clone())
            else {
                break;
            };
            if let Some((bytes, _)) = self.memory.remove(&oldest) {
                self.memory_bytes -= bytes.len();
            }
        }
    }

    fn disk_path(&self, fingerprint: &str, key: TileKey) -> PathBuf {
        self.cache.join(fingerprint).join(format!(
            "{}_{}_{}_{}.png",
            key.page, key.level, key.col, key.row
        ))
    }

    /// The loaded page, loading it (and closing the oldest) when needed.
    fn page(document: &mut Document, index: u32) -> Result<&PdfPage<'static>, PdfError> {
        if let Some(at) = document.loaded.iter().position(|(i, _)| *i == index) {
            let entry = document.loaded.remove(at).ok_or(PdfError::Render)?;
            document.loaded.push_back(entry);
        } else {
            let page = document
                .document
                .pages()
                .get(i32::try_from(index).map_err(|_| PdfError::Render)?)
                .map_err(|_| PdfError::Render)?;
            if document.loaded.len() >= LOADED_PAGES {
                document.loaded.pop_front();
            }
            document.loaded.push_back((index, page));
        }
        document
            .loaded
            .back()
            .map(|(_, page)| page)
            .ok_or(PdfError::Render)
    }

    fn render(&mut self, doc: DocId, key: TileKey) -> Result<Arc<Vec<u8>>, PdfError> {
        let document = self.documents.get_mut(&doc).ok_or(PdfError::Closed)?;
        let size = *document
            .sizes
            .get(key.page as usize)
            .ok_or(PdfError::Render)?;
        let fingerprint = document.fingerprint.clone();
        let (width, height) = layout::page_pixels(&size, key.level);
        let (x, y) = (key.col * TILE, key.row * TILE);
        if x >= width || y >= height {
            return Err(PdfError::Render);
        }
        let (tile_width, tile_height) = (TILE.min(width - x), TILE.min(height - y));
        let page = Self::page(document, key.page)?;
        let mut bitmap =
            PdfBitmap::empty(tile_width as i32, tile_height as i32, PdfBitmapFormat::BGRA)
                .map_err(|_| PdfError::Render)?;
        // The whole page at this level, shifted so the tile's corner is at
        // the bitmap's; PDFium draws only what lands inside the bitmap.
        let config = PdfRenderConfig::new()
            .set_target_size(width as i32, height as i32)
            .set_origin(-(x as i32), -(y as i32))
            .set_clear_color(PdfColor::WHITE)
            .set_reverse_byte_order(true)
            .render_annotations(true);
        page.render_into_bitmap_with_config(&mut bitmap, &config)
            .map_err(|_| PdfError::Render)?;
        let bytes = Arc::new(encode(&bitmap.as_raw_bytes(), tile_width, tile_height)?);
        if key.level <= layout::preview_level(&size) {
            let path = self.disk_path(&fingerprint, key);
            if let Some(folder) = path.parent() {
                let _ = std::fs::create_dir_all(folder);
            }
            let _ = std::fs::write(path, bytes.as_slice());
        }
        self.remember((fingerprint, key), bytes.clone());
        Ok(bytes)
    }

    fn text(&mut self, doc: DocId, index: u32) -> Result<Vec<TextRun>, PdfError> {
        let document = self.documents.get_mut(&doc).ok_or(PdfError::Closed)?;
        let height = document
            .sizes
            .get(index as usize)
            .ok_or(PdfError::Render)?
            .height;
        let page = Self::page(document, index)?;
        let text = page.text().map_err(|_| PdfError::Render)?;
        let runs = text
            .segments()
            .iter()
            .filter_map(|segment| {
                let content = segment.text();
                (!content.trim().is_empty()).then(|| TextRun {
                    text: content,
                    rect: top_left(segment.bounds(), height),
                })
            })
            .collect();
        Ok(runs)
    }

    fn search(&mut self, doc: DocId, query: &str) -> Result<Vec<SearchHit>, PdfError> {
        let document = self.documents.get_mut(&doc).ok_or(PdfError::Closed)?;
        let mut hits = Vec::new();
        if query.trim().is_empty() {
            return Ok(hits);
        }
        let options = PdfSearchOptions::new();
        for index in 0..document.sizes.len() as u32 {
            let height = document.sizes[index as usize].height;
            // Searching loads pages one by one; do not evict the view's.
            let page = document
                .document
                .pages()
                .get(i32::try_from(index).map_err(|_| PdfError::Render)?)
                .map_err(|_| PdfError::Render)?;
            let text = page.text().map_err(|_| PdfError::Render)?;
            let search = text.search(query, &options).map_err(|_| PdfError::Render)?;
            for found in search.iter(PdfSearchDirection::SearchForward) {
                let rects = found
                    .iter()
                    .map(|segment| top_left(segment.bounds(), height))
                    .collect();
                hits.push(SearchHit { page: index, rects });
            }
        }
        Ok(hits)
    }
}

/// A PDF rectangle (origin bottom-left) as points from the top-left.
fn top_left(rect: PdfRect, page_height: f32) -> Rect {
    Rect {
        left: f64::from(rect.left().value),
        top: f64::from(page_height - rect.top().value),
        width: f64::from(rect.width().value),
        height: f64::from(rect.height().value),
    }
}

/// RGBA pixels as a PNG; fast compression, since tiles are encoded on every
/// render and only travel within the app.
fn encode(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, PdfError> {
    let mut out = Vec::with_capacity(rgba.len() / 4);
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(|_| PdfError::Encode)?;
    writer
        .write_image_data(rgba)
        .map_err(|_| PdfError::Encode)?;
    writer.finish().map_err(|_| PdfError::Encode)?;
    Ok(out)
}

/// Names a file's tiles by its path, size and modification time, so a
/// replaced file never shows stale tiles.
fn fingerprint(path: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    if let Ok(meta) = std::fs::metadata(path) {
        meta.len().hash(&mut hasher);
        meta.modified().ok().hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

/// Removes the least recently written documents' tiles until the cache is
/// within `budget` bytes.
fn prune(cache: &Path, budget: u64) {
    let Ok(entries) = std::fs::read_dir(cache) else {
        return;
    };
    let mut folders: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| {
            let path = entry.path();
            let (mut size, mut newest) = (0, std::time::UNIX_EPOCH);
            for file in std::fs::read_dir(&path).into_iter().flatten().flatten() {
                if let Ok(meta) = file.metadata() {
                    size += meta.len();
                    newest = newest.max(meta.modified().unwrap_or(std::time::UNIX_EPOCH));
                }
            }
            (newest, size, path)
        })
        .collect();
    let mut total: u64 = folders.iter().map(|folder| folder.1).sum();
    folders.sort_by_key(|folder| folder.0);
    for (_, size, path) in folders {
        if total <= budget {
            break;
        }
        if std::fs::remove_dir_all(&path).is_ok() {
            total -= size;
        }
    }
}
