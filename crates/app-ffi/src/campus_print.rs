#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
//! Both phones' print screen hands Core a PDF (a picture already laid out
//! on its page) with the pages chosen; Core verifies whose student ID is
//! billed, sends it to the campus printer and words how it ended. The binding is one surface
//! for both phones (it is generated from the host's library), so the calls
//! exist everywhere and only the phones' libraries have the printer behind
//! them.

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
/// print screen offers its choices from.
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

/// What the system's print dialog or the app's print screen chose for one
/// job; a setting it left out gets the printer's default.
#[derive(Clone, Debug, uniffi::Record)]
pub struct CampusPrintJob {
    pub title: String,
    pub copies: u32,
    pub sides: Option<CampusPrintSides>,
    pub color: Option<CampusPrintColor>,
    /// The pages to print, 1-based; empty prints every page. The system's
    /// dialog renders only the chosen pages already and leaves it empty.
    pub pages: Vec<u32>,
}

/// Whether a print would go through now, for the print screen to show
/// before anything is sent: the student ID it bills, or why not.
#[derive(Clone, Debug, uniffi::Record)]
pub struct CampusPrinterState {
    /// Set when a print would go through.
    pub student_id: Option<String>,
    /// Set when it would not, in the words its print would end with.
    pub problem: Option<CampusPrintMessage>,
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
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        Ok(phone::capabilities())
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        Err(crate::session::unavailable("Campus printer"))
    }
}

#[uniffi::export]
impl Session {
    /// Verifies the account and tries the printer, as the print screen
    /// opens and when the person checks again.
    pub fn campus_printer_state(&self) -> Result<Arc<Operation>, SdkError> {
        #[cfg(any(target_os = "android", target_os = "ios"))]
        {
            self.start(false, move |inner, _| async move {
                Ok(crate::OperationResult::CampusPrinterState {
                    state: phone::state(&inner.core).await,
                })
            })
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            self.blocking(|_| Err(crate::session::unavailable("Campus printer")))
        }
    }

    /// Sends one job; it is not cancellable once called.
    pub fn campus_print(
        &self,
        document: Vec<u8>,
        job: CampusPrintJob,
    ) -> Result<Arc<Operation>, SdkError> {
        #[cfg(any(target_os = "android", target_os = "ios"))]
        {
            self.start(false, move |inner, _| async move {
                Ok(crate::OperationResult::CampusPrint {
                    report: phone::print(&inner.core, document, job).await,
                })
            })
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            let _ = (document, job);
            self.blocking(|_| Err(crate::session::unavailable("Campus printer")))
        }
    }
}

#[cfg(any(target_os = "android", target_os = "ios"))]
mod phone {
    use super::{
        CampusPrintColor, CampusPrintJob, CampusPrintMedia, CampusPrintMessage, CampusPrintReport,
        CampusPrintSides, CampusPrinterCapabilities, CampusPrinterState,
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
            .campus_print_pages(document.into(), job.title, options, job.pages)
            .await
            .report();
        CampusPrintReport {
            printed: report.printed,
            message: message(report.message),
        }
    }

    pub(super) async fn state(core: &app_core::Core) -> CampusPrinterState {
        let status = core.campus_printer_status().await;
        CampusPrinterState {
            student_id: status.student_id().map(str::to_owned),
            problem: status.problem().map(message),
        }
    }

    fn message(message: app_core::campus_printer::CampusPrintMessage) -> CampusPrintMessage {
        CampusPrintMessage {
            key: message.key.into(),
            args: message
                .args
                .into_iter()
                .map(|(name, value)| (name.into(), value))
                .collect(),
        }
    }
}
