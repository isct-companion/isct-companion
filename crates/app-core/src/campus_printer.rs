//! The campus printer (lpd://172.20.83.212/kyocera), desktop only.
//!
//! The printer bills each job to the student ID in the LPD user field and
//! authenticates nothing, so what Core guarantees is where the ID comes
//! from: only the account name of the person's verified Tokyo Tech (理工学系
//! Portal) identity, in the student-ID shape (`adpt_printer::StudentId`).
//! Nobody types an ID. Core also decides whether the printer can be
//! reached: one short TCP connect to its LPD port when the person opens or
//! clicks the setting, never in the background, and never ICMP (often
//! filtered, privileged on some systems, and blind to whether LPD answers).
//!
//! Shells do the platform part: on macOS and Linux they run the root
//! installer Core hands them (`CupsInstall`) behind the system's admin prompt; on
//! Windows Core sends the file itself (`Core::campus_print`).

use crate::school_auth::SchoolAuthProfile;
use crate::school_session::WebStop;
use crate::Core;
use adpt_printer::StudentId;
use serde::Serialize;
use std::net::SocketAddr;
use std::time::Duration;

#[cfg(windows)]
pub mod ipp;

#[cfg(windows)]
enum PrintDocument {
    File(std::path::PathBuf),
    Rendered { bytes: Vec<u8>, title: String },
}

/// The account whose name is the student ID.
const STUDENT_ACCOUNT: SchoolAuthProfile = SchoolAuthProfile::TokyoTech;
/// Long enough for a slow campus network, short enough to answer a
/// click.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Whose jobs the printer would bill, as the setting shows it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PrinterAccount {
    /// Verified now: jobs bill this student ID.
    #[serde(rename_all = "camelCase")]
    Ready { student_id: String },
    /// No Tokyo Tech account is saved.
    NotConnected,
    /// The verified account's name is not a student ID (a staff account).
    NotStudentId,
    /// The school refused the saved credentials or wants the person on its
    /// page: the account page says what to do.
    NeedsAttention,
    /// No verdict now: offline, the network, maintenance, a pause after a
    /// failure, or the account changing meanwhile.
    Unverified,
}

impl PrinterAccount {
    fn of(verified: Result<String, WebStop>) -> Self {
        match verified {
            Ok(name) => match StudentId::parse(name.trim()) {
                Some(student) => Self::Ready {
                    student_id: student.to_string(),
                },
                None => Self::NotStudentId,
            },
            Err(WebStop::NotConfigured) => Self::NotConnected,
            Err(WebStop::Rejected | WebStop::NeedsWeb) => Self::NeedsAttention,
            Err(
                WebStop::Unreachable | WebStop::Storage | WebStop::Stale | WebStop::Unsupported,
            ) => Self::Unverified,
        }
    }

    fn student(&self) -> Option<StudentId> {
        match self {
            Self::Ready { student_id } => StudentId::parse(student_id),
            _ => None,
        }
    }
}

/// Whether the printer answered just now.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PrinterReach {
    Reachable,
    /// Not on a network that reaches it (ScienceTokyo Wi-Fi does), or the
    /// printer is down.
    Unreachable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusPrinterStatus {
    pub account: PrinterAccount,
    pub reach: PrinterReach,
}

/// One TCP connect to `address` within `within`.
pub async fn probe_at(address: SocketAddr, within: Duration) -> PrinterReach {
    match tokio::time::timeout(within, tokio::net::TcpStream::connect(address)).await {
        Ok(Ok(_)) => PrinterReach::Reachable,
        Ok(Err(_)) | Err(_) => PrinterReach::Unreachable,
    }
}

/// The root step that installs the CUPS queue: run `installer`, written to
/// an executable file, as root with `args`.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub struct CupsInstall {
    pub installer: &'static [u8],
    pub args: [String; 2],
}

/// How a print from the Windows print window ended.
#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CampusPrintOutcome {
    /// Every copy reached the printer.
    Printed { copies: u32 },
    /// No verified student ID: nothing was sent.
    Account { account: PrinterAccount },
    /// Not PDF or PostScript: nothing was sent.
    NotPrintable,
    /// Copies outside 1..=`adpt_printer::document::MAX_COPIES`.
    InvalidCopies,
    /// The file could not be read: nothing was sent.
    Unreadable,
    /// No connection for the next copy: it and the later ones were not
    /// sent. `printed` copies before it were accepted.
    Unreachable { printed: u32 },
    /// The printer refused the next copy, or the connection failed before
    /// its data was all sent: it was not printed, nor the later ones.
    /// `printed` copies before it were accepted.
    NotPrinted { printed: u32 },
    /// The next copy's data all went out and the printer never answered:
    /// it may have printed, so the person checks the printer before sending
    /// it again. `printed` copies before it were accepted.
    Uncertain { printed: u32 },
}

#[cfg(windows)]
pub use adpt_printer::document::PrintOptions;

impl Core {
    /// What the printer setting shows when it opens or the person clicks
    /// it: the verified student ID and whether the printer answers, found
    /// together.
    pub async fn campus_printer_status(&self) -> CampusPrinterStatus {
        let (account, reach) = tokio::join!(
            self.campus_printer_account(),
            probe_at(adpt_printer::ADDRESS, PROBE_TIMEOUT)
        );
        log::info!("[campus-printer] status {account:?} {reach:?}");
        CampusPrinterStatus { account, reach }
    }

    /// Whose jobs the printer would bill now, verified for this call.
    pub async fn campus_printer_account(&self) -> PrinterAccount {
        PrinterAccount::of(self.sessions().verified_account_name(STUDENT_ACCOUNT).await)
    }

    /// The queue's root step for the verified student ID, printable only by
    /// `login` (the system account installing it, which the installer checks),
    /// or why there is no ID. Installing again updates the same queue.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub async fn campus_printer_install(
        &self,
        login: String,
    ) -> Result<CupsInstall, PrinterAccount> {
        let account = self.campus_printer_account().await;
        let student = account.student().ok_or(account)?;
        Ok(CupsInstall {
            installer: adpt_cups::INSTALLER,
            args: [student.to_string(), login],
        })
    }

    /// Sends the PDF or PostScript file at `path` to the printer, billed to
    /// the verified student ID, one upload per copy.
    #[cfg(windows)]
    pub async fn campus_print(
        &self,
        path: std::path::PathBuf,
        options: PrintOptions,
    ) -> CampusPrintOutcome {
        self.print_document(PrintDocument::File(path), options)
            .await
    }

    #[cfg(windows)]
    async fn print_document(
        &self,
        input: PrintDocument,
        options: PrintOptions,
    ) -> CampusPrintOutcome {
        use crate::runtime_tasks::RuntimeTaskError;
        use adpt_printer::{document, lpd};
        let account = self.campus_printer_account().await;
        let Some(student) = account.student() else {
            return CampusPrintOutcome::Account { account };
        };
        // Core owns the send: it runs to its end when the window closes
        // first (bounded by the LPD client's timeouts), and Core's shutdown,
        // which the app's exit waits for, waits for it too. It logs its own
        // result, which no window may be left to show.
        let sent = self
            .runtime
            .tasks()
            .run(async move {
                tokio::task::spawn_blocking(move || {
                    let sent = match input {
                        PrintDocument::File(path) => {
                            document::print(adpt_printer::ADDRESS, &student, &path, &options)
                        }
                        PrintDocument::Rendered { bytes, title } => document::print_bytes(
                            adpt_printer::ADDRESS,
                            &student,
                            &bytes,
                            &title,
                            &options,
                        ),
                    };
                    log::info!("[campus-printer] print {sent:?}");
                    sent
                })
                .await
            })
            .await;
        match sent {
            Ok(Ok(Ok(copies))) => CampusPrintOutcome::Printed { copies },
            Ok(Ok(Err(document::PrintError::NotPrintable))) => CampusPrintOutcome::NotPrintable,
            Ok(Ok(Err(document::PrintError::Copies))) => CampusPrintOutcome::InvalidCopies,
            Ok(Ok(Err(document::PrintError::File(_)))) => CampusPrintOutcome::Unreadable,
            Ok(Ok(Err(document::PrintError::Lpd {
                accepted: printed,
                error,
            }))) => match error {
                lpd::LpdError::Connect(_) => CampusPrintOutcome::Unreachable { printed },
                error if error.may_have_printed() => CampusPrintOutcome::Uncertain { printed },
                _ => CampusPrintOutcome::NotPrinted { printed },
            },
            // Core is shutting down: the send never started.
            Err(RuntimeTaskError::Stopped | RuntimeTaskError::Unavailable) => {
                CampusPrintOutcome::NotPrinted { printed: 0 }
            }
            // The send died midway; how far it got is unknown.
            Ok(Err(_)) | Err(RuntimeTaskError::Panicked | RuntimeTaskError::Interrupted) => {
                CampusPrintOutcome::Uncertain { printed: 0 }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/campus_printer.rs"]
mod tests;
