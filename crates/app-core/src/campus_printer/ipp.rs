//! Windows renders PDF; Core verifies billing and owns delivery. No document
//! files are retained. IPP usernames never select the billed student account.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::{CampusPrintOutcome, PrintDocument, PrintOptions};
use crate::Core;
use ::ipp::{parser::IppParser, prelude::*};
use adpt_printer::pjl::{ColorMode, Duplex, Settings};
use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
mod attributes;
pub const MAX_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum PrintQueueError {
    #[error("Invalid IPP message: {0}")]
    Protocol(#[from] ::ipp::parser::IppParseError),
    #[error("Could not read the print document: {0}")]
    Document(#[from] std::io::Error),
    #[error("The print queue is unavailable.")]
    Unavailable,
}
#[derive(Clone)]
struct Job {
    id: i32,
    title: String,
    options: PrintOptions,
    state: i32,
    message: String,
    created: Instant,
}
struct Jobs {
    next: i32,
    entries: BTreeMap<i32, Job>,
}
pub struct CampusPrintQueue {
    core: Arc<Core>,
    uri: String,
    started: Instant,
    jobs: Mutex<Jobs>,
    sending: Arc<tokio::sync::Semaphore>,
}
impl CampusPrintQueue {
    pub fn new(core: Arc<Core>, uri: String) -> Arc<Self> {
        Arc::new(Self {
            core,
            uri,
            started: Instant::now(),
            jobs: Mutex::new(Jobs {
                next: (uuid::Uuid::new_v4().as_u128() % 1_000_000_000) as i32 + 1,
                entries: BTreeMap::new(),
            }),
            sending: Arc::new(tokio::sync::Semaphore::new(2)),
        })
    }
    fn update(&self, id: i32, state: i32, message: String) {
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
        jobs.entries
            .retain(|_, job| job.state == 5 || job.created.elapsed() < Duration::from_secs(600));
        if jobs.entries.len() >= 128 {
            let terminal = jobs
                .entries
                .iter()
                .find(|(_, job)| job.state >= 7)
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
            state: 3,
            message: "Waiting for document".into(),
            created: Instant::now(),
        };
        jobs.entries.insert(id, job.clone());
        Ok(Some(job))
    }
    /// The IPP library retains a binary payload reader, avoiding text conversion.
    pub async fn handle(self: &Arc<Self>, bytes: Vec<u8>) -> Result<Vec<u8>, PrintQueueError> {
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
                    Some(job) if job.state != 3 => {
                        status(&mut response, StatusCode::ClientErrorNotPossible)
                    }
                    Some(job) if operation == 8 => {
                        let mut jobs =
                            self.jobs.lock().map_err(|_| PrintQueueError::Unavailable)?;
                        if let Some(current) = jobs.entries.get_mut(&job.id) {
                            if current.state == 3 {
                                current.state = 7;
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
                        || (which == "completed" && job.state >= 7)
                        || (which == "not-completed" && job.state < 7)
                    {
                        self.job_response(&mut response, &job)?;
                    }
                }
            }
            _ => status(&mut response, StatusCode::ServerErrorOperationNotSupported),
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
        if pdf.len() > MAX_DOCUMENT_BYTES || !pdf.starts_with(b"%PDF-") {
            status(response, StatusCode::ClientErrorDocumentFormatError);
            self.update(id, 8, "Invalid PDF document".into());
            return Ok(());
        }
        let permit = match self.sending.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                status(response, StatusCode::ServerErrorBusy);
                return Ok(());
            }
        };
        let job = {
            let mut jobs = self.jobs.lock().map_err(|_| PrintQueueError::Unavailable)?;
            let Some(job) = jobs.entries.get_mut(&id) else {
                status(response, StatusCode::ClientErrorNotFound);
                return Ok(());
            };
            if job.state != 3 {
                status(response, StatusCode::ClientErrorNotPossible);
                return Ok(());
            }
            // Claim before any await: a duplicate Send-Document never resends.
            job.state = 5;
            job.message = "Sending to campus printer".into();
            job.clone()
        };
        let service = self.clone();
        let title = job.title.clone();
        let accepted = self.core.runtime.tasks().spawn(async move {
            let _permit = permit;
            let outcome = service
                .core
                .print_document(PrintDocument::Rendered { bytes: pdf, title }, job.options)
                .await;
            let (state, message) = match outcome {
                CampusPrintOutcome::Printed { .. } => (
                    9,
                    "Accepted by campus printer; release the job at the printer".to_string(),
                ),
                CampusPrintOutcome::Uncertain { .. } => (
                    8,
                    "Delivery uncertain; check the printer before resubmitting".to_string(),
                ),
                other => (8, format!("Campus print failed: {other:?}")),
            };
            log::info!("[campus-printer] IPP job {id}: {message}");
            service.update(id, state, message);
        });
        if !accepted {
            self.update(id, 8, "Application is stopping".into());
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
            ("job-state", IppValue::Enum(job.state)),
            (
                "job-state-message",
                IppValue::new_text_without_language(job.message.clone())?,
            ),
            (
                "job-state-reasons",
                IppValue::new_keyword(match job.state {
                    9 => "job-completed-successfully",
                    8 => "aborted-by-system",
                    7 => "job-canceled-by-user",
                    5 => "job-printing",
                    _ => "job-incoming",
                })?,
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
fn document_format(request: &IppRequestResponse) -> Result<(), StatusCode> {
    if value(request, "document-format").is_some_and(|v| v.to_string() != "application/pdf") {
        return Err(StatusCode::ClientErrorDocumentFormatNotSupported);
    }
    if value(request, "compression").is_some_and(|v| v.to_string() != "none") {
        return Err(StatusCode::ClientErrorCompressionNotSupported);
    }
    Ok(())
}
fn options(request: &IppRequestResponse) -> Result<PrintOptions, StatusCode> {
    document_format(request)?;
    let unsupported = StatusCode::ClientErrorAttributesOrValuesNotSupported;
    let copies = match value(request, "copies") {
        None => 1,
        Some(IppValue::Integer(1)) => 1,
        _ => return Err(unsupported),
    };
    let duplex = match value(request, "sides")
        .map(ToString::to_string)
        .as_deref()
        .unwrap_or("one-sided")
    {
        "one-sided" => Duplex::Off,
        _ => return Err(unsupported),
    };
    let color = match value(request, "print-color-mode")
        .map(ToString::to_string)
        .as_deref()
        .unwrap_or("color")
    {
        "color" => ColorMode::Color,
        _ => return Err(unsupported),
    };
    Ok(PrintOptions {
        copies,
        settings: Settings {
            duplex: Some(duplex),
            color: Some(color),
        },
    })
}
