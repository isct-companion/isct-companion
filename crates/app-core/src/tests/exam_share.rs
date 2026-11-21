use super::*;
use crate::ports::{EventError, SecretError};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use std::{collections::HashMap, sync::Mutex as StdMutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

// ScienceTokyoExam and Firebase have no test mode, so these tests answer the
// App API's documented requests and error codes from a local listener the
// client is pointed at.

#[derive(Default)]
struct Vault(StdMutex<HashMap<String, String>>);
impl SecretVault for Vault {
    fn load(&self, account: &str) -> Result<Option<String>, SecretError> {
        Ok(self.0.lock().unwrap().get(account).cloned())
    }
    fn store(&self, account: &str, secret: &str) -> Result<(), SecretError> {
        self.0.lock().unwrap().insert(account.into(), secret.into());
        Ok(())
    }
    fn delete(&self, account: &str) -> Result<(), SecretError> {
        self.0.lock().unwrap().remove(account);
        Ok(())
    }
}

#[derive(Default)]
struct Events(StdMutex<Vec<ExamAccount>>);
impl EventSink for Events {
    fn emit(&self, event: CoreEvent) -> Result<(), EventError> {
        if let CoreEvent::ExamAccount(account) = event {
            self.0.lock().unwrap().push(account);
        }
        Ok(())
    }
}

type Reply = (u16, String);
type Route = Box<dyn Fn(&str, &str) -> Reply + Send + Sync>;

/// A one-request-per-connection HTTP listener answering from `route`, and
/// the requests it saw as "METHOD /path".
struct Api {
    origin: url::Url,
    seen: Arc<StdMutex<Vec<String>>>,
    route: Arc<StdMutex<Route>>,
}

impl Api {
    async fn start(route: Route) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin =
            url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let route = Arc::new(StdMutex::new(route));
        let (log, answer) = (seen.clone(), route.clone());
        tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut data = Vec::new();
                let mut buffer = [0; 8192];
                // Read the head, then the body its Content-Length announces.
                let (head, body_start) = loop {
                    let read = stream.read(&mut buffer).await.unwrap();
                    data.extend_from_slice(&buffer[..read]);
                    if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        break (String::from_utf8_lossy(&data[..end]).into_owned(), end + 4);
                    }
                };
                let length = head
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                while data.len() < body_start + length {
                    let read = stream.read(&mut buffer).await.unwrap();
                    data.extend_from_slice(&buffer[..read]);
                }
                let mut line = head.lines().next().unwrap().split(' ');
                let method = line.next().unwrap().to_owned();
                let path = line.next().unwrap().split('?').next().unwrap().to_owned();
                log.lock().unwrap().push(format!("{method} {path}"));
                let (status, body) = (answer.lock().unwrap())(&method, &path);
                let reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            }
        });
        Self {
            origin,
            seen,
            route,
        }
    }

    fn answer(&self, route: Route) {
        *self.route.lock().unwrap() = route;
    }

    fn count(&self, request: &str) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|seen| *seen == request)
            .count()
    }
}

/// A Firebase ID token for `uid`; Core reads its claims, Firebase signs it.
fn id_token(uid: &str, claims: serde_json::Value) -> String {
    let mut body = json!({
        "sub": uid,
        "aud": client::FIREBASE_PROJECT,
        "iss": format!("https://securetoken.google.com/{}", client::FIREBASE_PROJECT),
        "exp": chrono::Utc::now().timestamp() + 3600,
        "allowed": true,
    });
    body.as_object_mut()
        .unwrap()
        .extend(claims.as_object().unwrap().clone());
    format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(body.to_string()))
}

fn refreshed(uid: &str) -> Reply {
    (
        200,
        json!({"user_id": uid, "refresh_token": "r2", "id_token": id_token(uid, json!({}))})
            .to_string(),
    )
}

struct Fixture {
    root: PathBuf,
    vault: Arc<Vault>,
    events: Arc<Events>,
    share: ExamShare,
    store: DataStore,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A registered account `u1` left by an earlier run, served by `api`.
fn fixture(api: &Api, name: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("isct-exam-{name}-{}", uuid::Uuid::new_v4()));
    let (store, _) = DataStore::initialize_dir(root.join("data")).unwrap();
    let vault = Arc::new(Vault::default());
    vault
        .store(
            ACCOUNT,
            &json!({"uid": "u1", "email": "s@m.isct.ac.jp", "refresh_token": "r1", "registered": true})
                .to_string(),
        )
        .unwrap();
    let events = Arc::new(Events::default());
    let folder = root.join("exam-share");
    let mut share =
        ExamShare::new(vault.clone(), folder, "9.0.0".into(), None, events.clone()).unwrap();
    share.api.origin = Some(api.origin.clone());
    Fixture {
        root,
        vault,
        events,
        share,
        store,
    }
}

impl Fixture {
    async fn act(&self, action: ExamAction) -> Result<ExamAnswer, ExamError> {
        self.share
            .act(action, CancellationToken::new(), &self.store)
            .await
    }

    fn account_folder(&self) -> PathBuf {
        self.share.account_folder("u1")
    }

    fn last_event(&self) -> ExamAccount {
        self.events.0.lock().unwrap().last().cloned().unwrap()
    }
}

fn status(answer: Result<ExamAnswer, ExamError>) -> ExamAccountStatus {
    match answer {
        Ok(ExamAnswer::Account(account)) => account.status,
        other => panic!("expected the account, got {other:?}"),
    }
}

/// An outage keeps a registered account and its downloads; an expired
/// refresh token signs it out and removes them, and shells hear of it.
#[tokio::test]
async fn an_outage_keeps_the_account_and_an_expired_grant_clears_it() {
    let api = Api::start(Box::new(|_, _| (503, "{}".into()))).await;
    let fixture = fixture(&api, "session");
    std::fs::create_dir_all(fixture.account_folder()).unwrap();
    std::fs::write(fixture.account_folder().join("copy.pdf"), b"%PDF").unwrap();

    assert_eq!(
        status(fixture.act(ExamAction::Account).await),
        ExamAccountStatus::Connected
    );
    assert!(fixture.vault.load(ACCOUNT).unwrap().is_some());
    assert!(fixture.account_folder().join("copy.pdf").is_file());

    api.answer(Box::new(|method, path| match (method, path) {
        ("GET", "/api/client-config") => (200, "{}".into()),
        ("POST", "/v1/token") => (
            400,
            json!({"error": {"message": "INVALID_REFRESH_TOKEN"}}).to_string(),
        ),
        _ => (500, "{}".into()),
    }));
    assert_eq!(
        status(fixture.act(ExamAction::Account).await),
        ExamAccountStatus::Disconnected
    );
    assert!(fixture.vault.load(ACCOUNT).unwrap().is_none());
    assert!(!fixture.account_folder().exists());
    let shown = fixture.last_event();
    assert_eq!(
        (shown.status, shown.email),
        (ExamAccountStatus::Disconnected, None)
    );
}

fn connected(method: &str, path: &str) -> Option<Reply> {
    match (method, path) {
        ("GET", "/api/client-config") => Some((200, "{}".into())),
        ("POST", "/v1/token") => Some(refreshed("u1")),
        ("GET", "/api/me") => Some((200, json!({"profile": {"role": "student"}}).to_string())),
        _ => None,
    }
}

fn journal(fixture: &Fixture, phase: &str, retry_after: i64) {
    let draft = json!({
        "subjectCodes": ["LAS.B103"], "teacherIds": [], "newTeachers": [], "academicYear": 2025,
        "occasion": "final", "occasionNumber": 0, "variant": "", "role": "question",
        "provenance": "original", "answerSource": null, "note": "", "requestId": null,
    });
    let mut registration = draft.as_object().unwrap().clone();
    registration.extend(
        json!({"storagePath": "exams/u1/key", "fileName": "a.pdf", "fileSize": 4,
            "fileType": "application/pdf", "fileHash": "hash"})
        .as_object()
        .unwrap()
        .clone(),
    );
    let pending = json!({"registration": registration, "phase": phase, "retry_after": retry_after});
    std::fs::create_dir_all(fixture.share.folder.clone()).unwrap();
    std::fs::write(fixture.share.upload_journal("u1"), pending.to_string()).unwrap();
}

fn journal_phase(fixture: &Fixture) -> Option<String> {
    let path = fixture.share.upload_journal("u1");
    path.is_file().then(|| {
        let saved: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        saved["phase"].as_str().unwrap().to_owned()
    })
}

/// The upload journal is what keeps a transferred file from being lost or
/// published twice: an unanswered registration waits, one the service
/// already holds counts as done, and a rollback the service refuses
/// because the file is in use turns back into a registration.
#[tokio::test]
async fn a_recovered_upload_is_registered_once_and_never_dropped() {
    let api = Api::start(Box::new(|method, path| {
        connected(method, path).unwrap_or((503, "{}".into()))
    }))
    .await;
    let fixture = fixture(&api, "upload");
    journal(&fixture, "Register", 0);

    assert!(matches!(
        fixture.act(ExamAction::RecoverUpload).await,
        Err(ExamError::UploadPending)
    ));
    assert_eq!(journal_phase(&fixture).as_deref(), Some("Register"));
    assert_eq!(fixture.last_event().upload.phase, ExamUploadPhase::Pending);
    // Waiting out the retry delay asks the service nothing.
    let registrations = api.count("POST /api/exams");
    assert!(matches!(
        fixture.act(ExamAction::RecoverUpload).await,
        Err(ExamError::UploadPending)
    ));
    assert_eq!(api.count("POST /api/exams"), registrations);

    journal(&fixture, "Register", 0);
    api.answer(Box::new(|method, path| {
        connected(method, path).unwrap_or_else(|| match (method, path) {
            ("POST", "/api/exams") => (
                409,
                json!({"code": "upload.already_registered"}).to_string(),
            ),
            _ => (500, "{}".into()),
        })
    }));
    // An attempt whose answer was lost has no id to hand the shell.
    assert!(matches!(
        fixture.act(ExamAction::RecoverUpload).await,
        Ok(ExamAnswer::Uploaded(None))
    ));
    assert_eq!(journal_phase(&fixture), None);
    assert_eq!(fixture.last_event().upload.phase, ExamUploadPhase::Idle);

    // One that registers it hands back the new document, to open.
    journal(&fixture, "Register", 0);
    api.answer(Box::new(|method, path| {
        connected(method, path).unwrap_or_else(|| match (method, path) {
            ("POST", "/api/exams") => (200, json!({"examId": "exam-new"}).to_string()),
            _ => (500, "{}".into()),
        })
    }));
    assert!(matches!(
        fixture.act(ExamAction::RecoverUpload).await,
        Ok(ExamAnswer::Uploaded(Some(id))) if id == "exam-new"
    ));
    assert_eq!(journal_phase(&fixture), None);

    journal(&fixture, "Transfer", 0);
    api.answer(Box::new(|method, path| {
        connected(method, path).unwrap_or_else(|| match (method, path) {
            ("POST", "/api/exams/uploads/rollback") => {
                (409, json!({"code": "upload.rollback_in_use"}).to_string())
            }
            _ => (500, "{}".into()),
        })
    }));
    assert!(matches!(
        fixture.act(ExamAction::RecoverUpload).await,
        Err(ExamError::UploadPending)
    ));
    assert_eq!(journal_phase(&fixture).as_deref(), Some("Register"));
}

fn draft(role: ExamRole, occasion: ExamOccasion) -> ExamUploadDraft {
    ExamUploadDraft {
        subject_codes: vec!["LAS.B103".into()],
        teacher_ids: vec![],
        new_teachers: vec![],
        academic_year: Some(2025),
        occasion,
        occasion_number: 3,
        variant: String::new(),
        role,
        provenance: ExamProvenance::Original,
        answer_source: None,
        note: String::new(),
        request_id: None,
    }
}

/// The API accepts an occasion only for exam materials, a number only for
/// numbered occasions and an answer source only for answers; the forms keep
/// a draft that way and an upload is checked against the API's limits.
#[test]
fn a_draft_follows_its_role_and_the_api_limits() {
    let mut notes = draft(ExamRole::Notes, ExamOccasion::Quiz);
    upload::validate(&mut notes).unwrap();
    assert_eq!(
        (notes.occasion, notes.occasion_number, notes.answer_source),
        (ExamOccasion::NotApplicable, 0, None)
    );

    let mut answer = draft(ExamRole::Answer, ExamOccasion::NotApplicable);
    upload::validate(&mut answer).unwrap();
    assert_eq!(
        (
            answer.occasion,
            answer.occasion_number,
            answer.answer_source
        ),
        (ExamOccasion::Unknown, 0, Some(ExamAnswerSource::Unknown))
    );

    let mut quiz = draft(ExamRole::Question, ExamOccasion::Quiz);
    upload::validate(&mut quiz).unwrap();
    assert_eq!(quiz.occasion_number, 3);

    let mut crowded = draft(ExamRole::Question, ExamOccasion::Final);
    crowded.subject_codes = (0..=form::limits().subjects)
        .map(|n| format!("LAS.B1{n:02}"))
        .collect();
    assert!(matches!(
        upload::validate(&mut crowded),
        Err(ExamError::InvalidInput)
    ));
    let mut long_note = draft(ExamRole::Question, ExamOccasion::Final);
    long_note.note = "x".repeat(41);
    assert!(matches!(
        upload::validate(&mut long_note),
        Err(ExamError::UploadNoteLength)
    ));
}

/// How the App API's statuses and error codes reach the person: each must
/// land on the error a shell shows the right next step for.
#[tokio::test]
async fn service_answers_map_to_what_the_person_does_next() {
    let cases: [(&str, u16, Value); 9] = [
        ("expired", 401, json!({})),
        ("subject", 404, json!({"code": "course.not_found"})),
        ("missing", 404, json!({})),
        ("consent", 403, json!({"code": "auth.not_registered"})),
        ("denied", 403, json!({"code": "auth.not_allowed"})),
        ("busy", 429, json!({"code": "rate.too_many"})),
        ("down", 503, json!({})),
        ("field", 400, json!({"code": "upload.variant_too_long"})),
        ("other", 409, json!({"code": "something.else"})),
    ];
    let answers: HashMap<String, (u16, String)> = cases
        .iter()
        .map(|(name, status, body)| (format!("/api/{name}"), (*status, body.to_string())))
        .collect();
    let api = Api::start(Box::new(move |_, path| answers[path].clone())).await;
    let mut client = Client::new("9.0.0".into()).unwrap();
    client.origin = Some(api.origin.clone());
    let mut seen = Vec::new();
    for (name, _, _) in cases {
        let request = client
            .request(client::Endpoint::Site, Method::GET, &["api", name])
            .unwrap();
        seen.push(format!(
            "{:?}",
            client.service::<Value>(request).await.unwrap_err()
        ));
    }
    assert_eq!(
        seen,
        [
            "Expired",
            "SubjectNotFound",
            "NotFound",
            "Consent",
            "Denied",
            "Unavailable",
            "Unavailable",
            "InvalidInput",
            "Rejected"
        ]
    );
}

/// Core trusts a Firebase ID token's claims only for its own project and
/// account, while it is valid; an administrator account is refused.
#[test]
fn only_a_current_token_of_this_project_and_account_is_taken() {
    let saved = || identity::SavedSession {
        uid: "u1".into(),
        email: "s@m.isct.ac.jp".into(),
        refresh_token: "r".into(),
        registered: true,
    };
    let take = |claims: Value| identity::SessionToken::new(saved(), id_token("u1", claims));
    assert!(take(json!({})).is_ok_and(|token| token.allowed));
    for claims in [
        json!({"aud": "another-project"}),
        json!({"iss": "https://securetoken.google.com/another-project"}),
        json!({"sub": "u2"}),
        json!({"exp": chrono::Utc::now().timestamp() - 1}),
    ] {
        assert!(matches!(take(claims), Err(ExamError::Response)));
    }
    assert!(matches!(
        take(json!({"admin": true})),
        Err(ExamError::Administrator)
    ));
}

/// Signed file URLs are fetched only from the storage host, over HTTPS on
/// its own port and without credentials in the URL.
#[test]
fn files_come_only_from_the_storage_host() {
    let client = Client::new("9.0.0".into()).unwrap();
    let file = |url: &str| client.is_file_url(&url::Url::parse(url).unwrap());
    assert!(file(
        "https://bucket.r2.cloudflarestorage.com/exams/u1/key?X-Amz=1"
    ));
    for refused in [
        "http://bucket.r2.cloudflarestorage.com/exams/u1/key",
        "https://bucket.r2.cloudflarestorage.com:8443/exams/u1/key",
        "https://user:pass@bucket.r2.cloudflarestorage.com/exams/u1/key",
        "https://r2.cloudflarestorage.com.example.com/exams/u1/key",
        "https://sciencetokyoexam.com/exams/u1/key",
    ] {
        assert!(!file(refused), "{refused}");
    }
}

#[test]
fn upload_readiness_validates_staged_bytes_and_the_actual_draft() {
    let folder = std::env::temp_dir().join(format!("exam-readiness-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&folder).unwrap();
    let image = folder.join("photo.png");
    image::RgbImage::new(2, 2).save(&image).unwrap();
    let pdf = folder.join("document.jpg");
    std::fs::write(&pdf, b"%PDF-1.7").unwrap();
    let file = |path: &std::path::Path, name: &str| ExamUploadFile {
        path: path.to_string_lossy().into(),
        name: name.into(),
    };
    let photo = file(&image, "misleading.pdf");
    let document = file(&pdf, "misleading.jpg");
    let mut draft = draft(ExamRole::Question, ExamOccasion::Final);
    let ready = |draft: &ExamUploadDraft, files: &[ExamUploadFile], reason: Option<&str>| {
        assert_eq!(upload_readiness(draft, files).reason_key.as_deref(), reason);
    };
    ready(&draft, &[], Some("error.exam-share-upload-no-files"));
    ready(
        &draft,
        &[photo.clone(), document],
        Some("error.exam-share-upload-mixed"),
    );
    ready(&draft, &[photo.clone(), photo.clone()], None);
    draft.new_teachers.push(ExamNewTeacher {
        family_name: " ".into(),
        given_name: "A".into(),
    });
    ready(
        &draft,
        std::slice::from_ref(&photo),
        Some("error.exam-share-upload-teacher-name"),
    );
    draft.new_teachers[0].family_name = "𠮷".repeat(51);
    ready(
        &draft,
        std::slice::from_ref(&photo),
        Some("error.exam-share-upload-teacher-length"),
    );
    draft.new_teachers[0].family_name = "𠮷".repeat(50);
    draft.variant = "𠮷".repeat(51);
    ready(
        &draft,
        std::slice::from_ref(&photo),
        Some("error.exam-share-upload-variant-length"),
    );
    draft.variant.clear();
    draft.note = "𠮷".repeat(21);
    ready(
        &draft,
        &[photo],
        Some("error.exam-share-upload-note-length"),
    );
    std::fs::remove_dir_all(folder).unwrap();
}
