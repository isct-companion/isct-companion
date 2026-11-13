//! The campus printer (lpd://172.20.83.212/kyocera): the desktops and Android.
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
//! Shells do the platform part: on macOS they run the root
//! installer Core hands them (`CupsInstall`) behind the system's admin prompt; on
//! Windows and Android every document reaches the printer through one call,
//! `Core::campus_print`: the file the Windows print window read, the
//! PDF Windows rendered for its system queue (`ipp`), and the PDF Android
//! rendered for the app's print service. Core also owns what the printer
//! offers (`CAPABILITIES`) and the words for how a print ended
//! (`CampusPrintOutcome::message`).
//!
//! The printer holds every job until the student releases it at the device,
//! and bills only then. A job delivered twice is one more held job that
//! costs nothing unless released, so a failed or uncertain send is simply
//! reported, and sending again is always the person's choice.
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use crate::school_auth::SchoolAuthProfile;
use crate::school_session::WebStop;
use crate::Core;
use adpt_printer::StudentId;
use serde::Serialize;
#[cfg(any(windows, target_os = "android"))]
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::Duration;

#[cfg(windows)]
pub mod ipp;

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
#[cfg(target_os = "macos")]
pub struct CupsInstall {
    pub installer: &'static [u8],
    pub args: [String; 2],
}

/// How a print ended, from the Windows print window or system queue, or
/// from Android's print service.
#[cfg(any(windows, target_os = "android"))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CampusPrintOutcome {
    /// LPD acknowledged every copy; physical printing is not reported.
    Printed { copies: u32 },
    /// No verified student ID: nothing was sent.
    Account { account: PrinterAccount },
    /// Not PDF or PostScript: nothing was sent.
    NotPrintable,
    /// Copies outside 1..=`CAPABILITIES.max_copies`: nothing was sent.
    InvalidCopies,
    /// Larger than `CAPABILITIES.max_document_bytes`: nothing was sent.
    TooLarge,
    /// The shell could not read the file: nothing was sent.
    Unreadable,
    /// No connection for the next copy: it and the later ones were not
    /// sent. `printed` copies before it were accepted.
    Unreachable { printed: u32 },
    /// The printer refused the next copy, or the connection failed before
    /// its data was all sent: it was not printed, nor the later ones.
    /// `printed` copies before it were accepted.
    NotPrinted { printed: u32 },
    /// The next copy's data all went out and the printer never answered:
    /// it may be held at the printer. `printed` copies before it were
    /// accepted.
    Uncertain { printed: u32 },
}

/// Words for the person: a key in the shared catalogs
/// (`resources/locales`) and the values its `{{name}}` placeholders take.
/// Every shell shows a print's end through this one mapping.
#[cfg(any(windows, target_os = "android"))]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CampusPrintMessage {
    pub key: &'static str,
    pub args: BTreeMap<&'static str, u32>,
}

#[cfg(any(windows, target_os = "android"))]
impl CampusPrintMessage {
    fn new(key: &'static str) -> Self {
        Self {
            key,
            args: BTreeMap::new(),
        }
    }
    fn with(key: &'static str, name: &'static str, value: u32) -> Self {
        Self {
            key,
            args: BTreeMap::from([(name, value)]),
        }
    }
    /// One key when nothing was accepted, the `after` key naming the
    /// accepted copies otherwise.
    fn after(none: &'static str, after: &'static str, printed: u32) -> Self {
        if printed == 0 {
            Self::new(none)
        } else {
            Self::with(after, "printed", printed)
        }
    }
}

/// How a print ended as the print window shows it.
#[cfg(any(windows, target_os = "android"))]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CampusPrintReport {
    pub printed: bool,
    pub message: CampusPrintMessage,
}

#[cfg(any(windows, target_os = "android"))]
impl CampusPrintOutcome {
    pub fn printed(&self) -> bool {
        matches!(self, Self::Printed { .. })
    }

    pub fn message(&self) -> CampusPrintMessage {
        type M = CampusPrintMessage;
        match self {
            Self::Printed { copies } => M::with("campusPrint.window.sent", "copies", *copies),
            Self::Account { account } => M::new(match account {
                PrinterAccount::NotConnected => "tools.campusPrinter.saveTheTokyoTechAccount",
                PrinterAccount::NotStudentId => "tools.campusPrinter.accountIsNotAStudentId",
                PrinterAccount::NeedsAttention => "tools.campusPrinter.accountNeedsAttention",
                PrinterAccount::Unverified | PrinterAccount::Ready { .. } => {
                    "tools.campusPrinter.accountNotVerified"
                }
            }),
            Self::NotPrintable => M::new("campusPrint.window.notAPdf"),
            Self::InvalidCopies => M::with(
                "campusPrint.window.copiesRange",
                "max",
                CAPABILITIES.max_copies,
            ),
            Self::TooLarge => M::with(
                "campusPrint.notice.tooLarge",
                "megabytes",
                (CAPABILITIES.max_document_bytes / (1024 * 1024)) as u32,
            ),
            Self::Unreadable => M::new("campusPrint.window.couldNotReadTheFile"),
            Self::Unreachable { printed } => M::after(
                "tools.campusPrinter.cannotReachThePrinter",
                "campusPrint.window.unreachableAfter",
                *printed,
            ),
            Self::NotPrinted { printed } => M::after(
                "campusPrint.window.notPrinted",
                "campusPrint.window.notPrintedAfter",
                *printed,
            ),
            Self::Uncertain { printed } => M::after(
                "campusPrint.window.uncertain",
                "campusPrint.window.uncertainAfter",
                *printed,
            ),
        }
    }

    pub fn report(&self) -> CampusPrintReport {
        CampusPrintReport {
            printed: self.printed(),
            message: self.message(),
        }
    }
}

#[cfg(any(windows, target_os = "android"))]
pub use adpt_printer::{
    document::PrintOptions,
    pjl::{ColorMode, Settings, Sides},
};

/// A sheet the printer takes.
#[cfg(any(windows, target_os = "android"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Media {
    A4,
    Letter,
}

#[cfg(any(windows, target_os = "android"))]
impl Media {
    /// Its IPP `media` keyword (PWG 5101.1).
    pub fn keyword(self) -> &'static str {
        match self {
            Self::A4 => "iso_a4_210x297mm",
            Self::Letter => "na_letter_8.5x11in",
        }
    }
    /// Width and height in hundredths of a millimetre.
    pub fn size(self) -> (i32, i32) {
        match self {
            Self::A4 => (21000, 29700),
            Self::Letter => (21590, 27940),
        }
    }
}

/// What the campus printer offers, the one set the Windows queue's IPP
/// attributes, Android's printer capabilities and the Windows print window
/// are built from.
#[cfg(any(windows, target_os = "android"))]
pub struct Capabilities {
    /// The paper it takes; the first is the default.
    pub media: [Media; 2],
    /// What a job gets when the dialog does not say.
    pub defaults: Settings,
    pub max_copies: u32,
    /// The largest document one print takes. It is held in memory until its
    /// last copy is sent, and shells stop reading at this size.
    pub max_document_bytes: usize,
    /// The unprintable edge on every side, in hundredths of a millimetre
    /// (4.23 mm, 1/6 inch).
    pub margin: i32,
}

#[cfg(any(windows, target_os = "android"))]
pub const CAPABILITIES: Capabilities = Capabilities {
    media: [Media::A4, Media::Letter],
    defaults: Settings {
        sides: Sides::LongEdge,
        color: ColorMode::Mono,
    },
    max_copies: adpt_printer::document::MAX_COPIES,
    max_document_bytes: 64 * 1024 * 1024,
    margin: 423,
};

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
    #[cfg(target_os = "macos")]
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

    /// Sends `document` to the printer, billed to the verified student ID,
    /// one upload per copy: the one way a document reaches the printer on
    /// Windows and Android. Sends run one at a time, in the order they
    /// arrive, whichever window, queue or service they came from.
    #[cfg(any(windows, target_os = "android"))]
    pub async fn campus_print(
        &self,
        document: bytes::Bytes,
        title: String,
        options: PrintOptions,
    ) -> CampusPrintOutcome {
        let outcome = self.send_campus_print(document, title, options).await;
        log::info!("[campus-printer] print {outcome:?}");
        outcome
    }

    #[cfg(any(windows, target_os = "android"))]
    async fn send_campus_print(
        &self,
        document: bytes::Bytes,
        title: String,
        options: PrintOptions,
    ) -> CampusPrintOutcome {
        use crate::runtime_tasks::RuntimeTaskError;
        if document.len() > CAPABILITIES.max_document_bytes {
            return CampusPrintOutcome::TooLarge;
        }
        let account = self.campus_printer_account().await;
        let Some(student) = account.student() else {
            return CampusPrintOutcome::Account { account };
        };
        let send = send_in_turn(
            self.campus_print_turn.clone(),
            adpt_printer::ADDRESS,
            student,
            document,
            title,
            options,
        );
        match self.runtime.tasks().run(send).await {
            Ok(outcome) => outcome,
            Err(RuntimeTaskError::Stopped | RuntimeTaskError::Unavailable) => {
                CampusPrintOutcome::NotPrinted { printed: 0 }
            }
            Err(RuntimeTaskError::Panicked | RuntimeTaskError::Interrupted) => {
                CampusPrintOutcome::Uncertain { printed: 0 }
            }
        }
    }
}

/// Sends once every earlier send holding `turn` has ended.
#[cfg(any(windows, target_os = "android"))]
async fn send_in_turn(
    turn: std::sync::Arc<tokio::sync::Mutex<()>>,
    server: SocketAddr,
    student: StudentId,
    document: bytes::Bytes,
    title: String,
    options: PrintOptions,
) -> CampusPrintOutcome {
    let _turn = turn.lock_owned().await;
    let sent = tokio::task::spawn_blocking(move || {
        adpt_printer::document::print_bytes(server, &student, &document, &title, &options)
    })
    .await;
    map_send_result(sent)
}

#[cfg(any(windows, target_os = "android"))]
fn map_send_result(
    sent: Result<Result<u32, adpt_printer::document::PrintError>, tokio::task::JoinError>,
) -> CampusPrintOutcome {
    use adpt_printer::{document::PrintError, lpd};
    match sent {
        Ok(Ok(copies)) => CampusPrintOutcome::Printed { copies },
        Ok(Err(PrintError::NotPrintable)) => CampusPrintOutcome::NotPrintable,
        Ok(Err(PrintError::Copies)) => CampusPrintOutcome::InvalidCopies,
        Ok(Err(PrintError::Lpd {
            accepted: printed,
            error,
        })) => match error {
            lpd::LpdError::Connect(_) => CampusPrintOutcome::Unreachable { printed },
            error if error.may_have_printed() => CampusPrintOutcome::Uncertain { printed },
            _ => CampusPrintOutcome::NotPrinted { printed },
        },
        Err(_) => CampusPrintOutcome::Uncertain { printed: 0 },
    }
}

#[cfg(test)]
#[path = "tests/campus_printer.rs"]
mod tests;
