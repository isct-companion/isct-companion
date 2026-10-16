use super::*;
#[test]
fn scanner_busy_is_retryable_without_claiming_backend_unavailable() {
    assert!(matches!(
        ocr_error(isct_ocr::MatrixOcrError::Busy),
        SdkError::Busy { .. }
    ));
    assert!(matches!(
        ocr_error(isct_ocr::MatrixOcrError::Unavailable),
        SdkError::Unavailable { .. }
    ));
}
