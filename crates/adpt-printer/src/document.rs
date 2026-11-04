//! Printing a PDF or PostScript file without a driver: the printer renders
//! the file itself, wrapped in the PJL header that sets duplex and color.

use crate::{lpd, pjl, StudentId, QUEUE};
use serde::Deserialize;
use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::net::SocketAddr;
use std::path::Path;

/// What the print window offers. Each copy is a separate upload of one
/// copy, as the CUPS backend does, so the printer never has to interpret a
/// copy count.
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
    /// The file could not be opened or read.
    File(io::Error),
    /// Not PDF or PostScript: the printer cannot render it.
    NotPrintable,
    /// `copies` is outside 1..=`MAX_COPIES`.
    Copies,
    /// Copy `copy` (counted from 1) did not reach the printer; the earlier
    /// ones did.
    Lpd { copy: u32, error: lpd::LpdError },
}

impl fmt::Display for PrintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(error) => write!(f, "cannot read the file: {error}"),
            Self::NotPrintable => f.write_str("not a PDF or PostScript file"),
            Self::Copies => write!(f, "copies must be between 1 and {MAX_COPIES}"),
            Self::Lpd { copy, error } => write!(f, "copy {copy}: {error}"),
        }
    }
}

impl std::error::Error for PrintError {}

/// Sends `copies` uploads of the file at `path` to the campus queue on
/// `server`, billed to `student`. Returns the uploads sent.
pub fn print(
    server: SocketAddr,
    student: &StudentId,
    path: &Path,
    options: &PrintOptions,
) -> Result<u32, PrintError> {
    if !(1..=MAX_COPIES).contains(&options.copies) {
        return Err(PrintError::Copies);
    }
    let header = pjl::header(detect(path)?, &options.settings);
    let title = lpd::sanitize(
        &path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    let host = lpd::local_hostname();
    let mut number = lpd::first_job_number();
    for copy in 1..=options.copies {
        let job = lpd::Job {
            number,
            host: &host,
            user: student,
            title: &title,
        };
        number = (number + 1) % 1000;
        // Opened per copy, so each upload announces the length it sends.
        let file = File::open(path).map_err(PrintError::File)?;
        let file_len = file.metadata().map_err(PrintError::File)?.len();
        let data_len = header.len() as u64 + file_len + pjl::JOB_END.len() as u64;
        let mut data = header
            .as_slice()
            .chain(file.take(file_len))
            .chain(pjl::JOB_END);
        lpd::send(server, QUEUE, &job, &mut data, data_len)
            .map_err(|error| PrintError::Lpd { copy, error })?;
    }
    Ok(options.copies)
}

fn detect(path: &Path) -> Result<pjl::Language, PrintError> {
    let mut head = Vec::new();
    File::open(path)
        .and_then(|file| file.take(8).read_to_end(&mut head))
        .map_err(PrintError::File)?;
    pjl::Language::detect(&head).ok_or(PrintError::NotPrintable)
}

#[cfg(test)]
#[path = "tests/document.rs"]
mod tests;
