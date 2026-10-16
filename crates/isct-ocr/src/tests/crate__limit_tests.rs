use super::*;

#[test]
fn base64_payload_bound_matches_the_decoded_image_limit() {
    let largest = MAX_IMAGE_BYTES.div_ceil(3) * 4;
    assert_eq!(check_base64_image_len(largest), Ok(()));
    assert_eq!(check_base64_image_len(largest + 1), Err(image_too_large()));
}
