use super::*;
use bytes::Bytes;
use reqwest::{header, Body};
use serde::Serialize;
use std::{path::Path, sync::Mutex};

const REGISTER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const RECOVERY_DELAY: i64 = 60;

/// The upload in progress. Every change reaches the shells through the
/// account it belongs to (`Publisher::upload`).
pub(super) struct UploadTracker {
    progress: Mutex<Progress>,
    publisher: Arc<Publisher>,
}

struct Progress {
    status: ExamUploadStatus,
    cancel: Option<CancellationToken>,
}

struct UploadRun<'a> {
    tracker: &'a UploadTracker,
    journal: &'a Path,
}

impl UploadTracker {
    pub fn new(publisher: Arc<Publisher>, status: ExamUploadStatus) -> Self {
        Self {
            progress: Mutex::new(Progress {
                status,
                cancel: None,
            }),
            publisher,
        }
    }

    pub fn cancel(&self) -> Result<(), ExamError> {
        let progress = self.progress.lock().map_err(|_| ExamError::Storage)?;
        if progress.status.cancellable {
            if let Some(cancel) = &progress.cancel {
                cancel.cancel();
            }
        }
        Ok(())
    }

    pub fn settled(&self, pending: bool) {
        self.set(settled_status(pending), None);
    }

    fn begin<'a>(
        &'a self,
        journal: &'a Path,
        phase: ExamUploadPhase,
        cancel: &CancellationToken,
    ) -> UploadRun<'a> {
        self.set(mode(phase), Some(cancel.clone()));
        UploadRun {
            tracker: self,
            journal,
        }
    }

    fn update(&self, status: ExamUploadStatus) {
        if let Ok(mut progress) = self.progress.lock() {
            progress.status = status;
        }
        self.publisher.upload(status);
    }

    fn set(&self, status: ExamUploadStatus, cancel: Option<CancellationToken>) {
        if let Ok(mut progress) = self.progress.lock() {
            progress.status = status;
            progress.cancel = cancel;
        }
        self.publisher.upload(status);
    }
}

impl Drop for UploadRun<'_> {
    fn drop(&mut self) {
        self.tracker.settled(self.journal.is_file());
    }
}

/// The upload to recover, beside the account's folder. It took this name
/// when the draft changed after 1.4.0, so 1.4.0's is never read; deleting
/// the account removes it (`OLD_JOURNAL`).
fn journal(folder: &Path, uid: &str) -> PathBuf {
    super::cache::account_folder(folder, uid).with_extension("pending-upload.json")
}

/// The name 1.4.0 kept its upload journal under.
pub(super) const OLD_JOURNAL: &str = "upload.json";

/// Whether the session's account left an upload to recover.
pub(super) fn pending_upload(folder: &Path, session: &Session) -> bool {
    session
        .saved
        .as_ref()
        .filter(|saved| saved.registered)
        .is_some_and(|saved| journal(folder, &saved.uid).is_file())
}

/// The status a run ends in: a journal left behind needs recovery.
pub(super) fn settled_status(pending: bool) -> ExamUploadStatus {
    mode(if pending {
        ExamUploadPhase::Pending
    } else {
        ExamUploadPhase::Idle
    })
}

pub(super) fn mode(phase: ExamUploadPhase) -> ExamUploadStatus {
    progress(phase, 0, 0)
}

fn progress(phase: ExamUploadPhase, sent: u32, total: u32) -> ExamUploadStatus {
    ExamUploadStatus {
        phase,
        sent,
        total,
        // Once registration starts, stopping could publish it unseen.
        cancellable: matches!(
            phase,
            ExamUploadPhase::Preparing | ExamUploadPhase::Transferring
        ),
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Registration {
    #[serde(flatten)]
    fields: ExamUploadDraft,
    storage_path: String,
    file_name: String,
    file_size: u32,
    file_type: String,
    file_hash: String,
}

#[derive(Serialize, Deserialize)]
enum Phase {
    Transfer,
    Register,
    Cleanup,
}

#[derive(Serialize, Deserialize)]
struct Pending {
    registration: Registration,
    phase: Phase,
    retry_after: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthorizationRequest<'a> {
    #[serde(flatten)]
    fields: &'a ExamUploadDraft,
    file_name: &'a str,
    content_type: &'a str,
    size: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Authorization {
    storage_path: String,
    upload_url: String,
    content_type: String,
}

#[derive(Deserialize)]
struct Duplicate {
    exists: bool,
}

#[derive(Deserialize)]
struct Deleted {
    deleted: bool,
}

impl ExamShare {
    pub(super) fn upload_journal(&self, uid: &str) -> PathBuf {
        journal(&self.folder, uid)
    }

    pub(super) fn update_upload_account(&self, session: &Session) {
        self.upload.settled(pending_upload(&self.folder, session));
    }

    /// The connected account's id and upload journal, resuming it if needed.
    async fn upload_account(
        &self,
        session: &mut Session,
        cancel: &CancellationToken,
    ) -> Result<(String, PathBuf), ExamError> {
        if session.token.is_none() {
            interrupt(cancel, self.resume(session)).await?;
        }
        if !matches!(session.status, ExamAccountStatus::Connected) {
            return Err(ExamError::Consent);
        }
        let uid = session
            .saved
            .as_ref()
            .ok_or(ExamError::SignedOut)?
            .uid
            .clone();
        let path = self.upload_journal(&uid);
        Ok((uid, path))
    }

    pub(super) async fn upload(
        &self,
        session: &mut Session,
        files: Vec<ExamUploadFile>,
        draft: ExamUploadDraft,
        cancel: &CancellationToken,
    ) -> Result<ExamAnswer, ExamError> {
        let (uid, path) = self.upload_account(session, cancel).await?;
        if read_pending(&path).await?.is_some() {
            return Err(ExamError::UploadPending);
        }
        let _run = self.upload.begin(&path, ExamUploadPhase::Preparing, cancel);
        self.send_upload(session, &uid, &path, files, draft, cancel)
            .await
    }

    pub(super) async fn recover_upload(
        &self,
        session: &mut Session,
        cancel: &CancellationToken,
    ) -> Result<ExamAnswer, ExamError> {
        let (_, path) = self.upload_account(session, cancel).await?;
        let Some(mut pending) = read_pending(&path).await? else {
            self.upload.settled(false);
            return Ok(ExamAnswer::Done);
        };
        if crate::clock::now().timestamp() < pending.retry_after {
            return Err(ExamError::UploadPending);
        }
        match pending.phase {
            Phase::Register => {
                let _run = self
                    .upload
                    .begin(&path, ExamUploadPhase::Registering, cancel);
                self.register_upload(session, &path, &mut pending, cancel)
                    .await
            }
            Phase::Transfer | Phase::Cleanup => {
                let _run = self.upload.begin(&path, ExamUploadPhase::Cleaning, cancel);
                self.rollback_upload(session, &path, &mut pending, cancel)
                    .await?;
                Ok(ExamAnswer::Done)
            }
        }
    }

    async fn send_upload(
        &self,
        session: &mut Session,
        uid: &str,
        path: &Path,
        files: Vec<ExamUploadFile>,
        mut draft: ExamUploadDraft,
        cancel: &CancellationToken,
    ) -> Result<ExamAnswer, ExamError> {
        let choices = interrupt(cancel, self.choices()).await?;
        form::normalize(&mut draft, &choices);
        // A service that names no limit checks the size itself.
        let limit = choices.limits.max_upload_bytes.map_or(u64::MAX, u64::from);
        let preparation = cancel.clone();
        let file = tokio::task::spawn_blocking(move || {
            super::upload_file::prepare(&files, limit, &preparation)
        })
        .await
        .map_err(|_| ExamError::UploadFile)??;
        let duplicate: Duplicate = interrupt(
            cancel,
            self.call(
                session,
                self.service(Method::POST, &["exams", "hash-exists"])?
                    .json(&json!({ "fileHash": file.hash })),
            ),
        )
        .await?;
        if duplicate.exists {
            return Err(ExamError::UploadDuplicate);
        }
        let size = file.bytes.len() as u32;
        // The whole draft goes with the request, so the service can refuse
        // it before the file is sent.
        let authorization: Authorization = interrupt(
            cancel,
            self.call(
                session,
                self.service(Method::POST, &["exams", "uploads", "authorize"])?
                    .json(&service_body(&AuthorizationRequest {
                        fields: &draft,
                        file_name: &file.name,
                        content_type: file.content_type,
                        size,
                    })?),
            ),
        )
        .await?;
        let url = url::Url::parse(&authorization.upload_url).map_err(|_| ExamError::Response)?;
        if !client::is_file_url(&url)
            || authorization.content_type != file.content_type
            || !matches!(authorization.storage_path.split('/').collect::<Vec<_>>().as_slice(),
                ["exams", owner, key] if *owner == uid && !matches!(*key, "" | "." | ".."))
        {
            return Err(ExamError::Response);
        }
        let mut pending = Pending {
            registration: Registration {
                fields: draft,
                storage_path: authorization.storage_path,
                file_name: file.name,
                file_size: size,
                file_type: authorization.content_type,
                file_hash: file.hash,
            },
            phase: Phase::Transfer,
            retry_after: crate::clock::now().timestamp() + RECOVERY_DELAY,
        };
        write_pending(path, &pending).await?;
        match interrupt(cancel, self.put(url, file.content_type, file.bytes, size)).await {
            Ok(()) => {
                self.register_upload(session, path, &mut pending, cancel)
                    .await
            }
            Err(error) => {
                pending.retry_after = crate::clock::now().timestamp() + RECOVERY_DELAY;
                write_pending(path, &pending).await?;
                Err(error)
            }
        }
    }

    async fn put(
        &self,
        url: url::Url,
        content_type: &str,
        bytes: Vec<u8>,
        total: u32,
    ) -> Result<(), ExamError> {
        self.upload
            .update(progress(ExamUploadPhase::Transferring, 0, total));
        // The body can outlive this request future. A channel reports bytes consumed
        // by the HTTP transport without borrowing application state in the stream.
        let (report, mut received) = tokio::sync::watch::channel(0);
        let chunks = futures_util::stream::unfold(
            (Bytes::from(bytes), 0, report),
            |(mut rest, sent, report)| async move {
                if rest.is_empty() {
                    return None;
                }
                let chunk = rest.split_to(rest.len().min(64 * 1024));
                let sent = sent + chunk.len() as u32;
                report.send_replace(sent);
                Some((Ok::<_, std::io::Error>(chunk), (rest, sent, report)))
            },
        );
        let request = self
            .api
            .http
            .put(url)
            .header(header::CONTENT_TYPE, content_type)
            .header(header::CONTENT_LENGTH, total)
            .header(header::IF_NONE_MATCH, "*")
            .body(Body::wrap_stream(chunks))
            .send();
        tokio::pin!(request);
        let response = loop {
            tokio::select! {
                response = &mut request => break response.map_err(client::network)?,
                Ok(()) = received.changed() => {
                    let sent = *received.borrow_and_update();
                    self.upload.update(progress(ExamUploadPhase::Transferring, sent, total));
                }
            }
        };
        if !response.status().is_success() {
            return Err(ExamError::Unavailable);
        }
        Ok(())
    }

    async fn register_upload(
        &self,
        session: &mut Session,
        path: &Path,
        pending: &mut Pending,
        cancel: &CancellationToken,
    ) -> Result<ExamAnswer, ExamError> {
        self.upload.update(mode(ExamUploadPhase::Registering));
        pending.phase = Phase::Register;
        pending.retry_after = crate::clock::now().timestamp() + RECOVERY_DELAY;
        write_pending(path, pending).await?;
        let answer: Result<Value, _> = interrupt(
            cancel,
            self.call(
                session,
                self.service(Method::POST, &["exams"])?
                    .timeout(REGISTER_TIMEOUT)
                    .json(&service_body(&pending.registration)?),
            ),
        )
        .await;
        let exam_id = match answer {
            // Registered by this attempt, or by one whose answer was lost.
            Ok(created) => created
                .get("examId")
                .and_then(Value::as_str)
                .map(str::to_owned),
            Err(ExamError::UploadRegistered) => None,
            // Refused: the file it left is removed.
            Err(error) if error.refused_by_service() => {
                pending.phase = Phase::Cleanup;
                pending.retry_after = crate::clock::now().timestamp();
                write_pending(path, pending).await?;
                let _ = self.rollback_upload(session, path, pending, cancel).await;
                return Err(error);
            }
            // The answer may have been lost: recover it later.
            Err(
                ExamError::Network
                | ExamError::Timeout
                | ExamError::Response
                | ExamError::Unavailable,
            ) => {
                pending.retry_after = crate::clock::now().timestamp() + RECOVERY_DELAY;
                write_pending(path, pending).await?;
                return Err(ExamError::UploadPending);
            }
            // The session ended or the run stopped; recovery resumes it.
            Err(error) => return Err(error),
        };
        tokio::fs::remove_file(path)
            .await
            .map_err(|_| ExamError::Storage)?;
        Ok(ExamAnswer::Uploaded(exam_id))
    }

    async fn rollback_upload(
        &self,
        session: &mut Session,
        path: &Path,
        pending: &mut Pending,
        cancel: &CancellationToken,
    ) -> Result<(), ExamError> {
        self.upload.update(mode(ExamUploadPhase::Cleaning));
        let answer: Result<Deleted, _> = interrupt(
            cancel,
            self.call(
                session,
                self.service(Method::POST, &["exams", "uploads", "rollback"])?
                    .json(&json!({ "storagePath": pending.registration.storage_path })),
            ),
        )
        .await;
        match answer {
            Ok(Deleted { deleted: true }) => tokio::fs::remove_file(path)
                .await
                .map_err(|_| ExamError::Storage),
            Err(ExamError::UploadInUse) => {
                pending.phase = Phase::Register;
                write_pending(path, pending).await?;
                Err(ExamError::UploadPending)
            }
            Ok(Deleted { deleted: false }) => Err(ExamError::UploadPending),
            Err(error) => Err(error),
        }
    }
}

/// `value` as the service reads it: an occasion not picked yet is left out,
/// which the service refuses, rather than sent as null, which it still takes
/// as 1.4.0's "unknown" occasion.
pub(super) fn service_body(value: &impl Serialize) -> Result<Value, ExamError> {
    let mut body = serde_json::to_value(value).map_err(|_| ExamError::Response)?;
    if let Some(fields) = body.as_object_mut() {
        if fields.get("occasion").is_some_and(Value::is_null) {
            fields.remove("occasion");
        }
    }
    Ok(body)
}

/// The upload to recover, if one is left.
async fn read_pending(path: &Path) -> Result<Option<Pending>, ExamError> {
    match tokio::fs::read(path).await {
        Ok(data) => serde_json::from_slice(&data)
            .map(Some)
            .map_err(|_| ExamError::Storage),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(ExamError::Storage),
    }
}

async fn write_pending(path: &Path, pending: &Pending) -> Result<(), ExamError> {
    let data = serde_json::to_vec(pending).map_err(|_| ExamError::Storage)?;
    super::cache::write_atomically(path, &data).await
}
