use crate::{
    json::{project, project_input},
    types::ExamAction,
    Operation, OperationResult, SdkError, Session,
};
#[cfg(not(feature = "demo"))]
use app_core::exam_share::ExamSignIn;
use std::sync::Arc;

/// The sign-in this build offers: iOS's registered client in a web
/// authentication session.
#[cfg(all(not(feature = "demo"), target_os = "ios"))]
pub(crate) fn sign_in(
    session: Option<Arc<dyn crate::ports::NativeWebAuthSession>>,
    client_id: Option<String>,
) -> Option<ExamSignIn> {
    let (session, id) = session.zip(client_id)?;
    Some(ExamSignIn {
        client: app_core::oauth::code_exchange::OAuthClient { id, secret: None },
        browser: Arc::new(crate::ports::BrowserAuth(session)),
    })
}

/// The sign-in this build offers: the installed-app client, answered on
/// Core's loopback once the browser opened Google's page.
#[cfg(all(not(feature = "demo"), not(target_os = "ios")))]
pub(crate) fn sign_in(
    opener: Option<Arc<dyn crate::ports::NativeBrowserOpener>>,
) -> Option<ExamSignIn> {
    Some(ExamSignIn {
        client: app_core::oauth::code_exchange::installed_client()?,
        browser: Arc::new(crate::ports::BrowserAuth(opener?)),
    })
}

#[uniffi::export]
impl Session {
    pub fn exam_share(&self, action: ExamAction) -> Result<Arc<Operation>, SdkError> {
        self.start(true, move |inner, cancel| async move {
            let answer = inner
                .core
                .exam_share(project_input(action)?, cancel)
                .await?;
            Ok(OperationResult::ExamShare {
                answer: project(answer)?,
            })
        })
    }
}
