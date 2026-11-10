//! Android's print service hands Core the PDF the system rendered; Core
//! verifies whose student ID is billed and sends it to the campus printer.
//! The binding is one surface for both phones (it is generated from the
//! host's library), so the call exists everywhere and only Android's
//! library has the printer behind it.

use crate::{Operation, SdkError, Session};
use std::sync::Arc;

/// The largest document the print service may hand over, the size the
/// Windows queue takes too. The document is held in memory until its last
/// copy is sent, and the shell stops reading at this size.
const MAX_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;

/// How many bytes `Session::campus_print` takes: the shell reads the
/// system's document only up to this, so a larger one is never loaded.
#[uniffi::export]
pub fn campus_print_document_limit() -> u64 {
    MAX_DOCUMENT_BYTES as u64
}

/// Which sides the print dialog asked for.
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum CampusPrintSides {
    OneSided,
    LongEdge,
    ShortEdge,
}

/// What the system's print dialog chose for one job. A setting left out is
/// not sent, so the printer's own default applies.
#[derive(Clone, Debug, uniffi::Record)]
pub struct CampusPrintJob {
    /// The job's name at the printer.
    pub title: String,
    pub copies: u32,
    pub sides: Option<CampusPrintSides>,
    /// `true` color, `false` black and white.
    pub color: Option<bool>,
}

/// How a print ended (`app_core::campus_printer::CampusPrintOutcome`, with
/// the account verdicts flattened for the shell's one message per case).
#[derive(Clone, Debug, uniffi::Enum)]
pub enum CampusPrintResult {
    /// Every copy reached the printer.
    Printed { copies: u32 },
    /// No Tokyo Tech account is saved: nothing was sent.
    NotConnected,
    /// The account's name is not a student ID: nothing was sent.
    NotStudentId,
    /// The school refused the saved account: nothing was sent.
    NeedsAttention,
    /// The account could not be verified now: nothing was sent.
    Unverified,
    /// Not a PDF, or more copies than one print sends: nothing was sent.
    NotPrintable,
    /// Larger than `campus_print_document_limit`: nothing was sent.
    TooLarge,
    /// No connection for the next copy; `printed` were accepted before it.
    Unreachable { printed: u32 },
    /// The printer did not take the next copy; `printed` were accepted.
    NotPrinted { printed: u32 },
    /// The next copy went out unanswered and may have printed; `printed`
    /// were accepted before it.
    Uncertain { printed: u32 },
}

#[uniffi::export]
impl Session {
    /// Sends `document`, the PDF the system rendered, to the campus
    /// printer, billed to the verified Tokyo Tech student ID. Core owns the
    /// send past this operation's waiters.
    pub fn campus_print(
        &self,
        document: Vec<u8>,
        job: CampusPrintJob,
    ) -> Result<Arc<Operation>, SdkError> {
        #[cfg(target_os = "android")]
        {
            self.start(false, move |inner, _| async move {
                Ok(crate::OperationResult::CampusPrint {
                    result: android::print(&inner.core, document, job).await,
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
    use super::{CampusPrintJob, CampusPrintResult, CampusPrintSides, MAX_DOCUMENT_BYTES};
    use app_core::campus_printer::{
        pjl::{ColorMode, Duplex, Settings},
        CampusPrintOutcome, PrintOptions, PrinterAccount,
    };

    pub(super) async fn print(
        core: &app_core::Core,
        document: Vec<u8>,
        job: CampusPrintJob,
    ) -> CampusPrintResult {
        if document.len() > MAX_DOCUMENT_BYTES {
            return CampusPrintResult::TooLarge;
        }
        let options = PrintOptions {
            copies: job.copies,
            settings: Settings {
                duplex: job.sides.map(|sides| match sides {
                    CampusPrintSides::OneSided => Duplex::Off,
                    CampusPrintSides::LongEdge => Duplex::LongEdge,
                    CampusPrintSides::ShortEdge => Duplex::ShortEdge,
                }),
                color: job.color.map(|color| {
                    if color {
                        ColorMode::Color
                    } else {
                        ColorMode::Mono
                    }
                }),
            },
        };
        match core
            .campus_print_rendered(document, job.title, options)
            .await
        {
            CampusPrintOutcome::Printed { copies } => CampusPrintResult::Printed { copies },
            CampusPrintOutcome::Account { account } => match account {
                PrinterAccount::NotConnected => CampusPrintResult::NotConnected,
                PrinterAccount::NotStudentId => CampusPrintResult::NotStudentId,
                PrinterAccount::NeedsAttention => CampusPrintResult::NeedsAttention,
                // `Ready` never comes back as a refusal.
                PrinterAccount::Unverified | PrinterAccount::Ready { .. } => {
                    CampusPrintResult::Unverified
                }
            },
            // `Unreadable` is a file's failure; a rendered document has none.
            CampusPrintOutcome::NotPrintable
            | CampusPrintOutcome::InvalidCopies
            | CampusPrintOutcome::Unreadable => CampusPrintResult::NotPrintable,
            CampusPrintOutcome::Unreachable { printed } => {
                CampusPrintResult::Unreachable { printed }
            }
            CampusPrintOutcome::NotPrinted { printed } => CampusPrintResult::NotPrinted { printed },
            CampusPrintOutcome::Uncertain { printed } => CampusPrintResult::Uncertain { printed },
        }
    }
}
