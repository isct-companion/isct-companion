//! Windows renders PDF; Core verifies billing and owns delivery. No document
//! files are retained. IPP usernames never select the billed student account.
//!
//! A job stays processing until LPD acknowledges every copy. The campus
//! printer does not report physical release or paper output.
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
use super::{CampusPrintOutcome, PrintOptions};
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
const MAX_QUEUED: usize = 1;

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
    delivery_reporter: Option<DeliveryReporter>,
    journal: Mutex<Option<rusqlite::Connection>>,
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
        let journal = open_journal(core.store.paths().local("campus-print-ipp.sqlite"));
        let (entries, next) = match journal.as_ref() {
            Ok(journal) => recover_jobs(journal).unwrap_or_else(|error| {
                log::error!("[campus-printer] could not recover IPP jobs: {error}");
                (BTreeMap::new(), 0)
            }),
            Err(error) => {
                log::error!("[campus-printer] could not open IPP journal: {error}");
                (BTreeMap::new(), 1)
            }
        };
        // Refuse new delivery when recovery was not possible: forgetting an
        // in-flight job can cause the system queue to repeat it.
        let journal = journal.ok().filter(|_| next != 0);
        Arc::new(Self {
            core,
            uri,
            printer_uuid,
            started: Instant::now(),
            jobs: Mutex::new(Jobs { next, entries }),
            queued: Arc::new(tokio::sync::Semaphore::new(MAX_QUEUED)),
            delivery_reporter,
            journal: Mutex::new(journal),
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
    fn record(&self, job: &Job, student: &str, state: &str) -> Result<(), PrintQueueError> {
        let journal = self
            .journal
            .lock()
            .map_err(|_| PrintQueueError::Unavailable)?;
        let journal = journal.as_ref().ok_or(PrintQueueError::Unavailable)?;
        journal.execute(
            "INSERT INTO jobs(id,title,student,state) VALUES (?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET student=excluded.student,state=excluded.state",
            rusqlite::params![job.id, job.title, student, state],
        ).map_err(|_| PrintQueueError::Unavailable)?;
        Ok(())
    }
    fn record_progress(&self, id: i32, accepted: u32) -> Result<(), PrintQueueError> {
        let journal = self
            .journal
            .lock()
            .map_err(|_| PrintQueueError::Unavailable)?;
        let journal = journal.as_ref().ok_or(PrintQueueError::Unavailable)?;
        journal
            .execute(
                "UPDATE jobs SET accepted=?2 WHERE id=?1",
                rusqlite::params![id, accepted],
            )
            .map_err(|_| PrintQueueError::Unavailable)?;
        Ok(())
    }
    fn lookup(&self, id: i32) -> Result<Option<Job>, PrintQueueError> {
        let job = self
            .jobs
            .lock()
            .map_err(|_| PrintQueueError::Unavailable)?
            .entries
            .get(&id)
            .cloned();
        if job.is_some() {
            return Ok(job);
        }
        let journal = self
            .journal
            .lock()
            .map_err(|_| PrintQueueError::Unavailable)?;
        let journal = journal.as_ref().ok_or(PrintQueueError::Unavailable)?;
        recover_job(journal, id).map_err(|_| PrintQueueError::Unavailable)
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
        // Persist allocation before exposing the ID, including Create-Job
        // requests which have not supplied a document yet.
        self.record(&job, "", "pending")?;
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
                            self.submit(&mut request, &mut response, job.id).await?;
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
                            self.submit(&mut request, &mut response, job.id).await?;
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
    async fn submit(
        self: &Arc<Self>,
        request: &mut IppRequestResponse,
        response: &mut IppRequestResponse,
        id: i32,
    ) -> Result<(), PrintQueueError> {
        if let Err(code) = document_format(request) {
            status(response, code);
            return Ok(());
        }
        // Reserve before copying the payload, so concurrent requests cannot
        // accumulate document buffers in the delivery queue.
        let slot = match self.queued.clone().try_acquire_owned() {
            Ok(slot) => slot,
            Err(_) => {
                status(response, StatusCode::ServerErrorBusy);
                return Ok(());
            }
        };
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
        let account = self.core.campus_printer_account().await;
        let Some(student) = account.student() else {
            self.update(
                id,
                JobState::Aborted,
                format!("Student account unavailable: {account:?}"),
            );
            if let Some(job) = self.lookup(id)? {
                if let Some(report) = &self.delivery_reporter {
                    report(job.title.clone(), CampusPrintOutcome::Account { account });
                }
                self.job_response(response, &job)?;
            }
            return Ok(());
        };
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
        if self.record(&job, student.as_str(), "processing").is_err() {
            self.update(
                id,
                JobState::Aborted,
                "Could not save delivery state".into(),
            );
            status(response, StatusCode::ServerErrorNotAcceptingJobs);
            return Ok(());
        }
        let queue = self.clone();
        let title = job.title.clone();
        let reported_title = job.title.clone();
        let delivery_reporter = self.delivery_reporter.clone();
        let student_for_record = student.clone();
        let refused_student = student.clone();
        let refused_job = job.clone();
        let accepted = self.core.runtime.tasks().spawn(async move {
            let _slot = slot;
            let progress_queue = queue.clone();
            let options = job.options;
            let sent = tokio::task::spawn_blocking(move || {
                adpt_printer::document::print_bytes_progress(
                    adpt_printer::ADDRESS,
                    &student,
                    &pdf,
                    &title,
                    &options,
                    |_| Ok(()),
                    |accepted| {
                        progress_queue
                            .record_progress(id, accepted)
                            .map_err(|_| adpt_printer::document::PrintError::Interrupted)
                    },
                )
            })
            .await;
            let outcome = super::map_send_result(sent);
            let state = if matches!(outcome, CampusPrintOutcome::Printed { .. }) {
                "completed"
            } else {
                "aborted"
            };
            // On a failed commit the processing record remains, so recovery
            // conservatively reports an unknown outcome rather than retrying.
            if let Err(error) = queue.record(&job, student_for_record.as_str(), state) {
                log::error!("[campus-printer] failed to save IPP outcome: {error}");
                queue.update(
                    id,
                    JobState::Aborted,
                    "Delivery outcome could not be saved; check the printer before retrying".into(),
                );
                if let Some(report) = delivery_reporter {
                    report(reported_title, CampusPrintOutcome::Uncertain { printed: 0 });
                }
                return;
            }
            if matches!(outcome, CampusPrintOutcome::Printed { .. }) {
                queue.update(
                    id,
                    JobState::Completed,
                    "Accepted by the campus printer; release it at the device".into(),
                );
                log::info!("[campus-printer] IPP job {id}: {outcome:?}");
            } else {
                queue.update(
                    id,
                    JobState::Aborted,
                    format!("Campus delivery did not complete: {outcome:?}"),
                );
                log::warn!("[campus-printer] IPP job {id}: {outcome:?}");
                if let Some(report) = delivery_reporter {
                    report(reported_title, outcome);
                }
            }
        });
        if !accepted {
            let _ = self.record(&refused_job, refused_student.as_str(), "aborted");
            self.update(id, JobState::Aborted, "Application is stopping".into());
            status(response, StatusCode::ServerErrorNotAcceptingJobs);
        } else if let Some(job) = self.lookup(id)? {
            self.job_response(response, &job)?;
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
                IppValue::Integer(
                    job.created
                        .saturating_duration_since(self.started)
                        .as_secs() as i32,
                ),
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
fn open_journal(path: std::path::PathBuf) -> rusqlite::Result<rusqlite::Connection> {
    let connection = rusqlite::Connection::open(path)?;
    connection.execute_batch("PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS jobs (id INTEGER PRIMARY KEY, title TEXT NOT NULL, student TEXT NOT NULL, state TEXT NOT NULL, accepted INTEGER NOT NULL DEFAULT 0)")?;
    Ok(connection)
}

fn decode_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<Job> {
    let id = row.get(0)?;
    let state: String = row.get(2)?;
    let accepted: u32 = row.get(3)?;
    let (state, message) = match state.as_str() {
        "completed" => (
            JobState::Completed,
            "Accepted by the campus printer; release it at the device",
        ),
        "pending" => (
            JobState::Aborted,
            "Document was not submitted before restart",
        ),
        "aborted" => (
            JobState::Aborted,
            "Campus delivery did not complete; check before retrying",
        ),
        _ => (
            JobState::Aborted,
            "Delivery outcome unknown after restart; check the printer before retrying",
        ),
    };
    Ok(Job {
        id,
        title: row.get(1)?,
        options: PrintOptions {
            copies: 1,
            settings: Settings::default(),
        },
        state,
        message: format!("{message} ({accepted} copies confirmed by LPD)"),
        created: Instant::now(),
    })
}

fn recover_job(connection: &rusqlite::Connection, id: i32) -> rusqlite::Result<Option<Job>> {
    use rusqlite::OptionalExtension;
    connection
        .query_row(
            "SELECT id,title,state,accepted FROM jobs WHERE id=?1",
            [id],
            decode_job,
        )
        .optional()
}

fn recover_jobs(connection: &rusqlite::Connection) -> rusqlite::Result<(BTreeMap<i32, Job>, i32)> {
    let max: i32 =
        connection.query_row("SELECT COALESCE(MAX(id),0) FROM jobs", [], |row| row.get(0))?;
    let next = max.checked_add(1).ok_or(rusqlite::Error::InvalidQuery)?;
    let mut statement = connection
        .prepare("SELECT id,title,state,accepted FROM jobs ORDER BY id DESC LIMIT 128")?;
    let mut entries = BTreeMap::new();
    for job in statement.query_map([], decode_job)? {
        let job = job?;
        entries.insert(job.id, job);
    }
    Ok((entries, next))
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
