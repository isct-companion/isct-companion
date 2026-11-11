#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
//! Android print-service job ownership and durable copy progress.
use super::{CampusPrintOutcome, PrintOptions};
use crate::Core;
use adpt_printer::{document, lpd, StudentId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

const LIMIT: usize = 64 * 1024 * 1024;
// Never evict a job Android may still retry. Refuse new jobs at this bound.
const MAX_JOBS: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Job {
    pub id: String,
    pub title: String,
    pub copies: u32,
    pub sides: Option<u8>,
    pub color: Option<bool>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Record {
    job: Job,
    digest: String,
    student: String,
    accepted: u32,
    in_flight: bool,
}
fn valid_job(job: &Job) -> bool {
    !job.id.is_empty()
        && job.sides.is_none_or(|side| side <= 2)
        && (1..=document::MAX_COPIES).contains(&job.copies)
}
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Campus print state could not be stored")]
    Storage,
    #[error("Campus print job does not match its saved identity")]
    Conflict,
    #[error("Invalid campus print job")]
    Invalid,
    #[error("Campus print was cancelled")]
    Cancelled,
    #[error("Campus print document exceeds the limit")]
    TooLarge,
}
fn storage_error(error: impl std::fmt::Display) -> Error {
    log::warn!("[campus-printer] state: {error}");
    Error::Storage
}
#[derive(Clone, Debug)]
pub enum Admission {
    Ready { lease: u64 },
    Busy,
    Unknown { printed: u32 },
}
struct Active {
    id: String,
    job: Job,
    lease: u64,
    cancelled: bool,
    sending: bool,
}
struct State {
    active: Option<Active>,
    next_lease: u64,
}
pub struct Service {
    directory: PathBuf,
    state: Mutex<State>,
    changed: tokio::sync::watch::Sender<u64>,
}

impl Service {
    pub fn new(directory: PathBuf) -> Arc<Self> {
        let (changed, _) = tokio::sync::watch::channel(0);
        Arc::new(Self {
            directory,
            state: Mutex::new(State {
                active: None,
                next_lease: 1,
            }),
            changed,
        })
    }
    fn path(&self, id: &str) -> PathBuf {
        self.directory.join(format!(
            "{}.json",
            data_encoding::HEXLOWER.encode(&Sha256::digest(id.as_bytes()))
        ))
    }
    fn load(&self, id: &str) -> Result<Option<Record>, Error> {
        match std::fs::read(self.path(id)) {
            Ok(data) => {
                let record: Record = serde_json::from_slice(&data).map_err(storage_error)?;
                let valid_digest = record.digest.len() == 64
                    && record
                        .digest
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
                let valid_student = StudentId::parse(&record.student)
                    .is_some_and(|student| student.as_str() == record.student);
                if record.job.id != id
                    || !valid_job(&record.job)
                    || !valid_digest
                    || !valid_student
                    || record.accepted > record.job.copies
                    || (record.in_flight && record.accepted == record.job.copies)
                {
                    return Err(Error::Storage);
                }
                Ok(Some(record))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(Error::Storage),
        }
    }
    fn save(&self, record: &Record) -> Result<(), Error> {
        std::fs::create_dir_all(&self.directory).map_err(storage_error)?;
        let parent = self.directory.parent().ok_or(Error::Storage)?;
        crate::storage::sync_directory(parent).map_err(storage_error)?;
        let bytes = serde_json::to_vec(record).map_err(storage_error)?;
        crate::storage::durable_replace(&self.path(&record.job.id), &bytes).map_err(storage_error)
    }
    pub fn reserve(&self, job: &Job) -> Result<Admission, Error> {
        if !valid_job(job) {
            return Err(Error::Invalid);
        }
        let mut state = self.state.lock().map_err(storage_error)?;
        if state.active.is_some() {
            return Ok(Admission::Busy);
        }
        if let Some(record) = self.load(&job.id)? {
            if record.job != *job {
                return Err(Error::Conflict);
            }
            if record.in_flight {
                return Ok(Admission::Unknown {
                    printed: record.accepted,
                });
            }
        } else if self.directory.exists()
            && std::fs::read_dir(&self.directory)
                .map_err(storage_error)?
                .take(MAX_JOBS)
                .count()
                >= MAX_JOBS
        {
            return Err(Error::Storage);
        }
        let lease = state.next_lease;
        state.next_lease = state.next_lease.checked_add(1).ok_or(Error::Storage)?;
        state.active = Some(Active {
            id: job.id.clone(),
            job: job.clone(),
            lease,
            cancelled: false,
            sending: false,
        });
        Ok(Admission::Ready { lease })
    }
    pub async fn wait_available(&self) -> Result<(), Error> {
        let mut changes = self.changed.subscribe();
        loop {
            if self.state.lock().map_err(storage_error)?.active.is_none() {
                return Ok(());
            }
            changes.changed().await.map_err(storage_error)?;
        }
    }
    pub fn cancel(&self, id: &str) -> Result<bool, Error> {
        let mut state = self.state.lock().map_err(storage_error)?;
        if let Some(active) = state.active.as_mut().filter(|active| active.id == id) {
            active.cancelled = true;
            return Ok(!active.sending);
        }
        Ok(true)
    }
    pub fn forget_completed(&self, id: &str) -> Result<(), Error> {
        let state = self.state.lock().map_err(storage_error)?;
        if state.active.as_ref().is_some_and(|active| active.id == id) {
            return Err(Error::Conflict);
        }
        let record = self.load(id)?.ok_or(Error::Conflict)?;
        if record.in_flight || record.accepted != record.job.copies {
            return Err(Error::Conflict);
        }
        std::fs::remove_file(self.path(id)).map_err(storage_error)?;
        crate::storage::sync_directory(&self.directory).map_err(storage_error)
    }
    pub fn release(&self, id: &str, lease: u64) {
        if let Ok(mut state) = self.state.lock() {
            if state
                .active
                .as_ref()
                .is_some_and(|active| active.id == id && active.lease == lease && !active.sending)
            {
                state.active = None;
                self.changed.send_modify(|v| *v = v.wrapping_add(1));
            }
        }
    }
    fn finish(&self, id: &str) {
        if let Ok(mut state) = self.state.lock() {
            if state.active.as_ref().is_some_and(|active| active.id == id) {
                state.active = None;
                self.changed.send_modify(|v| *v = v.wrapping_add(1));
            }
        }
    }
    pub async fn submit(
        self: Arc<Self>,
        core: Arc<Core>,
        job: Job,
        bytes: Vec<u8>,
        lease: u64,
    ) -> Result<CampusPrintOutcome, Error> {
        {
            let mut state = self.state.lock().map_err(storage_error)?;
            let active = state
                .active
                .as_mut()
                .filter(|active| active.id == job.id && active.lease == lease)
                .ok_or(Error::Cancelled)?;
            if active.job != job {
                return Err(Error::Conflict);
            }
            if active.cancelled || active.sending {
                return Err(Error::Cancelled);
            }
            active.sending = true;
        }
        if bytes.len() > LIMIT {
            self.finish(&job.id);
            return Err(Error::TooLarge);
        }
        let account = core.campus_printer_account().await;
        let Some(student) = account.student() else {
            self.finish(&job.id);
            return Ok(CampusPrintOutcome::Account { account });
        };
        let options = PrintOptions {
            copies: job.copies,
            settings: adpt_printer::pjl::Settings {
                duplex: job.sides.map(|side| match side {
                    0 => adpt_printer::pjl::Duplex::Off,
                    1 => adpt_printer::pjl::Duplex::LongEdge,
                    _ => adpt_printer::pjl::Duplex::ShortEdge,
                }),
                color: job.color.map(|color| {
                    if color {
                        adpt_printer::pjl::ColorMode::Color
                    } else {
                        adpt_printer::pjl::ColorMode::Mono
                    }
                }),
            },
        };
        let this = self.clone();
        let id = job.id.clone();
        let result =
            tokio::task::spawn_blocking(move || this.send(job, bytes, options, student)).await;
        self.finish(&id);
        result.map_err(storage_error)?
    }
    fn send(
        &self,
        job: Job,
        bytes: Vec<u8>,
        mut options: PrintOptions,
        student: StudentId,
    ) -> Result<CampusPrintOutcome, Error> {
        let mut state = self.state.lock().map_err(storage_error)?;
        let active = state
            .active
            .as_mut()
            .filter(|active| active.id == job.id)
            .ok_or(Error::Cancelled)?;
        if active.cancelled {
            return Err(Error::Cancelled);
        }
        let digest = data_encoding::HEXLOWER.encode(&Sha256::digest(&bytes));
        let mut record = match self.load(&job.id)? {
            Some(record) => {
                if record.job != job
                    || record.digest != digest
                    || record.student != student.to_string()
                {
                    return Err(Error::Conflict);
                }
                if record.in_flight {
                    return Ok(CampusPrintOutcome::Uncertain {
                        printed: record.accepted,
                    });
                }
                record
            }
            None => Record {
                job: job.clone(),
                digest,
                student: student.to_string(),
                accepted: 0,
                in_flight: false,
            },
        };
        self.save(&record)?;
        if record.accepted == job.copies {
            return Ok(CampusPrintOutcome::Printed {
                copies: record.accepted,
            });
        }
        let previous = record.accepted;
        options.copies = job.copies - previous;
        drop(state);
        let record = Mutex::new(record);
        let result = document::print_bytes_progress(
            adpt_printer::ADDRESS,
            &student,
            &bytes,
            &job.title,
            &options,
            |_| {
                let state = self
                    .state
                    .lock()
                    .map_err(|_| document::PrintError::Interrupted)?;
                if state
                    .active
                    .as_ref()
                    .is_none_or(|active| active.id != job.id || active.cancelled)
                {
                    return Err(document::PrintError::Interrupted);
                }
                let mut record = record
                    .lock()
                    .map_err(|_| document::PrintError::Interrupted)?;
                record.in_flight = true;
                self.save(&record)
                    .map_err(|_| document::PrintError::Interrupted)
            },
            |accepted| {
                let mut record = record
                    .lock()
                    .map_err(|_| document::PrintError::Interrupted)?;
                record.accepted = previous + accepted;
                record.in_flight = false;
                if self.save(&record).is_err() {
                    record.in_flight = true;
                    return Err(document::PrintError::Interrupted);
                }
                Ok(())
            },
        );
        let mut record = record.lock().map_err(storage_error)?;
        match result {
            Ok(_) => Ok(CampusPrintOutcome::Printed {
                copies: record.accepted,
            }),
            Err(document::PrintError::Lpd { error, .. }) => {
                if error.may_have_printed() {
                    return Ok(CampusPrintOutcome::Uncertain {
                        printed: record.accepted,
                    });
                }
                record.in_flight = false;
                self.save(&record)?;
                if matches!(error, lpd::LpdError::Connect(_)) {
                    Ok(CampusPrintOutcome::Unreachable {
                        printed: record.accepted,
                    })
                } else {
                    Ok(CampusPrintOutcome::NotPrinted {
                        printed: record.accepted,
                    })
                }
            }
            Err(document::PrintError::Interrupted) if record.in_flight => {
                Ok(CampusPrintOutcome::Uncertain {
                    printed: record.accepted,
                })
            }
            Err(document::PrintError::Interrupted) => Err(Error::Cancelled),
            Err(document::PrintError::NotPrintable) => Ok(CampusPrintOutcome::NotPrintable),
            Err(document::PrintError::Copies) => Ok(CampusPrintOutcome::InvalidCopies),
            Err(document::PrintError::File(_)) => Ok(CampusPrintOutcome::Unreadable),
        }
    }
}

#[cfg(test)]
#[path = "../tests/campus_printer_android.rs"]
mod tests;
