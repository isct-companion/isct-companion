//! Keeping only the pages a person chose before a PDF is sent. The printer
//! renders the PDF itself and PJL has no standard way to pick pages, so the
//! document that is sent holds just those pages, in their order in the
//! file; everything else in it (fonts, forms, links) is kept for them.

use lopdf::Document;
use std::collections::BTreeSet;
use std::fmt;

/// How far into a file the `%PDF-` header may start (as `pjl::Language`).
const PDF_HEADER_WINDOW: usize = 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum PagesError {
    /// Not a PDF: pages of PostScript or anything else cannot be picked.
    NotPdf,
    /// A PDF whose page tree cannot be read or written again, such as one
    /// locked with an open password.
    Unreadable,
    /// No page chosen, or a page number past the document's end.
    OutOfRange,
}

impl fmt::Display for PagesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotPdf => "not a PDF",
            Self::Unreadable => "the PDF's pages cannot be read",
            Self::OutOfRange => "the chosen pages are not in the document",
        })
    }
}

impl std::error::Error for PagesError {}

/// The PDF's page count, for a shell that has the file but not its pages.
pub fn count(document: &[u8]) -> Result<u32, PagesError> {
    let pdf = load(document)?;
    u32::try_from(pdf.get_pages().len()).map_err(|_| PagesError::Unreadable)
}

/// `document` with only `pages` (1-based, any order, repeats ignored).
/// Every page chosen: the document unchanged, byte for byte.
pub fn keep(document: &[u8], pages: &[u32]) -> Result<Vec<u8>, PagesError> {
    let chosen: BTreeSet<u32> = pages.iter().copied().collect();
    if chosen.is_empty() {
        return Err(PagesError::OutOfRange);
    }
    let mut pdf = load(document)?;
    let all = pdf.get_pages();
    if chosen.iter().any(|page| !all.contains_key(page)) {
        return Err(PagesError::OutOfRange);
    }
    let dropped: Vec<u32> = all
        .keys()
        .copied()
        .filter(|page| !chosen.contains(page))
        .collect();
    if dropped.is_empty() {
        return Ok(document.to_vec());
    }
    pdf.delete_pages(&dropped);
    pdf.prune_objects();
    let mut out = Vec::with_capacity(document.len());
    pdf.save_to(&mut out).map_err(|_| PagesError::Unreadable)?;
    Ok(out)
}

fn load(document: &[u8]) -> Result<Document, PagesError> {
    let window = &document[..document.len().min(PDF_HEADER_WINDOW)];
    if !window.windows(5).any(|bytes| bytes == b"%PDF-") {
        return Err(PagesError::NotPdf);
    }
    Document::load_mem(document).map_err(|_| PagesError::Unreadable)
}

#[cfg(test)]
#[path = "tests/pages.rs"]
mod tests;
