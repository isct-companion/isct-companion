use super::*;
#[test]
fn projection_errors_preserve_boundary_direction() {
    assert!(matches!(
        project::<_, [u8; 2]>(vec![1]),
        Err(crate::SdkError::Internal { .. })
    ));
    assert!(matches!(
        project_input::<_, [u8; 2]>(vec![1]),
        Err(crate::SdkError::InvalidInput { .. })
    ));
    assert_eq!(project_input::<_, [u8; 2]>(vec![1, 2]).unwrap(), [1, 2]);
}
