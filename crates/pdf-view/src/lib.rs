//! Tiled, virtual PDF rendering for the desktop app (slotledger aqchay).
//!
//! `layout` decides where pages sit and which tiles a viewport needs;
//! `engine` renders those tiles with PDFium on one thread, keeps decoded
//! pages and encoded tiles, and extracts text for selection and search.
//! iOS and Android show PDFs with their own system viewers instead.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
compile_error!("pdf-view is desktop-only (macOS and Windows)");

mod engine;
pub mod layout;

pub use engine::{DocId, Engine, Opened, Reply, SearchHit, TextRun};

#[derive(Debug, Clone, thiserror::Error)]
pub enum PdfError {
    #[error("PDFium could not be loaded: {0}")]
    Library(String),
    #[error("the PDF could not be opened: {0}")]
    Open(String),
    #[error("the PDF is no longer open")]
    Closed,
    #[error("the page could not be rendered")]
    Render,
    #[error("the tile could not be encoded")]
    Encode,
    #[error("the view no longer needs this tile")]
    Cancelled,
    #[error("the render thread has stopped")]
    Stopped,
}

/// The PDFium library file name the desktop bundles beside the app.
#[cfg(target_os = "macos")]
pub const LIBRARY_NAME: &str = "libpdfium.dylib";
#[cfg(target_os = "windows")]
pub const LIBRARY_NAME: &str = "pdfium.dll";
