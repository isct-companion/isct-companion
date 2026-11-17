//! The upload's signing, against the vector the inbox's own test signs
//! (Homepage `tests/floor-photos.test.mjs`): a change to either side breaks
//! both.
use super::*;

#[test]
fn the_signature_matches_the_inbox_vector() {
    let meta = Meta {
        kid: "test",
        time: 1_763_337_600,
        nonce: "00112233445566778899aabbccddeeff",
        building: "a10026fc-4e59-55a7-9d89-0193f1749620",
        sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    };
    assert_eq!(canonical(&meta).lines().count(), 6);
    assert_eq!(
        sign(b"not-a-real-key", &meta).as_deref(),
        Some("c3ddadd7ca0fd93a29433f78ba78f74f21d211e9485cdf197037d8017701b905")
    );
}
