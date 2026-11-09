//! The PJL wrapper around the print data. The job start, language switch
//! and job end strings are the ones in Kyocera's PPD for this printer
//! (`*JCLBegin`, `*JCLToPSInterpreter`, `*JCLEnd`). The PPD sets duplex and
//! color mode with PostScript code, which PDF direct printing does not run,
//! so those settings use the standard HP PJL variables instead.

use serde::Deserialize;

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

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Duplex {
    Off,
    LongEdge,
    ShortEdge,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ColorMode {
    Color,
    Mono,
}

/// Settings left as `None` are not sent, so the printer's own defaults apply.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Settings {
    pub duplex: Option<Duplex>,
    pub color: Option<ColorMode>,
}

pub fn header(language: Language, settings: &Settings) -> Vec<u8> {
    let mut out = JOB_BEGIN.to_vec();

    match settings.duplex {
        Some(Duplex::Off) => out.extend_from_slice(b"@PJL SET DUPLEX=OFF\n"),
        Some(Duplex::LongEdge) => {
            out.extend_from_slice(b"@PJL SET DUPLEX=ON\n@PJL SET BINDING=LONGEDGE\n")
        }
        Some(Duplex::ShortEdge) => {
            out.extend_from_slice(b"@PJL SET DUPLEX=ON\n@PJL SET BINDING=SHORTEDGE\n")
        }
        None => {}
    }

    match settings.color {
        Some(ColorMode::Color) => out.extend_from_slice(b"@PJL SET RENDERMODE=COLOR\n"),
        Some(ColorMode::Mono) => out.extend_from_slice(b"@PJL SET RENDERMODE=GRAYSCALE\n"),
        None => {}
    }

    out.extend_from_slice(format!("@PJL ENTER LANGUAGE={}\n", language.pjl_name()).as_bytes());
    out
}

#[cfg(test)]
#[path = "tests/pjl.rs"]
mod tests;
