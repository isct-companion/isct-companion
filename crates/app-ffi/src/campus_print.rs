#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
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
    pub id: String,
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

#[derive(Clone, Debug, uniffi::Enum)]
pub enum CampusPrintAdmission {
    Ready { lease: u64 },
    Busy,
    Unknown { printed: u32 },
}

#[uniffi::export]
impl Session {
    pub fn campus_print_reserve(
        &self,
        job: CampusPrintJob,
    ) -> Result<CampusPrintAdmission, SdkError> {
        #[cfg(target_os = "android")]
        {
            return self
                .inner
                .campus_print
                .reserve(&android::job(&job))
                .map(android::admission)
                .map_err(android::error);
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = job;
            Err(crate::session::unavailable("Campus printer"))
        }
    }
    pub fn campus_print_wait_available(&self) -> Result<Arc<Operation>, SdkError> {
        #[cfg(target_os = "android")]
        {
            self.start(true, |inner, cancel| async move {
                tokio::select! {
                    result = inner.campus_print.wait_available() => { result.map_err(android::error)?; Ok(crate::OperationResult::Done) }
                    _ = cancel.cancelled() => Err(SdkError::Cancelled),
                }
            })
        }
        #[cfg(not(target_os = "android"))]
        {
            self.blocking(|_| Err(crate::session::unavailable("Campus printer")))
        }
    }
    pub fn campus_print_cancel(&self, id: String) -> Result<bool, SdkError> {
        #[cfg(target_os = "android")]
        {
            return self.inner.campus_print.cancel(&id).map_err(android::error);
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = id;
            Err(crate::session::unavailable("Campus printer"))
        }
    }
    pub fn campus_print_forget_completed(&self, id: String) -> Result<(), SdkError> {
        #[cfg(target_os = "android")]
        {
            return self
                .inner
                .campus_print
                .forget_completed(&id)
                .map_err(android::error);
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = id;
            Err(crate::session::unavailable("Campus printer"))
        }
    }
    pub fn campus_print_release(&self, id: String, lease: u64) {
        #[cfg(target_os = "android")]
        self.inner.campus_print.release(&id, lease);
        #[cfg(not(target_os = "android"))]
        let _ = (id, lease);
    }
    pub fn campus_print(
        &self,
        document: Vec<u8>,
        job: CampusPrintJob,
        lease: u64,
    ) -> Result<Arc<Operation>, SdkError> {
        #[cfg(target_os = "android")]
        {
            self.start(false, move |inner, _| async move {
                Ok(crate::OperationResult::CampusPrint {
                    result: android::print(
                        &inner.core,
                        inner.campus_print.clone(),
                        document,
                        job,
                        lease,
                    )
                    .await?,
                })
            })
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = (document, job, lease);
            self.blocking(|_| Err(crate::session::unavailable("Campus printer")))
        }
    }
}

#[cfg(target_os = "android")]
mod android {
    use super::{CampusPrintAdmission, CampusPrintJob, CampusPrintResult, CampusPrintSides};
    use crate::SdkError;
    use app_core::campus_printer::{
        android::{Admission, Error, Job, Service},
        CampusPrintOutcome, PrinterAccount,
    };
    use std::sync::Arc;

    pub(super) fn job(job: &CampusPrintJob) -> Job {
        Job {
            id: job.id.clone(),
            title: job.title.clone(),
            copies: job.copies,
            sides: job.sides.map(|side| match side {
                CampusPrintSides::OneSided => 0,
                CampusPrintSides::LongEdge => 1,
                CampusPrintSides::ShortEdge => 2,
            }),
            color: job.color,
        }
    }
    pub(super) fn admission(value: Admission) -> CampusPrintAdmission {
        match value {
            Admission::Ready { lease } => CampusPrintAdmission::Ready { lease },
            Admission::Busy => CampusPrintAdmission::Busy,
            Admission::Unknown { printed } => CampusPrintAdmission::Unknown { printed },
        }
    }
    pub(super) fn error(error: Error) -> SdkError {
        let detail = "campus-print".to_string();
        match error {
            Error::Storage => SdkError::Storage { detail },
            Error::Conflict => SdkError::Conflict { detail },
            Error::Invalid | Error::TooLarge => SdkError::InvalidInput { detail },
            Error::Cancelled => SdkError::Cancelled,
        }
    }
    pub(super) async fn print(
        core: &Arc<app_core::Core>,
        service: Arc<Service>,
        document: Vec<u8>,
        input: CampusPrintJob,
        lease: u64,
    ) -> Result<CampusPrintResult, SdkError> {
        let outcome = service
            .submit(core.clone(), job(&input), document, lease)
            .await
            .map_err(error)?;
        Ok(match outcome {
            CampusPrintOutcome::Printed { copies } => CampusPrintResult::Printed { copies },
            CampusPrintOutcome::Account { account } => match account {
                PrinterAccount::NotConnected => CampusPrintResult::NotConnected,
                PrinterAccount::NotStudentId => CampusPrintResult::NotStudentId,
                PrinterAccount::NeedsAttention => CampusPrintResult::NeedsAttention,
                PrinterAccount::Unverified | PrinterAccount::Ready { .. } => {
                    CampusPrintResult::Unverified
                }
            },
            CampusPrintOutcome::NotPrintable
            | CampusPrintOutcome::InvalidCopies
            | CampusPrintOutcome::Unreadable => CampusPrintResult::NotPrintable,
            CampusPrintOutcome::Unreachable { printed } => {
                CampusPrintResult::Unreachable { printed }
            }
            CampusPrintOutcome::NotPrinted { printed } => CampusPrintResult::NotPrinted { printed },
            CampusPrintOutcome::Uncertain { printed } => CampusPrintResult::Uncertain { printed },
        })
    }
}
