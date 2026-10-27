use super::*;
use app_core::net::{NetError, SessionCookies as _};

struct SessionFailure(SdkError);
#[async_trait::async_trait]
impl NativeDownloadSession for SessionFailure {
    async fn cookie_header(&self, _: String) -> Result<String, SdkError> {
        Err(self.0.clone())
    }
    async fn store_set_cookies(&self, _: String, _: Vec<String>) -> Result<(), SdkError> {
        Err(self.0.clone())
    }
}

#[tokio::test]
async fn cookie_access_errors_do_not_masquerade_as_missing_login() {
    let url = Url::parse("https://lms.s.isct.ac.jp/").unwrap();
    for (error, expected) in [
        (
            SdkError::Callback {
                detail: "vault inaccessible".into(),
            },
            None,
        ),
        (
            SdkError::Network {
                detail: "offline".into(),
            },
            None,
        ),
        (
            SdkError::Authentication {
                detail: "expired".into(),
            },
            Some(NetError::SignInRequired),
        ),
        (
            SdkError::Timeout {
                detail: "cookie read".into(),
            },
            Some(NetError::Deadline),
        ),
        (SdkError::Cancelled, Some(NetError::Cancelled)),
    ] {
        let session = NativeCookies(Arc::new(SessionFailure(error)));
        let outcome = session.cookie_header(&url).await;
        match expected {
            Some(expected) => assert_eq!(outcome, Err(expected)),
            None => assert!(matches!(outcome, Err(NetError::Connect(_))), "{outcome:?}"),
        }
    }
}
