use super::*;

#[test]
fn browser_errors_keep_actionable_executor_categories() {
    assert_eq!(
        map_executor(SchoolBrowserError::Timeout("portal timed out".into())),
        SchoolExecutorError::Timeout("portal timed out".into())
    );
    assert_eq!(
        map_executor(SchoolBrowserError::Auth("sign-in expired".into())),
        SchoolExecutorError::Authentication("sign-in expired".into())
    );
    assert_eq!(
        map_executor(SchoolBrowserError::Storage("database unavailable".into())),
        SchoolExecutorError::Storage("database unavailable".into())
    );
    assert_eq!(
        map_executor(SchoolBrowserError::SiteChanged("portal changed".into())),
        SchoolExecutorError::SiteChanged("portal changed".into())
    );
    assert_eq!(
        map_executor(SchoolBrowserError::Stopped),
        SchoolExecutorError::Stopped("The school browser has stopped.".into())
    );
    assert_eq!(
        map_executor(SchoolBrowserError::Cancelled),
        SchoolExecutorError::Cancelled
    );
}
