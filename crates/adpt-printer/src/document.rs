//! Printing a PDF or PostScript document without a driver: the printer
//! renders it itself, wrapped in the PJL header that sets duplex and color.

use crate::{lpd, pjl, StudentId, QUEUE};
use std::fmt;
use std::io::Read as _;
use std::net::SocketAddr;

/// What one print asks for. Each copy is a separate upload of the
/// document, so the printer never has to interpret a copy count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrintOptions {
    pub copies: u32,
    pub settings: pjl::Settings,
}

/// The most copies one print sends; more is a typo, not a print run.
pub const MAX_COPIES: u32 = 99;

#[derive(Debug)]
pub enum PrintError {
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
            Self::NotPrintable => f.write_str("not a PDF or PostScript file"),
            Self::Copies => write!(f, "copies must be between 1 and {MAX_COPIES}"),
            Self::Lpd { accepted, error } => {
                write!(f, "after {accepted} accepted copies: {error}")
            }
        }
    }
}

impl std::error::Error for PrintError {}

/// Sends `copies` uploads of `document` to the campus queue on `server`,
/// billed to `student`, and returns the uploads sent. Nothing is sent for a
/// document the printer cannot render or a copy count outside the range;
/// after that the only failure is the printer's, with the copies it already
/// accepted.
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

#[cfg(test)]
#[path = "tests/document.rs"]
mod tests;
