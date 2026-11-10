//! Printing a PDF or PostScript file without a driver: the printer renders
//! the file itself, wrapped in the PJL header that sets duplex and color.

use crate::{lpd, pjl, StudentId, QUEUE};
use serde::Deserialize;
use std::fmt;
use std::io::{self, Read as _};
use std::net::SocketAddr;
use std::path::Path;

/// What the print window offers. Each copy is a separate upload of the
/// document, so the printer never has to interpret a copy count.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PrintOptions {
    pub copies: u32,
    #[serde(flatten)]
    pub settings: pjl::Settings,
}

/// The most copies one print sends; more is a typo, not a print run.
pub const MAX_COPIES: u32 = 99;

#[derive(Debug)]
pub enum PrintError {
    /// The file could not be read; nothing was sent.
    File(io::Error),
    /// Not PDF or PostScript: the printer cannot render it.
    NotPrintable,
    /// `copies` is outside 1..=`MAX_COPIES`.
    Copies,
    /// The next copy did not reach the printer; the `accepted` copies
    /// before it did.
    Lpd { accepted: u32, error: lpd::LpdError },
}

impl fmt::Display for PrintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(error) => write!(f, "cannot read the file: {error}"),
            Self::NotPrintable => f.write_str("not a PDF or PostScript file"),
            Self::Copies => write!(f, "copies must be between 1 and {MAX_COPIES}"),
            Self::Lpd { accepted, error } => {
                write!(f, "after {accepted} accepted copies: {error}")
            }
        }
    }
}

impl std::error::Error for PrintError {}

/// Sends `copies` uploads of the file at `path` to the campus queue on
/// `server`, billed to `student`. Returns the uploads sent. The file is
/// read once, before anything is sent, so every copy is the same document
/// whatever happens to the file meanwhile, and the only failure after the
/// first upload is the printer's, with the copies it already accepted.
pub fn print(
    server: SocketAddr,
    student: &StudentId,
    path: &Path,
    options: &PrintOptions,
) -> Result<u32, PrintError> {
    if !(1..=MAX_COPIES).contains(&options.copies) {
        return Err(PrintError::Copies);
    }
    let document = std::fs::read(path).map_err(PrintError::File)?;
    let title = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    print_bytes(server, student, &document, &title, options)
}

/// Sends `copies` uploads of a document already in memory, such as the PDF
/// the system rendered for its print queue or print service. Same contract as [`print`].
pub fn print_bytes(
    server: SocketAddr,
    student: &StudentId,
    document: &[u8],
    title: &str,
    options: &PrintOptions,
) -> Result<u32, PrintError> {
    if !(1..=MAX_COPIES).contains(&options.copies) {
        return Err(PrintError::Copies);
    }
    let language = pjl::Language::detect(document).ok_or(PrintError::NotPrintable)?;
    let header = pjl::header(language, &options.settings);
    let title = lpd::sanitize(title);
    let host = lpd::local_hostname();
    let data_len = (header.len() + document.len() + pjl::JOB_END.len()) as u64;
    let mut number = lpd::first_job_number();
    for accepted in 0..options.copies {
        let job = lpd::Job {
            number,
            host: &host,
            user: student,
            title: &title,
        };
        number = (number + 1) % 1000;
        let mut data = header.as_slice().chain(document).chain(pjl::JOB_END);
        lpd::send(server, QUEUE, &job, &mut data, data_len)
            .map_err(|error| PrintError::Lpd { accepted, error })?;
    }
    Ok(options.copies)
}

// The tests bind the reserved ports the Windows client sends from.
#[cfg(all(test, windows))]
#[path = "tests/document.rs"]
mod tests;
