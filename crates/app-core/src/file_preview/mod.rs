//! How a downloaded file is shown: the one decision every shell renders.
//!
//! A file's kind comes from its name (and, for a past exam, the type the
//! site validated); its viewer from the kind and what this platform can show
//! in the app. Shells render a viewer with their own components (PDF.js,
//! PDFKit or Quick Look, PdfRenderer; the bundled Markdown page) and never
//! classify a file themselves. What a platform shows in the app is fixed at
//! build time (`cfg`), not probed at run time.
//!
//! | Kind                     | Desktop | iOS        | Android |
//! |--------------------------|---------|------------|---------|
//! | PDF                      | PDF     | PDF        | PDF     |
//! | Markdown, text, code     | in app  | in app     | in app  |
//! | Image                    | in app  | in app     | in app  |
//! | Video, audio             | in app  | in app     | another app |
//! | Word, slides, sheets     | another app | Quick Look | another app |
//! | Notebook, web page       | another app | another app | another app |
//! | ZIP                      | folder  | folder     | folder  |
//!
//! A ZIP is listed as a folder (`Material::Archive`); one too large to list
//! opens in another app. Text over `MAX_TEXT_BYTES` is too large to read in the app. A file of no
//! known kind that reads as UTF-8 text is text; anything else opens in
//! another app.

pub mod archive;

use serde::{Deserialize, Serialize};
use std::{fs, io::Read, path::Path};

/// The most a text or Markdown file may hold to be read in the app.
pub(crate) const MAX_TEXT_BYTES: u64 = 1_000_000;
/// How much of a file is read to tell text from binary.
const SNIFF_BYTES: usize = 1024;

/// What a file is, by its name.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum FileKind {
    Pdf,
    Markdown,
    Text,
    Image,
    Video,
    Audio,
    Doc,
    Slides,
    Sheet,
    /// A ZIP, which Core lists as a folder.
    Zip,
    /// Any other archive, which only another app opens.
    Archive,
    Notebook,
    Web,
}

/// How a shell shows a file.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Viewer {
    Pdf,
    /// Read-only, its math set (the shells' Markdown viewers apply the same
    /// reading rules).
    Markdown,
    Text,
    Image,
    Video,
    Audio,
    /// An office document the system previews in the app (Quick Look); only
    /// iOS has one.
    Document,
    /// Text or Markdown over `MAX_TEXT_BYTES`; another app opens it.
    TooLarge,
    /// Another app opens it.
    External,
}

/// The extensions of each kind, lower case; the first kind that lists one
/// wins.
const KINDS: &[(FileKind, &[&str])] = &[
    (FileKind::Pdf, &["pdf"]),
    (FileKind::Markdown, &["md", "markdown"]),
    (FileKind::Notebook, &["ipynb"]),
    (FileKind::Web, &["html", "htm"]),
    (
        FileKind::Text,
        &[
            "txt", "tex", "bib", "js", "jsx", "ts", "tsx", "java", "py", "r", "m", "c", "cpp",
            "cxx", "cc", "h", "hpp", "rs", "go", "rb", "sh", "sql", "json", "yaml", "yml", "toml",
            "xml", "css", "csv", "tsv", "log", "v", "sv", "vhd", "vhdl", "asm", "s", "hs", "ml",
            "scala", "kt", "swift",
        ],
    ),
    (
        FileKind::Image,
        &[
            "png", "apng", "jpg", "jpeg", "jpe", "gif", "webp", "avif", "bmp", "ico", "svg", "tif",
            "tiff", "heic", "heif",
        ],
    ),
    (FileKind::Video, &["mp4", "m4v", "mov", "webm", "ogv"]),
    (
        FileKind::Audio,
        &["mp3", "m4a", "aac", "wav", "ogg", "oga", "flac", "opus"],
    ),
    (FileKind::Doc, &["doc", "docx", "odt", "rtf", "pages"]),
    (FileKind::Slides, &["ppt", "pptx", "odp", "key"]),
    (FileKind::Sheet, &["xls", "xlsx", "xlsm", "ods", "numbers"]),
    (FileKind::Zip, &["zip"]),
    (
        FileKind::Archive,
        &["7z", "rar", "tar", "gz", "tgz", "bz2", "xz"],
    ),
];

/// The kind a file name says, if any.
pub(crate) fn kind_of(name: &str) -> Option<FileKind> {
    let extension = Path::new(name).extension()?.to_str()?.to_ascii_lowercase();
    KINDS
        .iter()
        .find(|(_, extensions)| extensions.contains(&extension.as_str()))
        .map(|(kind, _)| *kind)
}

/// The kind a validated MIME type says, for files whose name is the
/// uploader's (past exams).
fn kind_of_type(mime: &str) -> Option<FileKind> {
    let mime = mime.split(';').next()?.trim().to_ascii_lowercase();
    match mime.as_str() {
        "application/pdf" => Some(FileKind::Pdf),
        "text/markdown" => Some(FileKind::Markdown),
        "text/plain" => Some(FileKind::Text),
        _ if mime.starts_with("image/") => Some(FileKind::Image),
        _ if mime.starts_with("video/") => Some(FileKind::Video),
        _ if mime.starts_with("audio/") => Some(FileKind::Audio),
        _ => None,
    }
}

/// How this platform shows a file of `kind`, before its size is known.
pub(crate) fn viewer_of_kind(kind: FileKind) -> Viewer {
    match kind {
        FileKind::Pdf => Viewer::Pdf,
        FileKind::Markdown => Viewer::Markdown,
        FileKind::Text => Viewer::Text,
        FileKind::Image => Viewer::Image,
        FileKind::Video => MEDIA.0,
        FileKind::Audio => MEDIA.1,
        FileKind::Doc | FileKind::Slides | FileKind::Sheet => OFFICE,
        // A ZIP Core could not list opens whole, like any other archive.
        FileKind::Zip | FileKind::Archive | FileKind::Notebook | FileKind::Web => Viewer::External,
    }
}

/// Video and audio play in the app, except on Android, whose system player
/// does it better than an embedded one.
#[cfg(not(target_os = "android"))]
const MEDIA: (Viewer, Viewer) = (Viewer::Video, Viewer::Audio);
#[cfg(target_os = "android")]
const MEDIA: (Viewer, Viewer) = (Viewer::External, Viewer::External);

/// Quick Look previews office documents inside the app; the desktop
/// webview and Android have no such viewer.
#[cfg(target_os = "ios")]
const OFFICE: Viewer = Viewer::Document;
#[cfg(not(target_os = "ios"))]
const OFFICE: Viewer = Viewer::External;

/// How a file is shown. `name` is the name it was given (an upload's, for a
/// past exam), `mime` the type a site validated, `bytes` its size and `head`
/// its first bytes.
///
/// A validated type wins over the name, except that a `text/plain` file
/// named as Markdown is Markdown (the exam site stores Markdown so).
pub fn viewer(name: &str, mime: Option<&str>, bytes: u64, head: &[u8]) -> Viewer {
    let named = kind_of(name);
    let kind = match mime.and_then(kind_of_type) {
        Some(FileKind::Text) if named == Some(FileKind::Markdown) => Some(FileKind::Markdown),
        Some(kind) => Some(kind),
        None => named,
    };
    let kind = kind.or_else(|| reads_as_text(head).then_some(FileKind::Text));
    match kind.map(viewer_of_kind) {
        Some(Viewer::Text | Viewer::Markdown) if bytes > MAX_TEXT_BYTES => Viewer::TooLarge,
        Some(viewer) => viewer,
        None => Viewer::External,
    }
}

/// [`viewer`] of the file at `path`, reading its first bytes; a file that
/// cannot be read opens in another app.
pub(crate) fn viewer_of_file(path: &Path, name: &str, mime: Option<&str>) -> Viewer {
    let mut head = Vec::with_capacity(SNIFF_BYTES);
    let bytes = fs::File::open(path)
        .and_then(|file| {
            let bytes = file.metadata()?.len();
            file.take(SNIFF_BYTES as u64).read_to_end(&mut head)?;
            Ok(bytes)
        })
        .ok();
    match bytes {
        Some(bytes) => viewer(name, mime, bytes, &head),
        None => Viewer::External,
    }
}

/// Text with no NUL that is UTF-8, allowing for a character cut off where
/// the sniffed bytes end.
fn reads_as_text(head: &[u8]) -> bool {
    !head.is_empty()
        && !head.contains(&0)
        && match std::str::from_utf8(head) {
            Ok(_) => true,
            Err(error) => error.error_len().is_none(),
        }
}

/// The kinds and their extensions, as the desktop frontend reads them before
/// a file exists (an item's icon, whether it previews or waits for
/// Download), with this build's viewer of each.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FileKindEntry {
    pub kind: FileKind,
    pub extensions: Vec<String>,
    pub viewer: Viewer,
}

/// Every kind with its extensions and viewer, in the order `kind_of` reads.
pub fn catalog() -> Vec<FileKindEntry> {
    KINDS
        .iter()
        .map(|(kind, extensions)| FileKindEntry {
            kind: *kind,
            extensions: extensions
                .iter()
                .map(|extension| (*extension).into())
                .collect(),
            viewer: viewer_of_kind(*kind),
        })
        .collect()
}

#[cfg(feature = "ts")]
pub fn declarations(cfg: &ts_rs::Config) -> Vec<String> {
    use ts_rs::TS;
    vec![
        FileKind::decl(cfg),
        Viewer::decl(cfg),
        FileKindEntry::decl(cfg),
    ]
}

#[cfg(test)]
#[path = "../tests/file_preview.rs"]
mod tests;
