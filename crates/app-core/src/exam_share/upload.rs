use super::*;
use bytes::Bytes;
use reqwest::{header, Body};
use serde::Serialize;
use std::{path::Path, sync::Mutex};
use tokio::io::AsyncWriteExt;

const REGISTER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const RECOVERY_DELAY: i64 = 60;

pub(super) struct UploadTracker(Mutex<Progress>);

struct Progress {
    status: ExamUploadStatus,
    cancel: Option<CancellationToken>,
}

struct UploadRun<'a> {
    tracker: &'a UploadTracker,
    journal: &'a Path,
}

impl UploadTracker {
    pub fn new() -> Self {
        Self(Mutex::new(Progress {
            status: mode(ExamUploadPhase::Idle),
            cancel: None,
        }))
    }

    pub fn status(&self) -> Result<ExamUploadStatus, ExamError> {
        self.0
            .lock()
            .map(|progress| progress.status)
            .map_err(|_| ExamError::Storage)
    }

    pub fn cancel(&self) -> Result<(), ExamError> {
        let progress = self.0.lock().map_err(|_| ExamError::Storage)?;
        if matches!(
            progress.status.phase,
            ExamUploadPhase::Preparing | ExamUploadPhase::Transferring
        ) {
            if let Some(cancel) = &progress.cancel {
                cancel.cancel();
            }
        }
        Ok(())
    }

    pub fn settled(&self, pending: bool) {
        if let Ok(mut progress) = self.0.lock() {
            progress.status = mode(if pending {
                ExamUploadPhase::Pending
            } else {
                ExamUploadPhase::Idle
            });
            progress.cancel = None;
        }
    }

    fn begin<'a>(
        &'a self,
        journal: &'a Path,
        phase: ExamUploadPhase,
        cancel: &CancellationToken,
    ) -> Result<UploadRun<'a>, ExamError> {
        let mut progress = self.0.lock().map_err(|_| ExamError::Storage)?;
        progress.status = mode(phase);
        progress.cancel = Some(cancel.clone());
        Ok(UploadRun {
            tracker: self,
            journal,
        })
    }

    fn update(&self, status: ExamUploadStatus) {
        if let Ok(mut progress) = self.0.lock() {
            progress.status = status;
        }
    }
}

impl Drop for UploadRun<'_> {
    fn drop(&mut self) {
        self.tracker.settled(self.journal.is_file());
    }
}

fn mode(phase: ExamUploadPhase) -> ExamUploadStatus {
    ExamUploadStatus {
        phase,
        sent: 0,
        total: 0,
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Authorization {
    storage_path: String,
    upload_url: String,
    content_type: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Created {
    exam_id: String,
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
        self.account_folder(uid).with_extension("upload.json")
    }

    pub(super) fn update_upload_account(&self, session: &Session) {
        let pending = session
            .saved
            .as_ref()
            .filter(|saved| saved.registered)
            .is_some_and(|saved| self.upload_journal(&saved.uid).is_file());
        self.upload.settled(pending);
    }

    pub(super) async fn upload_action(
        &self,
        session: &mut Session,
        action: ExamAction,
        cancel: &CancellationToken,
    ) -> Result<ExamAnswer, ExamError> {
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
        match action {
            ExamAction::Upload { files, draft } => {
                if read_pending(&path).await?.is_some() {
                    return Err(ExamError::UploadPending);
                }
                let _run = self
                    .upload
                    .begin(&path, ExamUploadPhase::Preparing, cancel)?;
                self.send_upload(session, &uid, &path, files, draft, cancel)
                    .await
            }
            ExamAction::RecoverUpload => {
                let Some(mut pending) = read_pending(&path).await? else {
                    return Ok(ExamAnswer::Done);
                };
                if chrono::Utc::now().timestamp() < pending.retry_after {
                    return Err(ExamError::UploadPending);
                }
                let phase = match pending.phase {
                    Phase::Register => ExamUploadPhase::Registering,
                    Phase::Transfer | Phase::Cleanup => ExamUploadPhase::Cleaning,
                };
                let _run = self.upload.begin(&path, phase, cancel)?;
                match pending.phase {
                    Phase::Register => {
                        self.register_upload(session, &path, &mut pending, cancel)
                            .await
                    }
                    Phase::Transfer | Phase::Cleanup => {
                        self.rollback_upload(session, &path, &mut pending, cancel)
                            .await?;
                        Ok(ExamAnswer::Done)
                    }
                }
            }
            _ => Err(ExamError::InvalidInput),
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
        validate(&mut draft)?;
        let preparation = cancel.clone();
        let file =
            tokio::task::spawn_blocking(move || super::upload_file::prepare(&files, &preparation))
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
        let authorization: Authorization = interrupt(cancel, self.call(
            session, self.service(Method::POST, &["exams", "uploads", "authorize"])?
                .json(&json!({ "fileName": file.name, "contentType": file.content_type, "size": size })),
        )).await?;
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
            retry_after: chrono::Utc::now().timestamp() + RECOVERY_DELAY,
        };
        write_pending(path, &pending).await?;
        match interrupt(cancel, self.put(url, file.content_type, file.bytes, size)).await {
            Ok(()) => {
                self.register_upload(session, path, &mut pending, cancel)
                    .await
            }
            Err(error) => {
                pending.retry_after = chrono::Utc::now().timestamp() + RECOVERY_DELAY;
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
        self.upload.update(ExamUploadStatus {
            phase: ExamUploadPhase::Transferring,
            sent: 0,
            total,
        });
        // The body can outlive this request future. A channel reports bytes consumed
        // by the HTTP transport without borrowing application state in the stream.
        let (progress, mut received) = tokio::sync::watch::channel(0);
        let chunks = futures_util::stream::unfold(
            (Bytes::from(bytes), 0, progress),
            |(mut rest, sent, progress)| async move {
                if rest.is_empty() {
                    return None;
                }
                let chunk = rest.split_to(rest.len().min(64 * 1024));
                let sent = sent + chunk.len() as u32;
                progress.send_replace(sent);
                Some((Ok::<_, std::io::Error>(chunk), (rest, sent, progress)))
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
                    self.upload.update(ExamUploadStatus { phase: ExamUploadPhase::Transferring, sent, total });
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
        pending.retry_after = chrono::Utc::now().timestamp() + RECOVERY_DELAY;
        write_pending(path, pending).await?;
        let answer: Result<Created, _> = interrupt(
            cancel,
            self.call(
                session,
                self.service(Method::POST, &["exams"])?
                    .timeout(REGISTER_TIMEOUT)
                    .json(&pending.registration),
            ),
        )
        .await;
        let id = match answer {
            Ok(answer) => answer.exam_id,
            Err(ExamError::UploadRegistered(id)) => id,
            Err(
                error @ (ExamError::UploadDuplicate
                | ExamError::UploadQuota
                | ExamError::UploadFile
                | ExamError::UploadSize
                | ExamError::InvalidInput),
            ) => {
                pending.phase = Phase::Cleanup;
                pending.retry_after = chrono::Utc::now().timestamp();
                write_pending(path, pending).await?;
                let _ = self.rollback_upload(session, path, pending, cancel).await;
                return Err(error);
            }
            Err(
                ExamError::Network
                | ExamError::Timeout
                | ExamError::Response
                | ExamError::Unavailable,
            ) => {
                pending.retry_after = chrono::Utc::now().timestamp() + RECOVERY_DELAY;
                write_pending(path, pending).await?;
                return Err(ExamError::UploadPending);
            }
            Err(error) => return Err(error),
        };
        if id.is_empty() {
            return Err(ExamError::UploadPending);
        }
        tokio::fs::remove_file(path)
            .await
            .map_err(|_| ExamError::Storage)?;
        Ok(ExamAnswer::Uploaded(id))
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

async fn interrupt<T>(
    cancel: &CancellationToken,
    work: impl std::future::Future<Output = Result<T, ExamError>>,
) -> Result<T, ExamError> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(ExamError::Cancelled),
        result = work => result,
    }
}

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
    let temp = path.with_extension("new");
    let data = serde_json::to_vec(pending).map_err(|_| ExamError::Storage)?;
    let mut file = tokio::fs::File::create(&temp)
        .await
        .map_err(|_| ExamError::Storage)?;
    file.write_all(&data)
        .await
        .map_err(|_| ExamError::Storage)?;
    file.sync_all().await.map_err(|_| ExamError::Storage)?;
    drop(file);
    tokio::fs::rename(&temp, path)
        .await
        .map_err(|_| ExamError::Storage)
}

fn fits(value: &str, limit: usize) -> bool {
    value.encode_utf16().count() <= limit
}

fn validate(draft: &mut ExamUploadDraft) -> Result<(), ExamError> {
    let ordinary = matches!(draft.role.as_str(), "question" | "answer" | "both");
    let non_exam = matches!(draft.role.as_str(), "notes" | "other");
    if !ordinary && !non_exam {
        return Err(ExamError::InvalidInput);
    }
    if non_exam {
        draft.occasion = "none".into();
    } else if !matches!(
        draft.occasion.as_str(),
        "midterm"
            | "final"
            | "makeup"
            | "quiz"
            | "check"
            | "report"
            | "exercise"
            | "textbook"
            | "mock"
            | "unknown"
    ) {
        return Err(ExamError::InvalidInput);
    }
    if !matches!(draft.occasion.as_str(), "quiz" | "check" | "report") {
        draft.occasion_number = 0;
    }
    if !matches!(draft.role.as_str(), "answer" | "both") {
        draft.answer_source = None;
    } else if !matches!(
        draft.answer_source.as_deref(),
        Some("official" | "student" | "unknown")
    ) {
        return Err(ExamError::InvalidInput);
    }
    let next_year = chrono::Datelike::year(&chrono::Utc::now()) as u32 + 1;
    if draft.subject_codes.is_empty()
        || draft.subject_codes.len() > 10
        || draft
            .subject_codes
            .iter()
            .any(|code| code.is_empty() || !fits(code, 40))
        || draft.teacher_ids.len() > 40
        || draft.new_teachers.len() > 40
        || draft
            .teacher_ids
            .iter()
            .any(|id| id.is_empty() || !fits(id, 200))
        || draft.new_teachers.iter().any(|name| {
            name.family_name.trim().is_empty()
                || !fits(&name.family_name, 100)
                || !fits(&name.given_name, 100)
        })
        || draft
            .academic_year
            .is_some_and(|year| !(1950..=next_year).contains(&year))
        || draft.occasion_number > 30
        || !fits(&draft.variant, 100)
        || !fits(draft.note.trim(), 40)
        || !matches!(draft.provenance.as_str(), "original" | "reconstructed")
        || draft
            .request_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || !fits(id, 100))
    {
        return Err(ExamError::InvalidInput);
    }
    Ok(())
}
