use super::*;

/// Without an engine the shared adapter must report the same unavailable
/// capability and typed failure that Core and the shells consume.
#[test]
fn disabled_backend_is_unavailable_through_the_core_port() {
    let vision = matrix_vision();
    assert!(!vision.available());
    assert!(matches!(
        vision.warm_up(),
        Err(MatrixScanError::Unavailable)
    ));
    assert!(matches!(
        vision.decode(&[]),
        Err(MatrixScanError::Unavailable)
    ));
    assert!(matches!(
        vision.recognize(&[]),
        Err(MatrixScanError::Unavailable)
    ));
}
