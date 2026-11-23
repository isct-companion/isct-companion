use super::cache::{
    cache_file, downloads, erase, file_path, local, save_metadata, Cached, TempFile,
};
use super::remote::{has_class, present_all, RemoteDocument, RemoteSubject, SubjectRecords};
use super::*;
use crate::locale::ContentLanguage;
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteExam {
    #[serde(flatten)]
    document: RemoteDocument,
    storage_path: String,
    updated_at: u64,
}

fn present_subject(
    records: SubjectRecords,
    class_id: Option<u32>,
    language: ContentLanguage,
) -> Result<ExamSubjectPage, ExamError> {
    let web_url = url::Url::parse_with_params(
        &format!("https://{}/search", client::Endpoint::Site.host()),
        [("course", &records.subject.code)],
    )
    .map_err(|_| ExamError::Response)?;
    Ok(records.present(class_id, language, web_url.into()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubjectDetails {
    #[serde(flatten)]
    page: SubjectRecords,
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
    subject: RemoteSubject,
    teachers: Vec<ExamTeacher>,
    academic_year: Option<u32>,
    occasion: Option<String>,
    wanted_role: Option<String>,
    description: String,
    status: String,
    answer_exam_id: Option<String>,
}

impl RemoteRequest {
    fn model(self, uid: &str, language: ContentLanguage) -> ExamRequest {
        let r = self.request;
        ExamRequest {
            id: r.id,
            subject: r.subject.present(language),
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
    exams: Vec<RemoteDocument>,
}
#[derive(Deserialize)]
struct OneExam {
    exam: Option<RemoteExam>,
}
#[derive(Deserialize)]
struct Subjects {
    subjects: Vec<SubjectHit>,
}

#[derive(Deserialize)]
struct SubjectHit {
    #[serde(flatten)]
    subject: RemoteSubject,
    rows: Vec<ExamClass>,
}

struct SubjectCandidate {
    subject: RemoteSubject,
    class_matches: bool,
    page: Option<SubjectRecords>,
}

fn course_candidates(candidates: BTreeMap<String, SubjectCandidate>) -> Vec<SubjectCandidate> {
    let class_matches = candidates.values().any(|candidate| candidate.class_matches);
    let direct_matches = candidates
        .values()
        .any(|candidate| candidate.page.is_some());
    candidates
        .into_values()
        .filter(|candidate| {
            if class_matches {
                candidate.class_matches
            } else {
                !direct_matches || candidate.page.is_some()
            }
        })
        .collect()
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

impl ExamShare {
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

    async fn search_subjects(
        &self,
        session: &mut Session,
        query: &str,
        limit: &str,
    ) -> Result<Vec<SubjectHit>, ExamError> {
        let answer: Subjects = self
            .call(
                session,
                self.service(Method::GET, &["subjects", "search"])?
                    .query(&[("q", query), ("limit", limit)]),
            )
            .await?;
        Ok(answer.subjects)
    }

    pub(super) async fn resolve_subjects(
        &self,
        session: &mut Session,
        codes: Vec<String>,
        class_id: Option<u32>,
        language: ContentLanguage,
    ) -> Result<ExamAnswer, ExamError> {
        let mut candidates = BTreeMap::new();
        for code in codes {
            match self.subject_details(session, &code).await {
                Ok(found) => {
                    let page = found.page;
                    candidates.insert(
                        page.subject.code.clone(),
                        SubjectCandidate {
                            subject: page.subject.clone(),
                            class_matches: has_class(&page.rows, class_id),
                            page: Some(page),
                        },
                    );
                }
                // The production contract points clients to search on this
                // specific 404. Its ordering does not establish identity.
                Err(ExamError::SubjectNotFound) => {
                    for hit in self.search_subjects(session, &code, "50").await? {
                        let matches = has_class(&hit.rows, class_id);
                        candidates
                            .entry(hit.subject.code.clone())
                            .and_modify(|candidate| candidate.class_matches |= matches)
                            .or_insert(SubjectCandidate {
                                subject: hit.subject,
                                page: None,
                                class_matches: matches,
                            });
                    }
                }
                Err(ExamError::NotFound) => {}
                Err(error) => return Err(error),
            }
        }
        let mut candidates = course_candidates(candidates);
        if candidates.len() == 1 {
            let candidate = candidates.pop().ok_or(ExamError::Response)?;
            let page = match candidate.page {
                Some(page) => page,
                None => {
                    self.subject_details(session, &candidate.subject.code)
                        .await?
                        .page
                }
            };
            return present_subject(page, class_id, language).map(ExamAnswer::Subject);
        }
        Ok(ExamAnswer::Subjects(
            candidates
                .into_iter()
                .map(|choice| choice.subject.present(language))
                .collect(),
        ))
    }

    pub(super) async fn search(
        &self,
        session: &mut Session,
        query: &str,
        language: ContentLanguage,
    ) -> Result<Vec<ExamSubject>, ExamError> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(vec![]);
        }
        if query.encode_utf16().count() > form::limits().query as usize {
            return Err(ExamError::InvalidInput);
        }
        let matches = self.search_subjects(session, query, "20").await?;
        Ok(matches
            .into_iter()
            .map(|hit| hit.subject.present(language))
            .collect())
    }

    async fn upload_subject(
        &self,
        session: &mut Session,
        code: &str,
        language: ContentLanguage,
    ) -> Result<ExamUploadSubject, ExamError> {
        let details = self.subject_details(session, code).await?;
        Ok(ExamUploadSubject {
            removable: false,
            subject: details.page.subject.present(language),
            teachers: form::distinct_teachers(
                details
                    .current_teachers
                    .iter()
                    .chain(&details.past_teachers),
            ),
        })
    }

    pub(super) async fn upload_form(
        &self,
        session: &mut Session,
        start: ExamUploadStart,
        language: ContentLanguage,
    ) -> Result<ExamUploadForm, ExamError> {
        let (code, request) = match start {
            ExamUploadStart::Subject { code } => (code, None),
            ExamUploadStart::Request { request } if request.status == "open" => {
                (request.subject.code.clone(), Some(request))
            }
            ExamUploadStart::Request { .. } => return Err(ExamError::InvalidInput),
        };
        let subject = self.upload_subject(session, &code, language).await?;
        let role = form::role(
            request
                .as_ref()
                .and_then(|item| item.wanted_role.as_deref()),
        );
        let mut form = ExamUploadForm {
            draft: ExamUploadDraft {
                subject_codes: vec![],
                teacher_ids: request
                    .iter()
                    .flat_map(|item| &item.teachers)
                    .map(|teacher| teacher.id.clone())
                    .collect(),
                new_teachers: vec![],
                academic_year: request.as_ref().and_then(|item| item.academic_year),
                occasion: form::occasion(
                    request.as_ref().and_then(|item| item.occasion.as_deref()),
                ),
                occasion_number: 0,
                variant: String::new(),
                role,
                provenance: ExamProvenance::Original,
                answer_source: None,
                note: String::new(),
                request_id: request.as_ref().map(|item| item.id.clone()),
            },
            subjects: vec![subject],
            teachers: vec![],
            request,
        };
        form::settle(&mut form);
        Ok(form)
    }

    pub(super) async fn edit_upload(
        &self,
        session: &mut Session,
        mut form: ExamUploadForm,
        edit: ExamUploadEdit,
        language: ContentLanguage,
    ) -> Result<ExamUploadForm, ExamError> {
        match edit {
            ExamUploadEdit::AddSubject { code } => {
                let subject = self.upload_subject(session, &code, language).await?;
                form::add_subject(&mut form, subject)?;
            }
            ExamUploadEdit::RemoveSubject { code } => form::remove_subject(&mut form, &code)?,
            ExamUploadEdit::Role { role } => {
                form.draft.role = role;
                form::normalize(&mut form.draft);
            }
        }
        Ok(form)
    }

    pub(super) async fn remove_download(
        &self,
        session: &mut Session,
        id: &str,
        language: ContentLanguage,
    ) -> Result<ExamLibrary, ExamError> {
        let uid = session
            .saved
            .as_ref()
            .ok_or(ExamError::SignedOut)?
            .uid
            .clone();
        erase(&self.document_folder(&uid, id)).await?;
        self.library(session, language).await
    }

    pub(super) async fn save(
        &self,
        session: &mut Session,
        id: &str,
        saved: bool,
    ) -> Result<bool, ExamError> {
        let answer: Saved = self
            .call(
                session,
                self.service(Method::POST, &["me", "saved-exams", id])?
                    .json(&json!({"saved": saved})),
            )
            .await?;
        Ok(answer.exam_ids.iter().any(|saved| saved == id))
    }

    pub(super) async fn delete_submission(
        &self,
        session: &mut Session,
        id: &str,
        language: ContentLanguage,
    ) -> Result<ExamLibrary, ExamError> {
        let _: Value = self
            .call(
                session,
                self.service(Method::POST, &["exams", id, "delete"])?,
            )
            .await?;
        let uid = session
            .saved
            .as_ref()
            .ok_or(ExamError::SignedOut)?
            .uid
            .clone();
        erase(&self.document_folder(&uid, id)).await?;
        self.library(session, language).await
    }

    pub(super) async fn report(
        &self,
        session: &mut Session,
        id: &str,
        reason: ExamReportReason,
        detail: &str,
    ) -> Result<(), ExamError> {
        let detail = detail.trim();
        if detail.encode_utf16().count() > form::limits().report_detail as usize {
            return Err(ExamError::InvalidInput);
        }
        let _: Value = self
            .call(
                session,
                self.service(Method::POST, &["exams", id, "report"])?
                    .json(&json!({"reason": reason.api_code(), "detail": detail})),
            )
            .await?;
        Ok(())
    }

    pub(super) async fn requests(
        &self,
        session: &mut Session,
        mine: bool,
        cursor: Option<ExamRequestCursor>,
        language: ContentLanguage,
    ) -> Result<ExamRequestPage, ExamError> {
        let request = if mine {
            self.service(Method::GET, &["me", "requests"])?
        } else {
            self.service(Method::GET, &["requests"])?
        };
        let request = match cursor {
            Some(cursor) => request.query(&[
                ("cursorCreatedAt", cursor.created_at.to_string()),
                ("cursorId", cursor.id),
            ]),
            None => request,
        };
        let answer: Requests = self.call(session, request).await?;
        let uid = &session.saved.as_ref().ok_or(ExamError::SignedOut)?.uid;
        Ok(ExamRequestPage {
            items: answer
                .items
                .into_iter()
                .map(|request| request.model(uid, language))
                .collect(),
            cursor: answer.cursor,
            web_url: client::site_page("request"),
        })
    }

    pub(super) async fn create_request(
        &self,
        session: &mut Session,
        draft: ExamRequestDraft,
    ) -> Result<(), ExamError> {
        if draft.description.encode_utf16().count() > form::limits().request_description as usize
            || draft
                .academic_year
                .is_some_and(|year| !form::valid_year(year))
        {
            return Err(ExamError::InvalidInput);
        }
        let _: Value = self
            .call(
                session,
                self.service(Method::POST, &["requests"])?.json(&draft),
            )
            .await?;
        Ok(())
    }

    pub(super) async fn set_request_open(
        &self,
        session: &mut Session,
        id: &str,
        open: bool,
        mine: bool,
        language: ContentLanguage,
    ) -> Result<ExamRequestPage, ExamError> {
        let status = if open { "open" } else { "closed" };
        let _: Value = self
            .call(
                session,
                self.service(Method::POST, &["requests", id, "status"])?
                    .json(&json!({"status": status})),
            )
            .await?;
        self.requests(session, mine, None, language).await
    }

    pub(super) async fn delete_request(
        &self,
        session: &mut Session,
        id: &str,
        mine: bool,
        language: ContentLanguage,
    ) -> Result<ExamRequestPage, ExamError> {
        let _: Value = self
            .call(
                session,
                self.service(Method::POST, &["requests", id, "delete"])?,
            )
            .await?;
        self.requests(session, mine, None, language).await
    }

    pub(super) async fn library(
        &self,
        session: &mut Session,
        language: ContentLanguage,
    ) -> Result<ExamLibrary, ExamError> {
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
            saved: present_all(saved, language),
            submitted: present_all(submitted, language),
            downloaded: downloads(&self.account_folder(uid), language).await?,
            web_url: client::site_page("library"),
        })
    }

    async fn remote_library(
        &self,
        session: &mut Session,
    ) -> Result<(Vec<RemoteDocument>, Vec<RemoteDocument>), ExamError> {
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

    pub(super) async fn open(
        &self,
        session: &mut Session,
        id: &str,
        download: bool,
        language: ContentLanguage,
    ) -> Result<ExamLocalDocument, ExamError> {
        let request = self.service(Method::GET, &["exams", id])?;
        let exam: Result<OneExam, _> = self.call(session, request).await;
        let saved = session.saved.as_ref().ok_or(ExamError::SignedOut)?;
        let registered = saved.registered;
        let base = self.document_folder(&saved.uid, id);
        let cached = cache_file(&base).await?;
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
                return local(&base, cached, true, None, language).await;
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
                    return match cached {
                        Some(cached) => local(&base, cached, true, None, language).await,
                        None => Err(error),
                    };
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
                .api
                .fetch_file(
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
                    return match cached {
                        Some(cached) => local(&base, cached, true, None, language).await,
                        None => Err(error),
                    };
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
        local(&base, metadata, false, saved, language).await
    }
}

#[cfg(test)]
#[path = "../tests/exam_share__library.rs"]
mod tests;
