use super::*;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteExam {
    #[serde(flatten)]
    document: ExamDocument,
    storage_path: String,
    updated_at: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubjectDetails {
    #[serde(flatten)]
    page: ExamSubjectPage,
    current_teachers: Vec<ExamTeacher>,
    past_teachers: Vec<ExamTeacher>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SubjectResponse {
    Page(SubjectDetails),
    Moved {
        #[serde(rename = "movedTo")]
        moved_to: Value,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteRequest {
    #[serde(flatten)]
    request: RequestRecord,
    author_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestRecord {
    id: String,
    subject: ExamSubject,
    teachers: Vec<ExamTeacher>,
    academic_year: Option<u32>,
    occasion: Option<String>,
    wanted_role: Option<String>,
    description: String,
    status: String,
    answer_exam_id: Option<String>,
}

impl RemoteRequest {
    fn model(self, uid: &str) -> ExamRequest {
        let r = self.request;
        ExamRequest {
            id: r.id,
            subject: r.subject,
            teachers: r.teachers,
            academic_year: r.academic_year,
            occasion: r.occasion,
            wanted_role: r.wanted_role,
            description: r.description,
            status: r.status,
            answer_exam_id: r.answer_exam_id,
            mine: self.author_id == uid,
        }
    }
}

#[derive(Deserialize)]
struct Exams {
    exams: Vec<ExamDocument>,
}
#[derive(Deserialize)]
struct OneExam {
    exam: Option<RemoteExam>,
}
#[derive(Deserialize)]
struct Subjects {
    subjects: Vec<ExamSubject>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    exam_ids: Vec<String>,
}
#[derive(Deserialize)]
struct Requests {
    items: Vec<RemoteRequest>,
    cursor: Option<ExamRequestCursor>,
}
#[derive(Deserialize)]
struct ReadUrl {
    url: String,
}

#[derive(serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cached {
    document: ExamDocument,
    updated_at: u64,
    downloaded: bool,
    // A new download uses a different name, so replacing the manifest works
    // while a native preview still holds the previous file open.
    file: uuid::Uuid,
}

fn key(value: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(value.as_bytes()))
}

impl ExamShare {
    pub(super) fn account_folder(&self, uid: &str) -> PathBuf {
        self.folder.join(key(uid))
    }

    async fn subject_details(
        &self,
        session: &mut Session,
        code: &str,
    ) -> Result<SubjectDetails, ExamError> {
        let response: SubjectResponse = self
            .call(
                session,
                self.service(Method::GET, &["subjects", code, "view"])?,
            )
            .await?;
        match response {
            SubjectResponse::Page(details) => Ok(details),
            SubjectResponse::Moved {
                moved_to: Value::String(code),
            } => {
                self.call(
                    session,
                    self.service(Method::GET, &["subjects", &code, "view"])?,
                )
                .await
            }
            SubjectResponse::Moved {
                moved_to: Value::Null,
            } => Err(ExamError::NotFound),
            SubjectResponse::Moved { .. } => Err(ExamError::Response),
        }
    }

    pub(super) async fn feature(
        &self,
        session: &mut Session,
        action: ExamAction,
    ) -> Result<ExamAnswer, ExamError> {
        let get = Method::GET;
        let post = Method::POST;
        match action {
            ExamAction::Search { query } => {
                let query = query.trim();
                if query.is_empty() {
                    return Ok(ExamAnswer::Subjects(vec![]));
                }
                if query.encode_utf16().count() > 100 {
                    return Err(ExamError::InvalidInput);
                }
                let answer: Subjects = self
                    .call(
                        session,
                        self.service(get, &["subjects", "search"])?
                            .query(&[("q", query)]),
                    )
                    .await?;
                Ok(ExamAnswer::Subjects(answer.subjects))
            }
            ExamAction::Subject { code } => {
                let details = self.subject_details(session, &code).await?;
                Ok(ExamAnswer::Subject(details.page))
            }
            ExamAction::UploadOptions {
                subject_code,
                request,
            } => {
                if request
                    .as_ref()
                    .is_some_and(|item| item.subject.code != subject_code || item.status != "open")
                {
                    return Err(ExamError::InvalidInput);
                }
                let details = self.subject_details(session, &subject_code).await?;
                let mut seen = std::collections::HashSet::new();
                let teachers: Vec<_> = details
                    .current_teachers
                    .into_iter()
                    .chain(details.past_teachers)
                    .filter(|teacher| seen.insert(teacher.id.clone()))
                    .collect();
                let request = request.as_ref();
                let role = request
                    .and_then(|item| item.wanted_role.as_deref())
                    .unwrap_or("question");
                let draft = ExamUploadDraft {
                    subject_codes: vec![details.page.subject.code],
                    teacher_ids: request
                        .into_iter()
                        .flat_map(|item| &item.teachers)
                        .filter(|teacher| seen.contains(&teacher.id))
                        .map(|teacher| teacher.id.clone())
                        .collect(),
                    new_teachers: vec![],
                    academic_year: request.and_then(|item| item.academic_year),
                    occasion: request
                        .and_then(|item| item.occasion.clone())
                        .unwrap_or_else(|| "unknown".into()),
                    occasion_number: 0,
                    variant: String::new(),
                    role: role.into(),
                    provenance: "original".into(),
                    answer_source: (role == "answer").then(|| "unknown".into()),
                    note: String::new(),
                    request_id: request.map(|item| item.id.clone()),
                };
                Ok(ExamAnswer::UploadOptions(ExamUploadOptions {
                    draft,
                    teachers,
                }))
            }
            ExamAction::Open { id, download } => self
                .open(session, &id, download)
                .await
                .map(ExamAnswer::Document),
            ExamAction::Library => self.library(session).await.map(ExamAnswer::Library),
            ExamAction::RemoveDownload { id } => {
                let uid = session
                    .saved
                    .as_ref()
                    .ok_or(ExamError::SignedOut)?
                    .uid
                    .clone();
                let base = self.account_folder(&uid).join(key(&id));
                erase(&base).await?;
                Ok(ExamAnswer::Done)
            }
            ExamAction::Save { id, saved } => {
                let path = ["me", "saved-exams", &id];
                let answer: Saved = self
                    .call(
                        session,
                        self.service(post, &path)?.json(&json!({"saved": saved})),
                    )
                    .await?;
                Ok(ExamAnswer::Saved(answer.exam_ids.contains(&id)))
            }
            ExamAction::DeleteSubmission { id } => {
                let _: Value = self
                    .call(session, self.service(post, &["exams", &id, "delete"])?)
                    .await?;
                let uid = &session.saved.as_ref().ok_or(ExamError::SignedOut)?.uid;
                erase(&self.account_folder(uid).join(key(&id))).await?;
                Ok(ExamAnswer::Done)
            }
            ExamAction::Report { id, reason, detail } => {
                let detail = detail.trim();
                if detail.encode_utf16().count() > 900 {
                    return Err(ExamError::InvalidInput);
                }
                let _: Value = self
                    .call(
                        session,
                        self.service(post, &["exams", &id, "report"])?
                            .json(&json!({"reason": reason.api_code(), "detail": detail})),
                    )
                    .await?;
                Ok(ExamAnswer::Done)
            }
            ExamAction::Requests { mine, cursor } => {
                let request = if mine {
                    self.service(get, &["me", "requests"])?
                } else {
                    self.service(get, &["requests"])?
                };
                let request = if let Some(cursor) = cursor {
                    request.query(&[
                        ("cursorCreatedAt", cursor.created_at.to_string()),
                        ("cursorId", cursor.id),
                    ])
                } else {
                    request
                };
                let answer: Requests = self.call(session, request).await?;
                let uid = &session.saved.as_ref().ok_or(ExamError::SignedOut)?.uid;
                Ok(ExamAnswer::Requests(ExamRequestPage {
                    items: answer
                        .items
                        .into_iter()
                        .map(|request| request.model(uid))
                        .collect(),
                    cursor: answer.cursor,
                }))
            }
            ExamAction::CreateRequest { draft } => {
                let _: Value = self
                    .call(session, self.service(post, &["requests"])?.json(&draft))
                    .await?;
                Ok(ExamAnswer::Done)
            }
            ExamAction::SetRequestOpen { id, open } => {
                let status = if open { "open" } else { "closed" };
                let _: Value = self
                    .call(
                        session,
                        self.service(post, &["requests", &id, "status"])?
                            .json(&json!({"status":status})),
                    )
                    .await?;
                Ok(ExamAnswer::Done)
            }
            ExamAction::DeleteRequest { id } => {
                let _: Value = self
                    .call(session, self.service(post, &["requests", &id, "delete"])?)
                    .await?;
                Ok(ExamAnswer::Done)
            }
            ExamAction::Account
            | ExamAction::AccountSnapshot
            | ExamAction::Connect
            | ExamAction::CancelConnect
            | ExamAction::AcceptTerms
            | ExamAction::Disconnect
            | ExamAction::Upload { .. }
            | ExamAction::UploadStatus
            | ExamAction::CancelUpload
            | ExamAction::RecoverUpload => Err(ExamError::InvalidInput),
        }
    }

    async fn library(&self, session: &mut Session) -> Result<ExamLibrary, ExamError> {
        let remote = self.remote_library(session).await;
        let (saved, submitted, offline) = match remote {
            Ok((saved, submitted)) => (saved, submitted, false),
            Err(error)
                if error.temporarily_unavailable()
                    && session
                        .saved
                        .as_ref()
                        .is_some_and(|session| session.registered) =>
            {
                (vec![], vec![], true)
            }
            Err(error) => return Err(error),
        };
        let uid = &session.saved.as_ref().ok_or(ExamError::SignedOut)?.uid;
        Ok(ExamLibrary {
            offline,
            saved,
            submitted,
            downloaded: downloads(&self.account_folder(uid)).await?,
        })
    }

    async fn remote_library(
        &self,
        session: &mut Session,
    ) -> Result<(Vec<ExamDocument>, Vec<ExamDocument>), ExamError> {
        let saved: Saved = self
            .call(session, self.service(Method::GET, &["me", "saved-exams"])?)
            .await?;
        let exams = if saved.exam_ids.is_empty() {
            vec![]
        } else {
            let answer: Exams = self
                .call(
                    session,
                    self.service(Method::POST, &["exams", "by-ids"])?
                        .json(&json!({"ids": saved.exam_ids})),
                )
                .await?;
            answer.exams
        };
        let submitted: Exams = self
            .call(session, self.service(Method::GET, &["me", "exams"])?)
            .await?;
        Ok((exams, submitted.exams))
    }

    async fn open(
        &self,
        session: &mut Session,
        id: &str,
        download: bool,
    ) -> Result<ExamLocalDocument, ExamError> {
        let request = self.service(Method::GET, &["exams", id])?;
        let exam: Result<OneExam, _> = self.call(session, request).await;
        let saved = session.saved.as_ref().ok_or(ExamError::SignedOut)?;
        let registered = saved.registered;
        let base = self.account_folder(&saved.uid).join(key(id));
        let cached = cached_file(&base).await;
        let remote = match exam {
            Ok(OneExam { exam: Some(remote) }) if remote.document.id == id => remote,
            Ok(OneExam { exam: None }) | Err(ExamError::NotFound) => {
                erase(&base).await?;
                return Err(ExamError::NotFound);
            }
            Ok(_) => return Err(ExamError::Response),
            Err(ExamError::Unavailable | ExamError::Network | ExamError::Timeout)
                if cached.is_some() && registered && !download =>
            {
                let cached = cached.ok_or(ExamError::Storage)?;
                return local(&base, cached, true, None);
            }
            Err(error) => return Err(error),
        };
        let mut metadata;
        if let Some(existing) = cached
            .as_ref()
            .filter(|existing| existing.updated_at == remote.updated_at)
        {
            metadata = Cached {
                document: remote.document.clone(),
                updated_at: existing.updated_at,
                downloaded: existing.downloaded,
                file: existing.file,
            };
            if download && !metadata.downloaded {
                metadata.downloaded = true;
                save_metadata(&base, &metadata).await?;
            }
        } else {
            let url: Result<ReadUrl, _> = self
                .call(
                    session,
                    self.service(Method::POST, &["files", "read-url"])?
                        .json(&json!({"storagePath": remote.storage_path})),
                )
                .await;
            let url = match url {
                Err(ExamError::NotFound) => {
                    erase(&base).await?;
                    return Err(ExamError::NotFound);
                }
                Err(error) if error.temporarily_unavailable() && !download && registered => {
                    return cached.map_or(Err(error), |cached| local(&base, cached, true, None));
                }
                result => result?,
            };
            let downloaded =
                download || cached.as_ref().is_some_and(|existing| existing.downloaded);
            let file = uuid::Uuid::new_v4();
            metadata = Cached {
                document: remote.document,
                updated_at: remote.updated_at,
                downloaded,
                file,
            };
            let path = file_path(&base, &metadata)?;
            tokio::fs::create_dir_all(&base)
                .await
                .map_err(|_| ExamError::Storage)?;
            let mut keep = TempFile(path.clone());
            match self
                .fetch(
                    url::Url::parse(&url.url).map_err(|_| ExamError::Response)?,
                    &path,
                )
                .await
            {
                Err(ExamError::NotFound) => {
                    erase(&base).await?;
                    return Err(ExamError::NotFound);
                }
                Err(error) if error.temporarily_unavailable() && !download && registered => {
                    return cached.map_or(Err(error), |cached| local(&base, cached, true, None));
                }
                result => result?,
            }
            save_metadata(&base, &metadata).await?;
            keep.0.clear();
            let mut entries = tokio::fs::read_dir(&base)
                .await
                .map_err(|_| ExamError::Storage)?;
            while let Some(entry) = entries.next_entry().await.map_err(|_| ExamError::Storage)? {
                if entry.path() != path && entry.file_name() != "metadata.json" {
                    let _ = tokio::fs::remove_file(entry.path()).await;
                }
            }
        }
        let saved: Result<Saved, _> = self
            .call(session, self.service(Method::GET, &["me", "saved-exams"])?)
            .await;
        let saved = match saved {
            Ok(answer) => Some(answer.exam_ids.iter().any(|value| value == id)),
            Err(error) if error.temporarily_unavailable() => None,
            Err(error) => return Err(error),
        };
        let _ = self
            .call::<Value>(session, self.service(Method::POST, &["me", "views", id])?)
            .await;
        local(&base, metadata, false, saved)
    }

    async fn fetch(&self, url: url::Url, path: &Path) -> Result<(), ExamError> {
        const LIMIT: usize = 25 * 1024 * 1024;
        if !client::is_file_url(&url) {
            return Err(ExamError::Response);
        }
        let mut response = self
            .api
            .http
            .get(url)
            .send()
            .await
            .map_err(client::network)?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(ExamError::NotFound);
        }
        if !response.status().is_success() {
            return Err(ExamError::Unavailable);
        }
        if response
            .content_length()
            .is_some_and(|len| len > LIMIT as u64)
        {
            return Err(ExamError::Response);
        }
        let mut stream = tokio::fs::File::create(path)
            .await
            .map_err(|_| ExamError::Storage)?;
        let mut written = 0;
        while let Some(chunk) = response.chunk().await.map_err(client::network)? {
            written += chunk.len();
            if written > LIMIT {
                return Err(ExamError::Response);
            }
            stream
                .write_all(&chunk)
                .await
                .map_err(|_| ExamError::Storage)?;
        }
        stream.flush().await.map_err(|_| ExamError::Storage)
    }
}

fn file_path(base: &Path, cached: &Cached) -> Result<PathBuf, ExamError> {
    let suffix = match cached.document.file_type.as_str() {
        "application/pdf" => "pdf",
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "text/plain" => "txt",
        _ => return Err(ExamError::Response),
    };
    Ok(base.join(format!("{}.{suffix}", cached.file)))
}

fn local(
    base: &Path,
    cached: Cached,
    offline: bool,
    saved: Option<bool>,
) -> Result<ExamLocalDocument, ExamError> {
    let path = file_path(base, &cached)?
        .to_str()
        .ok_or(ExamError::Storage)?
        .to_owned();
    Ok(ExamLocalDocument {
        document: cached.document,
        path,
        offline,
        downloaded: cached.downloaded,
        saved,
    })
}

async fn cached_file(base: &Path) -> Option<Cached> {
    let body = tokio::fs::read(base.join("metadata.json")).await.ok()?;
    let value: Cached = serde_json::from_slice(&body).ok()?;
    tokio::fs::metadata(file_path(base, &value).ok()?)
        .await
        .ok()?
        .is_file()
        .then_some(value)
}

async fn save_metadata(base: &Path, cached: &Cached) -> Result<(), ExamError> {
    let temp = TempFile(base.join("metadata-new.json"));
    let data = serde_json::to_vec(cached).map_err(|_| ExamError::Storage)?;
    tokio::fs::write(&temp.0, data)
        .await
        .map_err(|_| ExamError::Storage)?;
    tokio::fs::rename(&temp.0, base.join("metadata.json"))
        .await
        .map_err(|_| ExamError::Storage)
}

async fn erase(base: &Path) -> Result<(), ExamError> {
    match tokio::fs::remove_dir_all(base).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(ExamError::Storage),
    }
}

async fn downloads(base: &Path) -> Result<Vec<ExamDocument>, ExamError> {
    let mut entries = match tokio::fs::read_dir(base).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(_) => return Err(ExamError::Storage),
    };
    let mut list = vec![];
    while let Some(entry) = entries.next_entry().await.map_err(|_| ExamError::Storage)? {
        if let Some(cached) = cached_file(&entry.path())
            .await
            .filter(|item| item.downloaded)
        {
            list.push(cached.document);
        }
    }
    list.sort_by(|a, b| a.file_name.cmp(&b.file_name).then_with(|| a.id.cmp(&b.id)));
    Ok(list)
}

struct TempFile(PathBuf);
impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
