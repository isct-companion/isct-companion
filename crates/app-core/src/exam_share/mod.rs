//! Native client for the ScienceTokyoExam sharing service.
mod cache;
mod client;
mod form;
mod google;
mod identity;
mod library;
mod model;
mod remote;
mod sets;
mod upload;
mod upload_file;
pub use model::*;

use crate::oauth::{code_exchange::OAuthClient, NativeBrowserAuth};
use crate::{
    data::DataStore,
    events::CoreEvent,
    oauth::OAuthError,
    ports::{EventSink, SecretVault},
};
use client::Client;
use identity::{SavedSession, SessionToken};
use reqwest::Method;
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

const ACCOUNT: &str = "sciencetokyoexam";
// Match client-config's public Cache-Control max-age.
const VERSION_MAX_AGE: Duration = Duration::from_secs(300);
const VERSION_RETRY: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
pub enum ExamError {
    #[error("The ScienceTokyoExam request failed at the transport layer")]
    Network,
    #[error("The ScienceTokyoExam request timed out")]
    Timeout,
    #[error("The ScienceTokyoExam service is temporarily unavailable")]
    Unavailable,
    #[error("The ScienceTokyoExam API is incompatible with this app version")]
    Unsupported,
    #[error("ScienceTokyoExam returned an invalid response")]
    Response,
    #[error("No ScienceTokyoExam account is connected")]
    SignedOut,
    #[error("The ScienceTokyoExam authorization expired")]
    Expired,
    #[error("The ScienceTokyoExam account is not allowed")]
    Denied,
    #[error("The ScienceTokyoExam account needs to accept the terms")]
    Consent,
    #[error("The ScienceTokyoExam account has administrator privileges")]
    Administrator,
    #[error("The ScienceTokyoExam item is missing")]
    NotFound,
    #[error("The ScienceTokyoExam subject is missing")]
    SubjectNotFound,
    #[error("ScienceTokyoExam rejected the action")]
    Rejected,
    #[error("The Google identity could not be verified")]
    Identity,
    #[error("ScienceTokyoExam local storage could not be accessed")]
    Storage,
    #[error("The ScienceTokyoExam operation was cancelled")]
    Cancelled,
    #[error("This ScienceTokyoExam document has already been reported")]
    AlreadyReported,
    #[error("The ScienceTokyoExam daily request limit was reached")]
    Quota,
    #[error("The ScienceTokyoExam input is invalid")]
    InvalidInput,
    #[error("The selected upload files could not be prepared")]
    UploadFile,
    #[error("Only images can be combined into one upload")]
    UploadMixed,
    #[error("The upload exceeds the file size limit")]
    UploadSize,
    #[error("ScienceTokyoExam already has this material")]
    UploadDuplicate,
    #[error("The ScienceTokyoExam upload limit was reached")]
    UploadQuota,
    #[error("The upload result needs to be recovered")]
    UploadPending,
    #[error("The uploaded file is still referenced")]
    UploadInUse,
    #[error("The upload is already registered")]
    UploadRegistered,
}

impl ExamError {
    fn temporarily_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable | Self::Network | Self::Timeout)
    }
}

impl From<OAuthError> for ExamError {
    fn from(error: OAuthError) -> Self {
        match error {
            OAuthError::Cancelled => Self::Cancelled,
            OAuthError::Timeout => Self::Timeout,
            OAuthError::Configuration => Self::Unavailable,
            _ => Self::Identity,
        }
    }
}

/// Run `work` unless `cancel` fires first.
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

/// The account as shells last saw it. A change, upload progress included,
/// goes out as `CoreEvent::ExamAccount`, so shells never poll for it.
pub(super) struct Publisher {
    events: Arc<dyn EventSink>,
    shown: std::sync::Mutex<ExamAccount>,
}

impl Publisher {
    fn current(&self) -> ExamAccount {
        self.shown
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn session(&self, session: &Session) {
        self.change(|shown| {
            shown.status = session.status.clone();
            shown.email = session.saved.as_ref().map(|saved| saved.email.clone());
        });
    }

    fn upload(&self, upload: ExamUploadStatus) {
        self.change(|shown| shown.upload = upload);
    }

    fn change(&self, edit: impl FnOnce(&mut ExamAccount)) {
        let mut shown = self
            .shown
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let before = shown.clone();
        edit(&mut shown);
        if *shown != before {
            // Events are hints: a shell reads the account when it shows it.
            let _ = self.events.emit(CoreEvent::ExamAccount(shown.clone()));
        }
    }
}

/// What a shell provides to connect: the OAuth client and the browser port.
pub struct ExamSignIn {
    pub client: OAuthClient,
    pub browser: Arc<dyn NativeBrowserAuth>,
}

struct Session {
    saved: Option<SavedSession>,
    token: Option<SessionToken>,
    status: ExamAccountStatus,
    version_check: Option<VersionCheck>,
}

pub struct ExamShare {
    api: Client,
    version: String,
    vault: Arc<dyn SecretVault>,
    folder: PathBuf,
    session: Mutex<Session>,
    /// Absent when this build has no OAuth client configured.
    sign_in: Option<ExamSignIn>,
    upload: upload::UploadTracker,
    publisher: Arc<Publisher>,
    // Cancellation travels with a connection generation. Disconnect cancels
    // the generation before waiting for the gate to clear the vault.
    generation: std::sync::Mutex<CancellationToken>,
}

impl ExamShare {
    pub(crate) fn new(
        vault: Arc<dyn SecretVault>,
        folder: PathBuf,
        version: String,
        sign_in: Option<ExamSignIn>,
        events: Arc<dyn EventSink>,
    ) -> Result<Self, ExamError> {
        let api = Client::new(version.clone())?;
        let saved = Self::stored_session(vault.as_ref(), &folder)?;
        let status = if saved
            .as_ref()
            .is_some_and(|session: &SavedSession| session.registered)
        {
            ExamAccountStatus::Connected
        } else {
            ExamAccountStatus::Disconnected
        };
        let session = Session {
            saved,
            token: None,
            status,
            version_check: None,
        };
        let upload = upload::settled_status(upload::pending_upload(&folder, &session));
        let publisher = Arc::new(Publisher {
            events,
            shown: std::sync::Mutex::new(ExamAccount {
                status: session.status.clone(),
                email: session.saved.as_ref().map(|saved| saved.email.clone()),
                upload,
            }),
        });
        Ok(Self {
            api,
            version,
            vault,
            folder,
            session: Mutex::new(session),
            sign_in,
            upload: upload::UploadTracker::new(publisher.clone(), upload),
            publisher,
            generation: std::sync::Mutex::new(CancellationToken::new()),
        })
    }

    #[cfg(target_os = "ios")]
    fn stored_session(
        vault: &dyn SecretVault,
        folder: &std::path::Path,
    ) -> Result<Option<SavedSession>, ExamError> {
        if folder.join("installation").is_file() {
            return Self::load_session(vault);
        }
        // iOS keeps Keychain items across a reinstall that removed the data
        // folder; such a session belongs to an earlier installation.
        vault.delete(ACCOUNT).map_err(|_| ExamError::Storage)?;
        std::fs::create_dir_all(folder).map_err(|_| ExamError::Storage)?;
        std::fs::write(folder.join("installation"), []).map_err(|_| ExamError::Storage)?;
        Ok(None)
    }

    #[cfg(not(target_os = "ios"))]
    fn stored_session(
        vault: &dyn SecretVault,
        _folder: &std::path::Path,
    ) -> Result<Option<SavedSession>, ExamError> {
        Self::load_session(vault)
    }

    fn load_session(vault: &dyn SecretVault) -> Result<Option<SavedSession>, ExamError> {
        vault
            .load(ACCOUNT)
            .map_err(|_| ExamError::Storage)?
            .map(|encoded| serde_json::from_str(&encoded))
            .transpose()
            .map_err(|_| ExamError::Storage)
    }

    pub(crate) fn clean_cached_materials(&self, now: SystemTime) -> Result<(), ExamError> {
        let Ok(_session) = self.session.try_lock() else {
            return Ok(());
        };
        cache::clean_root(&self.folder, now)
    }

    pub fn cancel(&self) {
        if let Ok(generation) = self.generation.lock() {
            generation.cancel();
        }
    }

    /// Perform one action. Subject names in the answer follow the content
    /// language setting.
    pub(crate) async fn act(
        &self,
        action: ExamAction,
        cancel: CancellationToken,
        store: &DataStore,
    ) -> Result<ExamAnswer, ExamError> {
        let answer = match action {
            // These answer without waiting for the session gate: the first
            // needs no account, the others stop work that holds the gate.
            ExamAction::Choices => ExamAnswer::Choices(form::choices()),
            ExamAction::CancelUpload => {
                self.upload.cancel()?;
                ExamAnswer::Done
            }
            ExamAction::CancelConnect => {
                let mut generation = self.generation.lock().map_err(|_| ExamError::Storage)?;
                generation.cancel();
                *generation = CancellationToken::new();
                ExamAnswer::Done
            }
            ExamAction::Disconnect => {
                self.cancel();
                let mut session = self.session.lock().await;
                let outcome = self.clear(&mut session).await;
                *self.generation.lock().map_err(|_| ExamError::Storage)? = CancellationToken::new();
                self.publisher.session(&session);
                outcome?;
                ExamAnswer::Account(self.publisher.current())
            }
            action => {
                let generation = self
                    .generation
                    .lock()
                    .map_err(|_| ExamError::Storage)?
                    .clone();
                let operation = generation.child_token();
                let wake = operation.clone();
                // Authorization owns its platform cleanup, so give it a
                // cancellation signal and let it return before releasing the
                // session gate.
                let run = self.run(action, &operation, store);
                tokio::pin!(run);
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => { wake.cancel(); run.await }
                    result = &mut run => result,
                }?
            }
        };
        Ok(answer)
    }

    async fn run(
        &self,
        action: ExamAction,
        cancel: &CancellationToken,
        store: &DataStore,
    ) -> Result<ExamAnswer, ExamError> {
        let mut session = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(ExamError::Cancelled),
            lock = self.session.lock() => lock,
        };
        let result = if matches!(action, ExamAction::Connect) {
            self.connect(&mut session, cancel).await
        } else {
            self.perform(&mut session, action, cancel, store).await
        };
        let settled = self.apply_session_error(&mut session, &result).await;
        self.publisher.session(&session);
        settled?;
        result
    }

    async fn connect(
        &self,
        session: &mut Session,
        cancel: &CancellationToken,
    ) -> Result<ExamAnswer, ExamError> {
        let sign_in = self.sign_in.as_ref().ok_or(ExamError::Unavailable)?;
        self.clear(session).await?;
        self.check_version_for_action(session, cancel).await?;
        let token =
            google::connect(&self.api, &sign_in.client, sign_in.browser.as_ref(), cancel).await?;
        let result = interrupt(cancel, self.establish(session, token)).await;
        if matches!(&result, Err(ExamError::Cancelled)) {
            self.clear(session).await?;
        }
        result?;
        Ok(self.account_answer(session))
    }

    fn account_answer(&self, session: &Session) -> ExamAnswer {
        self.publisher.session(session);
        ExamAnswer::Account(self.publisher.current())
    }

    async fn perform(
        &self,
        session: &mut Session,
        action: ExamAction,
        cancel: &CancellationToken,
        store: &DataStore,
    ) -> Result<ExamAnswer, ExamError> {
        if session.saved.is_some() || matches!(session.status, ExamAccountStatus::Unsupported) {
            self.check_version_for_action(session, cancel).await?;
            if session.saved.is_none() {
                session.status = ExamAccountStatus::Disconnected;
            }
        }
        let answer = match action {
            ExamAction::Upload { files, draft } => {
                return self.upload(session, files, draft, cancel).await;
            }
            ExamAction::RecoverUpload => return self.recover_upload(session, cancel).await,
            ExamAction::Account => return interrupt(cancel, self.account(session)).await,
            ExamAction::AcceptTerms => return interrupt(cancel, self.accept_terms(session)).await,
            action => action,
        };
        let language = remote::content_language(store)?;
        interrupt(cancel, async {
            Ok(match answer {
                ExamAction::Search { query } => {
                    ExamAnswer::Subjects(self.search(session, &query, language).await?)
                }
                ExamAction::Subject { code, class_id } => {
                    self.resolve_subjects(session, vec![code], class_id, language)
                        .await?
                }
                ExamAction::CourseSubject { course_id } => {
                    let (codes, class_id) = course_subject(store, &course_id)?;
                    self.resolve_subjects(session, codes, class_id, language)
                        .await?
                }
                ExamAction::Open { id, download } => {
                    ExamAnswer::Document(self.open(session, &id, download, language).await?)
                }
                ExamAction::Library => ExamAnswer::Library(self.library(session, language).await?),
                ExamAction::Save { id, saved } => {
                    ExamAnswer::Saved(self.save(session, &id, saved).await?)
                }
                ExamAction::RemoveDownload { id } => {
                    ExamAnswer::Library(self.remove_download(session, &id, language).await?)
                }
                ExamAction::DeleteSubmission { id } => {
                    ExamAnswer::Library(self.delete_submission(session, &id, language).await?)
                }
                ExamAction::Report { id, reason, detail } => {
                    self.report(session, &id, reason, &detail).await?;
                    ExamAnswer::Done
                }
                ExamAction::Requests { mine, cursor } => {
                    ExamAnswer::Requests(self.requests(session, mine, cursor, language).await?)
                }
                ExamAction::CreateRequest { draft } => {
                    self.create_request(session, draft).await?;
                    ExamAnswer::Done
                }
                ExamAction::SetRequestOpen { id, open, mine } => {
                    ExamAnswer::Requests(
                    self.set_request_open(session, &id, open, mine, language)
                        .await?,
                )
                }
                ExamAction::DeleteRequest { id, mine } => {
                    ExamAnswer::Requests(self.delete_request(session, &id, mine, language).await?)
                }
                ExamAction::UploadForm { start } => {
                    ExamAnswer::UploadForm(self.upload_form(session, start, language).await?)
                }
                ExamAction::EditUpload { form, edit } => {
                    ExamAnswer::UploadForm(self.edit_upload(session, form, edit, language).await?)
                }
                // `act` and `run` answer these before reaching here.
                ExamAction::Choices
                | ExamAction::CancelUpload
                | ExamAction::CancelConnect
                | ExamAction::Disconnect
                | ExamAction::Connect
                | ExamAction::Upload { .. }
                | ExamAction::RecoverUpload
                | ExamAction::Account
                | ExamAction::AcceptTerms => return Err(ExamError::InvalidInput),
            })
        })
        .await
    }

    async fn account(&self, session: &mut Session) -> Result<ExamAnswer, ExamError> {
        if session.saved.is_some() && session.token.is_none() {
            let result = self.resume(session).await;
            self.apply_session_error(session, &result).await?;
            if let Err(error) = result {
                let offline = error.temporarily_unavailable()
                    && session.saved.as_ref().is_some_and(|saved| saved.registered);
                let signed_out = matches!(
                    error,
                    ExamError::Expired | ExamError::Denied | ExamError::Administrator
                );
                if !offline && !signed_out {
                    return Err(error);
                }
            }
        }
        Ok(self.account_answer(session))
    }

    async fn accept_terms(&self, session: &mut Session) -> Result<ExamAnswer, ExamError> {
        if !matches!(session.status, ExamAccountStatus::NeedsConsent) {
            return Err(ExamError::InvalidInput);
        }
        let current = session.token.as_ref().ok_or(ExamError::SignedOut)?;
        let _: Value = self
            .api
            .service(
                self.service(Method::POST, &["auth", "register"])?
                    .bearer_auth(&current.id_token),
            )
            .await?;
        let updated = identity::refresh(&self.api, &current.saved).await?;
        self.establish(session, updated).await?;
        Ok(self.account_answer(session))
    }

    async fn check_version_for_action(
        &self,
        session: &mut Session,
        cancel: &CancellationToken,
    ) -> Result<(), ExamError> {
        let result = interrupt(cancel, self.check_version(session)).await;
        self.apply_session_error(session, &result).await?;
        result
    }

    async fn check_version(&self, session: &mut Session) -> Result<(), ExamError> {
        let now = SystemTime::now();
        if let Some(check) = session
            .version_check
            .as_ref()
            .filter(|check| check.is_fresh(now))
        {
            return check.result();
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Config {
            min_app_version: Option<String>,
        }
        let config = self
            .api
            .service::<Config>(self.service(Method::GET, &["client-config"])?)
            .await;
        let (compatible, max_age) = match config {
            Ok(config) => (
                compatible_version(&self.version, config.min_app_version.as_deref())?,
                VERSION_MAX_AGE,
            ),
            Err(error)
                if error.temporarily_unavailable()
                    && session.saved.as_ref().is_some_and(|saved| saved.registered) =>
            {
                // A failed refresh does not lock out a previously registered
                // account offline, or override a known minimum that blocked it.
                (
                    session
                        .version_check
                        .as_ref()
                        .is_none_or(|known| known.compatible),
                    VERSION_RETRY,
                )
            }
            Err(error) => return Err(error),
        };
        let check = VersionCheck {
            checked_at: SystemTime::now(),
            compatible,
            max_age,
        };
        let result = check.result();
        session.version_check = Some(check);
        result
    }

    async fn resume(&self, session: &mut Session) -> Result<(), ExamError> {
        let saved = session.saved.as_ref().ok_or(ExamError::SignedOut)?;
        let token = identity::refresh(&self.api, saved).await?;
        self.establish(session, token).await
    }

    async fn establish(
        &self,
        session: &mut Session,
        mut token: SessionToken,
    ) -> Result<(), ExamError> {
        self.remember(session, &token);
        if !token.allowed {
            token = self.refresh_claims(&token).await?;
        }
        self.remember(session, &token);
        #[derive(Deserialize)]
        struct Profile {
            profile: Option<UserRole>,
        }
        #[derive(Deserialize)]
        struct UserRole {
            role: String,
        }
        let answer: Profile = self
            .api
            .service(
                self.service(Method::GET, &["me"])?
                    .bearer_auth(&token.id_token),
            )
            .await?;
        session.status = match answer.profile {
            Some(profile) if profile.role == "admin" => return Err(ExamError::Administrator),
            Some(_) => ExamAccountStatus::Connected,
            None => ExamAccountStatus::NeedsConsent,
        };
        token.saved.registered = matches!(session.status, ExamAccountStatus::Connected);
        self.remember(session, &token);
        session.token = Some(token);
        self.update_upload_account(session);
        Ok(())
    }

    async fn refresh_claims(&self, current: &SessionToken) -> Result<SessionToken, ExamError> {
        #[derive(Deserialize)]
        struct Claim {
            allowed: bool,
        }
        let claim: Claim = self
            .api
            .service(
                self.service(Method::POST, &["auth", "claims", "refresh"])?
                    .bearer_auth(&current.id_token),
            )
            .await?;
        if !claim.allowed {
            return Err(ExamError::Denied);
        }
        let updated = identity::refresh(&self.api, &current.saved).await?;
        if !updated.allowed {
            return Err(ExamError::Denied);
        }
        Ok(updated)
    }

    fn remember(&self, session: &mut Session, token: &SessionToken) {
        session.saved = Some(token.saved.clone());
        if let Ok(data) = serde_json::to_string(&token.saved) {
            // Token rotation remains usable in memory if the keychain is
            // temporarily locked; each successful refresh tries again.
            let _ = self.vault.store(ACCOUNT, &data);
        }
    }

    async fn apply_session_error<T>(
        &self,
        session: &mut Session,
        result: &Result<T, ExamError>,
    ) -> Result<(), ExamError> {
        match result {
            Err(ExamError::Expired | ExamError::Denied | ExamError::Administrator) => {
                self.clear(session).await?
            }
            Err(ExamError::Unsupported) => {
                session.status = ExamAccountStatus::Unsupported;
                // If the version gate opens again, re-establish the account
                // through its stored session instead of showing stale state.
                session.token = None;
            }
            Err(ExamError::Consent) => session.status = ExamAccountStatus::NeedsConsent,
            _ => {}
        }
        Ok(())
    }

    async fn clear(&self, session: &mut Session) -> Result<(), ExamError> {
        session.token = None;
        session.status = ExamAccountStatus::Disconnected;
        session.version_check = None;
        self.upload.settled(false);
        self.vault.delete(ACCOUNT).map_err(|_| ExamError::Storage)?;
        if let Some(saved) = session.saved.take() {
            cache::erase(&self.account_folder(&saved.uid)).await?;
        }
        Ok(())
    }

    fn service(&self, method: Method, path: &[&str]) -> Result<reqwest::RequestBuilder, ExamError> {
        let path: Vec<_> = std::iter::once("api").chain(path.iter().copied()).collect();
        self.api.request(client::Endpoint::Site, method, &path)
    }

    async fn call<T: DeserializeOwned>(
        &self,
        session: &mut Session,
        request: reqwest::RequestBuilder,
    ) -> Result<T, ExamError> {
        if session.token.is_none() {
            self.resume(session).await?;
        }
        if !matches!(session.status, ExamAccountStatus::Connected) {
            return Err(
                if matches!(session.status, ExamAccountStatus::NeedsConsent) {
                    ExamError::Consent
                } else {
                    ExamError::Unsupported
                },
            );
        }
        let current = session.token.as_ref().ok_or(ExamError::SignedOut)?;
        if current.expires <= chrono::Utc::now().timestamp() + 60 {
            self.renew(session).await?;
        }
        let current = session.token.as_ref().ok_or(ExamError::SignedOut)?;
        let retry = request.try_clone().ok_or(ExamError::Response)?;
        let mut result = self
            .api
            .service(request.bearer_auth(&current.id_token))
            .await;
        if matches!(result, Err(ExamError::Expired)) {
            self.renew(session).await?;
            let current = session.token.as_ref().ok_or(ExamError::SignedOut)?;
            result = self.api.service(retry.bearer_auth(&current.id_token)).await;
        }
        if matches!(result, Err(ExamError::Denied)) {
            let current = session.token.as_ref().ok_or(ExamError::SignedOut)?;
            let updated = self.refresh_claims(current).await?;
            self.remember(session, &updated);
            session.token = Some(updated);
            return Err(ExamError::Rejected);
        }
        result
    }

    async fn renew(&self, session: &mut Session) -> Result<(), ExamError> {
        let current = session.token.as_ref().ok_or(ExamError::SignedOut)?;
        let mut token = identity::refresh(&self.api, &current.saved).await?;
        if !token.allowed {
            token = self.refresh_claims(&token).await?;
        }
        self.remember(session, &token);
        session.token = Some(token);
        Ok(())
    }
}

struct VersionCheck {
    checked_at: SystemTime,
    compatible: bool,
    max_age: Duration,
}

impl VersionCheck {
    fn is_fresh(&self, now: SystemTime) -> bool {
        now.duration_since(self.checked_at)
            .is_ok_and(|age| age < self.max_age)
    }

    fn result(&self) -> Result<(), ExamError> {
        self.compatible.then_some(()).ok_or(ExamError::Unsupported)
    }
}

fn compatible_version(current: &str, minimum: Option<&str>) -> Result<bool, ExamError> {
    let Some(minimum) = minimum else {
        return Ok(true);
    };
    let current = current
        .split(['-', '+'])
        .next()
        .ok_or(ExamError::Response)?;
    let current = semver::Version::parse(current).map_err(|_| ExamError::Response)?;
    let minimum = semver::Version::parse(minimum).map_err(|_| ExamError::Response)?;
    Ok(current >= minimum)
}

/// The subject codes and class a local course is looked up by.
fn course_subject(
    store: &DataStore,
    course_id: &str,
) -> Result<(Vec<String>, Option<u32>), ExamError> {
    let (workspace, school, _) = store
        .view(&["courses", "lmsCourses"].into())
        .map_err(|_| ExamError::Storage)?;
    let course = crate::schedule::visible_by_id(&workspace, "courses", course_id)
        .ok_or(ExamError::NotFound)?;
    let codes = crate::schedule::subject_codes(&workspace, &school, course);
    let class_id = course
        .get("offeringId")
        .and_then(Value::as_str)
        .and_then(|id| id.parse().ok());
    Ok((codes, class_id))
}

#[cfg(test)]
#[path = "../tests/exam_share.rs"]
mod tests;

#[cfg(feature = "ts")]
pub fn declarations(cfg: &ts_rs::Config) -> Vec<String> {
    use ts_rs::TS;
    vec![
        ExamSubject::decl(cfg),
        ExamTeacher::decl(cfg),
        ExamClass::decl(cfg),
        ExamDocument::decl(cfg),
        ExamSubjectPage::decl(cfg),
        ExamSet::decl(cfg),
        ExamRequest::decl(cfg),
        ExamRequestCursor::decl(cfg),
        ExamRequestPage::decl(cfg),
        ExamRequestDraft::decl(cfg),
        ExamLocalDocument::decl(cfg),
        ExamLibrary::decl(cfg),
        ExamAccountStatus::decl(cfg),
        ExamAccount::decl(cfg),
        ExamReportReason::decl(cfg),
        ExamUploadFile::decl(cfg),
        ExamNewTeacher::decl(cfg),
        ExamUploadDraft::decl(cfg),
        ExamRole::decl(cfg),
        ExamOccasion::decl(cfg),
        ExamProvenance::decl(cfg),
        ExamAnswerSource::decl(cfg),
        ExamRoleChoice::decl(cfg),
        ExamChoices::decl(cfg),
        ExamLimits::decl(cfg),
        ExamUploadSubject::decl(cfg),
        ExamUploadForm::decl(cfg),
        ExamUploadStart::decl(cfg),
        ExamUploadEdit::decl(cfg),
        ExamUploadPhase::decl(cfg),
        ExamUploadStatus::decl(cfg),
        ExamAction::decl(cfg),
        ExamAnswer::decl(cfg),
    ]
}
