#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
//! Android's print service hands Core the PDF the system rendered; Core
//! verifies whose student ID is billed, sends it to the campus printer and
//! words how it ended. The binding is one surface for both phones (it is
//! generated from the host's library), so the calls exist everywhere and
//! only Android's library has the printer behind them.

use crate::{Operation, SdkError, Session};
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum CampusPrintSides {
    OneSided,
    LongEdge,
    ShortEdge,
}

#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum CampusPrintColor {
    Mono,
    Color,
}

#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum CampusPrintMedia {
    A4,
    Letter,
}

/// What the campus printer offers: Core's one capability set, which the
/// print service lists its printer with.
#[derive(Clone, Debug, uniffi::Record)]
pub struct CampusPrinterCapabilities {
    /// The paper it takes; the first is the default.
    pub media: Vec<CampusPrintMedia>,
    pub default_sides: CampusPrintSides,
    pub default_color: CampusPrintColor,
    pub max_copies: u32,
    /// The shell stops reading a document at this size.
    pub max_document_bytes: u64,
    /// The unprintable edge on every side, in thousandths of an inch.
    pub margin_mils: i32,
}

/// What the system's print dialog chose for one job; a setting it left out
/// gets the printer's default.
#[derive(Clone, Debug, uniffi::Record)]
pub struct CampusPrintJob {
    pub title: String,
    pub copies: u32,
    pub sides: Option<CampusPrintSides>,
    pub color: Option<CampusPrintColor>,
}

/// Words for the person: a catalog key and its placeholder values.
#[derive(Clone, Debug, uniffi::Record)]
pub struct CampusPrintMessage {
    pub key: String,
    pub args: HashMap<String, u32>,
}

/// How a print ended: whether every copy reached the printer, and the
/// message to show either way.
#[derive(Clone, Debug, uniffi::Record)]
pub struct CampusPrintReport {
    pub printed: bool,
    pub message: CampusPrintMessage,
}

#[uniffi::export]
pub fn campus_printer_capabilities() -> Result<CampusPrinterCapabilities, SdkError> {
    #[cfg(target_os = "android")]
    {
        Ok(android::capabilities())
    }
    #[cfg(not(target_os = "android"))]
    {
        Err(crate::session::unavailable("Campus printer"))
    }
}

#[uniffi::export]
impl Session {
    /// Sends one job; it is not cancellable once called.
    pub fn campus_print(
        &self,
        document: Vec<u8>,
        job: CampusPrintJob,
    ) -> Result<Arc<Operation>, SdkError> {
        #[cfg(target_os = "android")]
        {
            self.start(false, move |inner, _| async move {
                Ok(crate::OperationResult::CampusPrint {
                    report: android::print(&inner.core, document, job).await,
                })
            })
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = (document, job);
            self.blocking(|_| Err(crate::session::unavailable("Campus printer")))
        }
    }
}

#[cfg(target_os = "android")]
mod android {
    use super::{
        CampusPrintColor, CampusPrintJob, CampusPrintMedia, CampusPrintMessage, CampusPrintReport,
        CampusPrintSides, CampusPrinterCapabilities,
    };
    use app_core::campus_printer::{ColorMode, Media, PrintOptions, Settings, Sides, CAPABILITIES};

    fn sides(sides: Sides) -> CampusPrintSides {
        match sides {
            Sides::OneSided => CampusPrintSides::OneSided,
            Sides::LongEdge => CampusPrintSides::LongEdge,
            Sides::ShortEdge => CampusPrintSides::ShortEdge,
        }
    }

    fn color(color: ColorMode) -> CampusPrintColor {
        match color {
            ColorMode::Mono => CampusPrintColor::Mono,
            ColorMode::Color => CampusPrintColor::Color,
        }
    }

    pub(super) fn capabilities() -> CampusPrinterCapabilities {
        CampusPrinterCapabilities {
            media: CAPABILITIES
                .media
                .iter()
                .map(|media| match media {
                    Media::A4 => CampusPrintMedia::A4,
                    Media::Letter => CampusPrintMedia::Letter,
                })
                .collect(),
            default_sides: sides(CAPABILITIES.defaults.sides),
            default_color: color(CAPABILITIES.defaults.color),
            max_copies: CAPABILITIES.max_copies,
            max_document_bytes: CAPABILITIES.max_document_bytes as u64,
            // Hundredths of a millimetre to thousandths of an inch, rounded.
            margin_mils: (CAPABILITIES.margin * 1000 + 1270) / 2540,
        }
    }

    pub(super) async fn print(
        core: &app_core::Core,
        document: Vec<u8>,
        job: CampusPrintJob,
    ) -> CampusPrintReport {
        let defaults = CAPABILITIES.defaults;
        let options = PrintOptions {
            copies: job.copies,
            settings: Settings {
                sides: job.sides.map_or(defaults.sides, |chosen| match chosen {
                    CampusPrintSides::OneSided => Sides::OneSided,
                    CampusPrintSides::LongEdge => Sides::LongEdge,
                    CampusPrintSides::ShortEdge => Sides::ShortEdge,
                }),
                color: job.color.map_or(defaults.color, |chosen| match chosen {
                    CampusPrintColor::Mono => ColorMode::Mono,
                    CampusPrintColor::Color => ColorMode::Color,
                }),
            },
        };
        let report = core
            .campus_print(document.into(), job.title, options)
            .await
            .report();
        CampusPrintReport {
            printed: report.printed,
            message: CampusPrintMessage {
                key: report.message.key.into(),
                args: report
                    .message
                    .args
                    .into_iter()
                    .map(|(name, value)| (name.into(), value))
                    .collect(),
            },
        }
    }
}
