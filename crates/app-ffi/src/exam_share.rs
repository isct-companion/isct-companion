use crate::{
    json::{project, project_input},
    ports::{NativeBrowserAuth, NativeVault},
    types::ExamAction,
    Operation, OperationResult, SdkError, Session,
};
use app_core::exam_share::{ExamError, ExamShare};
use std::{path::Path, sync::Arc};

pub(crate) fn service(
    vault: Arc<dyn NativeVault>,
    browser: Option<Arc<dyn NativeBrowserAuth>>,
    ios_client_id: Option<String>,
    data: &Path,
    version: String,
) -> Option<Arc<ExamShare>> {
    #[cfg(target_os = "ios")]
    let configuration = browser
        .zip(ios_client_id)
        .map(|(browser, id)| (browser, id, None));
    #[cfg(target_os = "android")]
    let configuration = {
        let _ = ios_client_id;
        browser
            .zip(option_env!("ISCT_GOOGLE_DESKTOP_CLIENT_ID"))
            .map(|(browser, id)| {
                (
                    browser,
                    id.to_owned(),
                    option_env!("ISCT_GOOGLE_DESKTOP_CLIENT_SECRET").map(str::to_owned),
                )
            })
    };
    #[cfg(any(target_os = "ios", target_os = "android"))]
    let browser = configuration.map(|(browser, id, secret)| {
        let client = app_core::oauth::code_exchange::OAuthClient { id, secret };
        let port: Arc<dyn app_core::oauth::NativeBrowserAuth> =
            Arc::new(crate::ports::BrowserAuth(browser));
        (client, port)
    });
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let browser = {
        let _ = (browser, ios_client_id);
        None
    };
    #[cfg(not(any(
        target_os = "ios",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    )))]
    let _ = (browser, ios_client_id);
    ExamShare::new(
        Arc::new(crate::ports::Vault(vault)),
        data.join("exam-share"),
        version,
        #[cfg(any(
            target_os = "ios",
            target_os = "macos",
            target_os = "windows",
            target_os = "android"
        ))]
        browser,
    )
    .ok()
    .map(Arc::new)
}

#[uniffi::export]
impl Session {
    pub fn exam_share(&self, action: ExamAction) -> Result<Arc<Operation>, SdkError> {
        self.start(true, move |inner, cancel| async move {
            let service = inner
                .exam_share
                .as_ref()
                .ok_or_else(|| SdkError::from(ExamError::Unavailable))?;
            let mut answer = service
                .act(project_input(action)?, cancel, &inner.core.store)
                .await?;
            answer.localize_subject_names(&inner.core.store)?;
            Ok(OperationResult::ExamShare {
                answer: project(answer)?,
            })
        })
    }
}
