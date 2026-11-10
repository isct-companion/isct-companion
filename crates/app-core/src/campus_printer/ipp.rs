//! Windows renders PDF; Core verifies billing and owns delivery. No document
//! files are retained. IPP usernames never select the billed student account.
//!
//! The campus printer holds every job until the student releases it at the
//! device and never reports back, so there is nothing for the Windows queue
//! to track: a received document is answered as completed at once, Windows
//! drops it from its list, and delivery continues here in order. A delivery
//! that fails is reported to the shell, which shows a system notification.
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
use super::{CampusPrintOutcome, PrintDocument, PrintOptions};
use crate::Core;
use ::ipp::{parser::IppParser, prelude::*};
use adpt_printer::{
    document::MAX_COPIES,
    pjl::{ColorMode, Duplex, Language, Settings},
};
use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
mod attributes;
pub const MAX_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;
/// Documents received and waiting for the printer. Each holds its PDF in
/// memory, so the bound is also the memory bound.
const MAX_QUEUED: usize = 8;

#[derive(Debug, thiserror::Error)]
pub enum PrintQueueError {
    #[error("Invalid IPP message: {0}")]
    Protocol(#[from] ::ipp::parser::IppParseError),
    #[error("Could not read the print document: {0}")]
    Document(#[from] std::io::Error),
    #[error("The print queue is unavailable.")]
    Unavailable,
}
/// A job's IPP `job-state` (RFC 8011 §5.3.7), limited to the states this
/// queue uses: a job waits for its document, is claimed for delivery, then
/// ends handed over, refused or cancelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JobState {
    /// Created; waiting for its document.
    Pending,
    /// The document was claimed for delivery; a duplicate send is refused.
    Processing,
    Canceled,
    Aborted,
    /// Handed to Core's ordered delivery. The campus printer holds jobs until
    /// release and never reports back, so this is the end as far as IPP goes.
    Completed,
}

impl JobState {
    fn code(self) -> i32 {
        match self {
            Self::Pending => 3,
            Self::Processing => 5,
            Self::Canceled => 7,
            Self::Aborted => 8,
            Self::Completed => 9,
        }
    }
    fn reason(self) -> &'static str {
        match self {
            Self::Pending => "job-incoming",
            Self::Processing => "job-printing",
            Self::Canceled => "job-canceled-by-user",
            Self::Aborted => "aborted-by-system",
            Self::Completed => "job-completed-successfully",
        }
    }
    fn is_terminal(self) -> bool {
        matches!(self, Self::Canceled | Self::Aborted | Self::Completed)
    }
}

#[derive(Clone)]
struct Job {
    id: i32,
    title: String,
    options: PrintOptions,
    state: JobState,
    message: String,
    created: Instant,
}
struct Jobs {
    next: i32,
    entries: BTreeMap<i32, Job>,
}
/// Called with the document's title and the outcome when a delivery does
/// not end `Printed`.
pub type DeliveryReporter = Arc<dyn Fn(String, CampusPrintOutcome) + Send + Sync>;
pub struct CampusPrintQueue {
    core: Arc<Core>,
    uri: String,
    printer_uuid: String,
    started: Instant,
    jobs: Mutex<Jobs>,
    /// Slots for received documents not yet delivered.
    queued: Arc<tokio::sync::Semaphore>,
    /// One delivery at a time, in the order the documents arrived.
    delivery: Arc<tokio::sync::Semaphore>,
    delivery_reporter: Option<DeliveryReporter>,
}
impl CampusPrintQueue {
    pub fn new(core: Arc<Core>, uri: String, printer_uuid: String) -> Arc<Self> {
        Self::new_with_reporter(core, uri, printer_uuid, None)
    }
    pub fn new_with_delivery_reporter(
        core: Arc<Core>,
        uri: String,
        printer_uuid: String,
        reporter: DeliveryReporter,
    ) -> Arc<Self> {
        Self::new_with_reporter(core, uri, printer_uuid, Some(reporter))
    }
    fn new_with_reporter(
        core: Arc<Core>,
        uri: String,
        printer_uuid: String,
        delivery_reporter: Option<DeliveryReporter>,
    ) -> Arc<Self> {
        Arc::new(Self {
            core,
            uri,
            printer_uuid,
            started: Instant::now(),
            jobs: Mutex::new(Jobs {
                next: (uuid::Uuid::new_v4().as_u128() % 1_000_000_000) as i32 + 1,
                entries: BTreeMap::new(),
            }),
            queued: Arc::new(tokio::sync::Semaphore::new(MAX_QUEUED)),
            delivery: Arc::new(tokio::sync::Semaphore::new(1)),
            delivery_reporter,
        })
    }
    fn update(&self, id: i32, state: JobState, message: String) {
        if let Ok(mut jobs) = self.jobs.lock() {
            if let Some(job) = jobs.entries.get_mut(&id) {
                job.state = state;
                job.message = message;
            }
        }
    }
    fn lookup(&self, id: i32) -> Result<Option<Job>, PrintQueueError> {
        Ok(self
            .jobs
            .lock()
            .map_err(|_| PrintQueueError::Unavailable)?
            .entries
            .get(&id)
            .cloned())
    }
    fn create(
        &self,
        request: &IppRequestResponse,
        options: PrintOptions,
    ) -> Result<Option<Job>, PrintQueueError> {
        let mut jobs = self.jobs.lock().map_err(|_| PrintQueueError::Unavailable)?;
        jobs.entries.retain(|_, job| {
            job.state == JobState::Processing || job.created.elapsed() < Duration::from_secs(600)
        });
        if jobs.entries.len() >= 128 {
            let terminal = jobs
                .entries
                .iter()
                .find(|(_, job)| job.state.is_terminal())
                .map(|(id, _)| *id);
            if let Some(id) = terminal {
                jobs.entries.remove(&id);
            } else {
                return Ok(None);
            }
        }
        let id = jobs.next;
        jobs.next = jobs
            .next
            .checked_add(1)
            .ok_or(PrintQueueError::Unavailable)?;
        let job = Job {
            id,
            title: value(request, "job-name")
                .map(ToString::to_string)
                .unwrap_or_else(|| "Campus print".into()),
            options,
            state: JobState::Pending,
            message: "Waiting for document".into(),
            created: Instant::now(),
        };
        jobs.entries.insert(id, job.clone());
        Ok(Some(job))
    }
    /// The IPP library retains a binary payload reader, avoiding text conversion.
    pub async fn handle(self: &Arc<Self>, bytes: bytes::Bytes) -> Result<Vec<u8>, PrintQueueError> {
        let mut request = IppParser::new(Cursor::new(bytes)).parse()?;
        let mut response = IppRequestResponse::new_response(
            request.header().version,
            StatusCode::SuccessfulOk,
            request.header().request_id,
        )?;
        if !matches!(request.header().version.0, 0x0100 | 0x0101 | 0x0200) {
            status(&mut response, StatusCode::ServerErrorVersionNotSupported);
            return Ok(response.to_bytes().to_vec());
        }
        let operation = request.header().operation_or_status;
        match operation {
            11 => attributes::printer(
                &mut response,
                &self.uri,
                &self.printer_uuid,
                self.started.elapsed().as_secs() as i32,
            )?,
            4 => {
                if let Err(error) = options(&request) {
                    status(&mut response, error);
                }
            }
            2 | 5 => match options(&request) {
                Err(error) => status(&mut response, error),
                Ok(options) => {
                    if self.core.runtime.tasks().is_closed() {
                        status(&mut response, StatusCode::ServerErrorNotAcceptingJobs);
                    } else if let Some(job) = self.create(&request, options)? {
                        if operation == 2 {
                            self.submit(&mut request, &mut response, job.id)?;
                        } else {
                            self.job_response(&mut response, &job)?;
                        }
                    } else {
                        status(&mut response, StatusCode::ServerErrorBusy);
                    }
                }
            },
            6 | 8 | 9 => {
                let id = value(&request, "job-id")
                    .and_then(|v| match v {
                        IppValue::Integer(id) => Some(*id),
                        _ => None,
                    })
                    .or_else(|| {
                        value(&request, "job-uri").and_then(|v| {
                            v.to_string()
                                .rsplit('/')
                                .next()
                                .and_then(|s| s.parse().ok())
                        })
                    });
                match id.map(|id| self.lookup(id)).transpose()?.flatten() {
                    None => status(&mut response, StatusCode::ClientErrorNotFound),
                    Some(job) if operation == 9 => self.job_response(&mut response, &job)?,
                    Some(job) if job.state != JobState::Pending => {
                        status(&mut response, StatusCode::ClientErrorNotPossible)
                    }
                    Some(job) if operation == 8 => {
                        let mut jobs =
                            self.jobs.lock().map_err(|_| PrintQueueError::Unavailable)?;
                        if let Some(current) = jobs.entries.get_mut(&job.id) {
                            if current.state == JobState::Pending {
                                current.state = JobState::Canceled;
                                current.message = "Canceled before submission".into();
                            } else {
                                status(&mut response, StatusCode::ClientErrorNotPossible);
                            }
                        }
                    }
                    Some(job) => {
                        if value(&request, "last-document") != Some(&IppValue::Boolean(true)) {
                            status(&mut response, StatusCode::ClientErrorNotPossible);
                        } else {
                            self.submit(&mut request, &mut response, job.id)?;
                        }
                    }
                }
            }
            10 => {
                let which = value(&request, "which-jobs")
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "not-completed".into());
                let jobs: Vec<_> = self
                    .jobs
                    .lock()
                    .map_err(|_| PrintQueueError::Unavailable)?
                    .entries
                    .values()
                    .cloned()
                    .collect();
                for job in jobs {
                    if which == "all"
                        || (which == "completed" && job.state.is_terminal())
                        || (which == "not-completed" && !job.state.is_terminal())
                    {
                        self.job_response(&mut response, &job)?;
                    }
                }
            }
            _ => status(&mut response, StatusCode::ServerErrorOperationNotSupported),
        }
        // Job operations, and any refusal, say what Windows asked and got;
        // the printer and job polls that succeed are not logged.
        let answer = response.header().operation_or_status;
        if answer != StatusCode::SuccessfulOk as i16 || !matches!(operation, 9..=11) {
            log::info!(
                "[campus-printer] IPP operation 0x{operation:04x} answered 0x{answer:04x} (format {:?}, sides {:?}, color {:?}, copies {:?})",
                value(&request, "document-format").map(ToString::to_string),
                value(&request, "sides").map(ToString::to_string),
                value(&request, "print-color-mode").map(ToString::to_string),
                value(&request, "copies").map(ToString::to_string),
            );
        }
        Ok(response.to_bytes().to_vec())
    }
    fn submit(
        self: &Arc<Self>,
        request: &mut IppRequestResponse,
        response: &mut IppRequestResponse,
        id: i32,
    ) -> Result<(), PrintQueueError> {
        if let Err(code) = document_format(request) {
            status(response, code);
            return Ok(());
        }
        let mut pdf = Vec::new();
        request
            .payload_mut()
            .take(MAX_DOCUMENT_BYTES as u64 + 1)
            .read_to_end(&mut pdf)?;
        if pdf.len() > MAX_DOCUMENT_BYTES || Language::detect(&pdf) != Some(Language::Pdf) {
            status(response, StatusCode::ClientErrorDocumentFormatError);
            self.update(id, JobState::Aborted, "Invalid PDF document".into());
            return Ok(());
        }
        // Only a full backlog refuses; otherwise the document waits its turn
        // here rather than failing in the Windows queue.
        let slot = match self.queued.clone().try_acquire_owned() {
            Ok(slot) => slot,
            Err(_) => {
                status(response, StatusCode::ServerErrorBusy);
                return Ok(());
            }
        };
        let delivery = self.delivery.clone();
        let job = {
            let mut jobs = self.jobs.lock().map_err(|_| PrintQueueError::Unavailable)?;
            let Some(job) = jobs.entries.get_mut(&id) else {
                status(response, StatusCode::ClientErrorNotFound);
                return Ok(());
            };
            if job.state != JobState::Pending {
                status(response, StatusCode::ClientErrorNotPossible);
                return Ok(());
            }
            // Claim before any await: a duplicate Send-Document never resends.
            job.state = JobState::Processing;
            job.message = "Sending to campus printer".into();
            job.clone()
        };
        let core = self.core.clone();
        let title = job.title.clone();
        let reported_title = job.title.clone();
        let delivery_reporter = self.delivery_reporter.clone();
        let accepted = self.core.runtime.tasks().spawn(async move {
            let _slot = slot;
            // The semaphore is never closed; holding the result holds the turn.
            let _turn = delivery.acquire_owned().await;
            let outcome = core
                .print_document(PrintDocument::Rendered { bytes: pdf, title }, job.options)
                .await;
            if matches!(outcome, CampusPrintOutcome::Printed { .. }) {
                log::info!("[campus-printer] IPP job {id}: {outcome:?}");
            } else {
                log::warn!("[campus-printer] IPP job {id}: {outcome:?}");
                if let Some(report) = delivery_reporter {
                    report(reported_title, outcome);
                }
            }
        });
        if !accepted {
            self.update(id, JobState::Aborted, "Application is stopping".into());
            status(response, StatusCode::ServerErrorNotAcceptingJobs);
        } else {
            // Windows can finish its spool job after Core accepts the handoff;
            // LPD delivery continues in the tracked background task.
            self.update(
                id,
                JobState::Completed,
                "Accepted by IsctCompanion; campus delivery continues in background".into(),
            );
            if let Some(job) = self.lookup(id)? {
                self.job_response(response, &job)?;
            }
        }
        Ok(())
    }
    fn job_response(
        &self,
        response: &mut IppRequestResponse,
        job: &Job,
    ) -> Result<(), PrintQueueError> {
        let mut group = IppAttributeGroup::new(DelimiterTag::JobAttributes);
        for (name, val) in [
            ("job-id", IppValue::Integer(job.id)),
            (
                "job-uri",
                IppValue::new_uri(format!("{}/jobs/{}", self.uri, job.id))?,
            ),
            ("job-printer-uri", IppValue::new_uri(self.uri.clone())?),
            ("job-state", IppValue::Enum(job.state.code())),
            (
                "job-state-message",
                IppValue::new_text_without_language(job.message.clone())?,
            ),
            (
                "job-state-reasons",
                IppValue::new_keyword(job.state.reason())?,
            ),
            (
                "job-printer-up-time",
                IppValue::Integer(self.started.elapsed().as_secs() as i32),
            ),
            (
                "time-at-creation",
                IppValue::Integer(job.created.duration_since(self.started).as_secs() as i32),
            ),
        ] {
            group
                .attributes_mut()
                .push(IppAttribute::with_name(name, val)?);
        }
        response.attributes_mut().groups_mut().push(group);
        Ok(())
    }
}
fn value<'a>(request: &'a IppRequestResponse, name: &str) -> Option<&'a IppValue> {
    request
        .attributes()
        .groups()
        .iter()
        .find_map(|group| group.get(name))
        .map(IppAttribute::value)
}
fn status(response: &mut IppRequestResponse, code: StatusCode) {
    response.header_mut().operation_or_status = code as i16;
}
/// Windows names the rendered PDF `application/pdf`; some clients send
/// `application/octet-stream`. The bytes are checked for a PDF either way.
fn document_format(request: &IppRequestResponse) -> Result<(), StatusCode> {
    if value(request, "document-format").is_some_and(|v| {
        !matches!(
            v.to_string().as_str(),
            "application/pdf" | "application/octet-stream"
        )
    }) {
        return Err(StatusCode::ClientErrorDocumentFormatNotSupported);
    }
    if value(request, "compression").is_some_and(|v| v.to_string() != "none") {
        return Err(StatusCode::ClientErrorCompressionNotSupported);
    }
    Ok(())
}
/// Job settings never fail a job: a value this printer does not offer falls
/// back to the queue default (long-edge duplex, monochrome), and copies are
/// clamped to what the LPD transport repeats. The transport always sends
/// whole documents, so copies are collated whatever
/// `multiple-document-handling` asks.
fn options(request: &IppRequestResponse) -> Result<PrintOptions, StatusCode> {
    document_format(request)?;
    let copies = match value(request, "copies") {
        Some(IppValue::Integer(n)) => (*n).clamp(1, MAX_COPIES as i32) as u32,
        _ => 1,
    };
    let duplex = match value(request, "sides").map(ToString::to_string).as_deref() {
        Some("one-sided") => Duplex::Off,
        Some("two-sided-short-edge") => Duplex::ShortEdge,
        _ => Duplex::LongEdge,
    };
    let color = match value(request, "print-color-mode")
        .map(ToString::to_string)
        .as_deref()
    {
        Some("color") => ColorMode::Color,
        _ => ColorMode::Mono,
    };
    Ok(PrintOptions {
        copies,
        settings: Settings {
            duplex: Some(duplex),
            color: Some(color),
        },
    })
}

#[cfg(test)]
#[path = "../tests/campus_printer_ipp.rs"]
mod tests;
