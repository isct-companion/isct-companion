//! The Windows system queue's IPP receiver. Windows renders a job to PDF and
//! posts it here; Core sends it with `Core::campus_print` and answers
//! Print-Job or Send-Document once the send has ended: `completed` when the
//! printer took every copy, `aborted` with Core's words for why otherwise.
//! IPP usernames never select the billed student account.
//!
//! Jobs live in memory only, bounded, so Windows can poll them during this
//! run. Nothing is retained or resent: the printer bills at release, so a
//! job delivered twice costs nothing (`super`).
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
use super::{CampusPrintMessage, ColorMode, PrintOptions, Settings, Sides, CAPABILITIES};
use crate::Core;
use ::ipp::{parser::IppParser, prelude::*};
use bytes::Bytes;
use std::{
    collections::BTreeMap,
    io::Cursor,
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
mod attributes;

/// Jobs kept for Windows' polls; the oldest finished one makes room.
const MAX_JOBS: usize = 128;

#[derive(Debug, thiserror::Error)]
pub enum PrintQueueError {
    #[error("Invalid IPP message: {0}")]
    Protocol(#[from] ::ipp::parser::IppParseError),
    #[error("The print queue is unavailable.")]
    Unavailable,
}

/// A job's IPP `job-state` (RFC 8011 §5.3.7), limited to the states this
/// queue uses: a Create-Job waits for its document, a document is being
/// sent, then the job ends sent, failed or cancelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JobState {
    Pending,
    Processing,
    Canceled,
    Aborted,
    /// Every copy reached the printer, which holds it until release and
    /// never reports back, so this is the end as far as IPP goes.
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

/// Words a message in the person's language (the shell owns the catalogs).
pub type Wording = Box<dyn Fn(&CampusPrintMessage) -> String + Send + Sync>;

/// A job that ended without printing, for the shell to tell the person.
#[derive(Debug, PartialEq, Eq)]
pub struct Failure {
    pub title: String,
    pub text: String,
}

pub struct IppReply {
    pub body: Vec<u8>,
    pub failed: Option<Failure>,
}

/// One parsed request; `document` shares the body's buffer.
struct Request {
    header: IppHeader,
    attributes: IppAttributes,
    document: Bytes,
}

pub struct CampusPrintQueue {
    core: Arc<Core>,
    uri: String,
    printer_uuid: String,
    started: Instant,
    wording: Wording,
    jobs: Mutex<Jobs>,
}

impl CampusPrintQueue {
    pub fn new(core: Arc<Core>, uri: String, printer_uuid: String, wording: Wording) -> Self {
        Self {
            core,
            uri,
            printer_uuid,
            started: Instant::now(),
            wording,
            jobs: Mutex::new(Jobs {
                next: first_job_id(),
                entries: BTreeMap::new(),
            }),
        }
    }

    fn jobs(&self) -> Result<std::sync::MutexGuard<'_, Jobs>, PrintQueueError> {
        self.jobs.lock().map_err(|_| PrintQueueError::Unavailable)
    }

    fn lookup(&self, id: i32) -> Result<Option<Job>, PrintQueueError> {
        Ok(self.jobs()?.entries.get(&id).cloned())
    }

    fn update(&self, id: i32, state: JobState, message: String) -> Result<(), PrintQueueError> {
        if let Some(job) = self.jobs()?.entries.get_mut(&id) {
            job.state = state;
            job.message = message;
        }
        Ok(())
    }

    /// A new pending job, or `None` when the table is full of unfinished ones.
    fn create(
        &self,
        request: &Request,
        options: PrintOptions,
    ) -> Result<Option<Job>, PrintQueueError> {
        let mut jobs = self.jobs()?;
        if jobs.entries.len() >= MAX_JOBS {
            let finished = jobs
                .entries
                .iter()
                .find(|(_, job)| job.state.is_terminal())
                .map(|(id, _)| *id);
            match finished {
                Some(id) => jobs.entries.remove(&id),
                None => return Ok(None),
            };
        }
        let id = jobs.next;
        jobs.next = id.checked_add(1).ok_or(PrintQueueError::Unavailable)?;
        let job = Job {
            id,
            title: value(&request.attributes, "job-name")
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

    pub async fn handle(&self, body: Bytes) -> Result<IppReply, PrintQueueError> {
        let request = parse(body)?;
        let mut response = IppRequestResponse::new_response(
            request.header.version,
            StatusCode::SuccessfulOk,
            request.header.request_id,
        )?;
        let mut failed = None;
        if !matches!(request.header.version.0, 0x0100 | 0x0101 | 0x0200) {
            status(&mut response, StatusCode::ServerErrorVersionNotSupported);
            return Ok(IppReply {
                body: response.to_bytes().to_vec(),
                failed,
            });
        }
        let operation = request.header.operation_or_status;
        match operation {
            11 => attributes::printer(
                &mut response,
                &self.uri,
                &self.printer_uuid,
                self.started.elapsed().as_secs() as i32,
            )?,
            4 => {
                if let Err(error) = options(&request.attributes) {
                    status(&mut response, error);
                }
            }
            2 | 5 => match options(&request.attributes) {
                Err(error) => status(&mut response, error),
                Ok(options) => {
                    if self.core.runtime.tasks().is_closed() {
                        status(&mut response, StatusCode::ServerErrorNotAcceptingJobs);
                    } else if let Some(job) = self.create(&request, options)? {
                        if operation == 2 {
                            failed = self.submit(&request, &mut response, job.id).await?;
                        } else {
                            self.job_response(&mut response, &job)?;
                        }
                    } else {
                        status(&mut response, StatusCode::ServerErrorBusy);
                    }
                }
            },
            6 | 8 | 9 => {
                let id = value(&request.attributes, "job-id")
                    .and_then(|v| match v {
                        IppValue::Integer(id) => Some(*id),
                        _ => None,
                    })
                    .or_else(|| {
                        value(&request.attributes, "job-uri").and_then(|v| {
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
                        let mut jobs = self.jobs()?;
                        match jobs.entries.get_mut(&job.id) {
                            Some(current) if current.state == JobState::Pending => {
                                current.state = JobState::Canceled;
                                current.message = "Canceled before submission".into();
                            }
                            _ => status(&mut response, StatusCode::ClientErrorNotPossible),
                        }
                    }
                    Some(job) => {
                        if value(&request.attributes, "last-document")
                            != Some(&IppValue::Boolean(true))
                        {
                            status(&mut response, StatusCode::ClientErrorNotPossible);
                        } else {
                            failed = self.submit(&request, &mut response, job.id).await?;
                        }
                    }
                }
            }
            10 => {
                let which = value(&request.attributes, "which-jobs")
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "not-completed".into());
                let jobs: Vec<_> = self.jobs()?.entries.values().cloned().collect();
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
            let attribute = |name| value(&request.attributes, name).map(ToString::to_string);
            log::info!(
                "[campus-printer] IPP operation 0x{operation:04x} answered 0x{answer:04x} (format {:?}, sides {:?}, color {:?}, copies {:?})",
                attribute("document-format"),
                attribute("sides"),
                attribute("print-color-mode"),
                attribute("copies"),
            );
        }
        Ok(IppReply {
            body: response.to_bytes().to_vec(),
            failed,
        })
    }

    /// Sends the request's document for pending job `id` and answers with
    /// the job as the send left it. Returns the failure to tell the person.
    async fn submit(
        &self,
        request: &Request,
        response: &mut IppRequestResponse,
        id: i32,
    ) -> Result<Option<Failure>, PrintQueueError> {
        if let Err(code) = document_format(&request.attributes) {
            status(response, code);
            return Ok(None);
        }
        let (title, options) = {
            let mut jobs = self.jobs()?;
            let Some(job) = jobs.entries.get_mut(&id) else {
                status(response, StatusCode::ClientErrorNotFound);
                return Ok(None);
            };
            if job.state != JobState::Pending {
                status(response, StatusCode::ClientErrorNotPossible);
                return Ok(None);
            }
            // Claimed before the send: a second Send-Document is refused.
            job.state = JobState::Processing;
            job.message = "Sending to the campus printer".into();
            (job.title.clone(), job.options)
        };
        let outcome = self
            .core
            .campus_print(request.document.clone(), title.clone(), options)
            .await;
        let text = (self.wording)(&outcome.message());
        let printed = outcome.printed();
        let state = if printed {
            JobState::Completed
        } else {
            JobState::Aborted
        };
        self.update(id, state, text.clone())?;
        if let Some(job) = self.lookup(id)? {
            self.job_response(response, &job)?;
        }
        Ok((!printed).then_some(Failure { title, text }))
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

/// Job IDs start from the clock, so a job Windows still lists from an
/// earlier run never shares an ID with one of this run's.
fn first_job_id() -> i32 {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    (seconds % 1_000_000_000) as i32 + 1
}

/// The header and attributes, and the document after them as a view of the
/// same buffer: the body is never copied.
fn parse(body: Bytes) -> Result<Request, PrintQueueError> {
    let (header, attributes, reader) = IppParser::new(Cursor::new(body.clone())).parse_parts()?;
    let start = reader.into_inner().position() as usize;
    Ok(Request {
        header,
        attributes,
        document: body.slice(start..),
    })
}

fn value<'a>(attributes: &'a IppAttributes, name: &str) -> Option<&'a IppValue> {
    attributes
        .groups()
        .iter()
        .find_map(|group| group.get(name))
        .map(IppAttribute::value)
}

fn status(response: &mut IppRequestResponse, code: StatusCode) {
    response.header_mut().operation_or_status = code as i16;
}

/// Each sides option and its IPP `sides` keyword.
const SIDES: [(Sides, &str); 3] = [
    (Sides::OneSided, "one-sided"),
    (Sides::LongEdge, "two-sided-long-edge"),
    (Sides::ShortEdge, "two-sided-short-edge"),
];
/// Each color option and its IPP `print-color-mode` keyword.
const COLORS: [(ColorMode, &str); 2] =
    [(ColorMode::Mono, "monochrome"), (ColorMode::Color, "color")];

fn keyword<T: PartialEq + Copy>(table: &[(T, &'static str)], option: T) -> &'static str {
    table
        .iter()
        .find_map(|(each, keyword)| (*each == option).then_some(*keyword))
        .unwrap_or_default()
}

/// The option a request's `name` keyword picks from `table`, if it is one.
fn chosen<T: Copy>(attributes: &IppAttributes, name: &str, table: &[(T, &str)]) -> Option<T> {
    let keyword = value(attributes, name)?.to_string();
    table
        .iter()
        .find_map(|(each, name)| (*name == keyword).then_some(*each))
}

/// Windows names the rendered PDF `application/pdf`; some clients send
/// `application/octet-stream`. Core checks the bytes either way.
fn document_format(attributes: &IppAttributes) -> Result<(), StatusCode> {
    if value(attributes, "document-format").is_some_and(|v| {
        !matches!(
            v.to_string().as_str(),
            "application/pdf" | "application/octet-stream"
        )
    }) {
        return Err(StatusCode::ClientErrorDocumentFormatNotSupported);
    }
    if value(attributes, "compression").is_some_and(|v| v.to_string() != "none") {
        return Err(StatusCode::ClientErrorCompressionNotSupported);
    }
    Ok(())
}

/// Job settings never fail a job: a value this printer does not offer falls
/// back to the printer's default, and copies are clamped to what LPD
/// repeats. The transport always sends whole documents, so copies are
/// collated whatever `multiple-document-handling` asks.
fn options(attributes: &IppAttributes) -> Result<PrintOptions, StatusCode> {
    document_format(attributes)?;
    let defaults = CAPABILITIES.defaults;
    let copies = match value(attributes, "copies") {
        Some(IppValue::Integer(n)) => (*n).clamp(1, CAPABILITIES.max_copies as i32) as u32,
        _ => 1,
    };
    Ok(PrintOptions {
        copies,
        settings: Settings {
            sides: chosen(attributes, "sides", &SIDES).unwrap_or(defaults.sides),
            color: chosen(attributes, "print-color-mode", &COLORS).unwrap_or(defaults.color),
        },
    })
}

#[cfg(test)]
#[path = "../tests/campus_printer_ipp.rs"]
mod tests;
