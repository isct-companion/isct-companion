//! The PJL wrapper around the print data. The job start, language switch
//! and job end strings are the ones in Kyocera's PPD for this printer
//! (`*JCLBegin`, `*JCLToPSInterpreter`, `*JCLEnd`). The PPD sets duplex and
//! color mode with PostScript code, which PDF direct printing does not run,
//! so those settings use the standard HP PJL variables instead.

use serde::{Deserialize, Serialize};

const JOB_BEGIN: &[u8] = b"\x1b%-12345X@PJL JOB\n";
pub const JOB_END: &[u8] = b"\x1b%-12345X@PJL EOJ\n\x1b%-12345X";
/// How far into a file the `%PDF-` header may start (ISO 32000 readers' limit).
const PDF_HEADER_WINDOW: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Pdf,
    PostScript,
}

impl Language {
    /// PDF readers accept the `%PDF-` header anywhere in the first 1024
    /// bytes (a BOM or a mail gateway's junk before it); the bytes are sent
    /// unchanged, so the file's own offsets stay valid.
    pub fn detect(head: &[u8]) -> Option<Language> {
        let window = &head[..head.len().min(PDF_HEADER_WINDOW)];
        if window.windows(5).any(|bytes| bytes == b"%PDF-") {
            Some(Language::Pdf)
        } else if head.starts_with(b"%!") {
            Some(Language::PostScript)
        } else {
            None
        }
    }

    fn pjl_name(self) -> &'static str {
        match self {
            Language::Pdf => "PDF",
            Language::PostScript => "POSTSCRIPT",
        }
    }
}

/// Which sides of the sheet a page goes on: the one sides model the print
/// window, the Windows queue's IPP `sides` and Android's duplex mode map into.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Sides {
    OneSided,
    LongEdge,
    ShortEdge,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ColorMode {
    Color,
    Mono,
}

/// Every job states both, so the printer's own panel defaults never decide.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Settings {
    pub sides: Sides,
    pub color: ColorMode,
}

pub fn header(language: Language, settings: &Settings) -> Vec<u8> {
    let mut out = JOB_BEGIN.to_vec();
    out.extend_from_slice(match settings.sides {
        Sides::OneSided => b"@PJL SET DUPLEX=OFF\n".as_slice(),
        Sides::LongEdge => b"@PJL SET DUPLEX=ON\n@PJL SET BINDING=LONGEDGE\n",
        Sides::ShortEdge => b"@PJL SET DUPLEX=ON\n@PJL SET BINDING=SHORTEDGE\n",
    });
    out.extend_from_slice(match settings.color {
        ColorMode::Color => b"@PJL SET RENDERMODE=COLOR\n".as_slice(),
        ColorMode::Mono => b"@PJL SET RENDERMODE=GRAYSCALE\n",
    });
    out.extend_from_slice(format!("@PJL ENTER LANGUAGE={}\n", language.pjl_name()).as_bytes());
    out
}

#[cfg(test)]
#[path = "tests/pjl.rs"]
mod tests;
