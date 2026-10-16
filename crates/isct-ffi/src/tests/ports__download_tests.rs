use super::*;
use isct_core::downloads::{DownloadSession as _, FetchError};

struct SessionFailure(SdkError);
#[async_trait::async_trait]
impl NativeDownloadSession for SessionFailure {
    async fn cookie_header(&self, _: String) -> Result<String, SdkError> {
        Err(self.0.clone())
    }
}

#[tokio::test]
async fn cookie_access_errors_do_not_masquerade_as_missing_login() {
    for (error, expected) in [
        (
            SdkError::Callback {
                detail: "vault inaccessible".into(),
            },
            FetchError::Network,
        ),
        (
            SdkError::Network {
                detail: "offline".into(),
            },
            FetchError::Network,
        ),
        (
            SdkError::Authentication {
                detail: "expired".into(),
            },
            FetchError::Login,
        ),
        (
            SdkError::Timeout {
                detail: "cookie read".into(),
            },
            FetchError::Timeout,
        ),
        (SdkError::Cancelled, FetchError::Cancelled),
    ] {
        let session = DownloadSession(Arc::new(SessionFailure(error)));
        assert_eq!(
            session
                .cookie_header(&Url::parse("https://lms.s.isct.ac.jp/").unwrap())
                .await,
            Err(expected)
        );
    }
}
