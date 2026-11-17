//! Photos of a building whose map data is not marked complete, sent to the
//! maintainers' private inbox (`https://isctcomp.app/api/floor-photos`, the
//! Homepage repository's `functions/_lib/floor-photo.ts`). Nothing sent is
//! shown to anyone; the photos are material for refining the map, and the
//! maintainers work out which floor each one shows.
//!
//! A request proves it comes from a release build by an HMAC-SHA256 over its
//! metadata with a key only those builds carry: the release workflows set
//! `ISCT_FLOOR_PHOTO_KEY` and `ISCT_FLOOR_PHOTO_KID` from the repository's
//! Actions secret and variable. That turns away scripts spraying the
//! endpoint, not someone who digs the key out of a binary; the key rotates
//! with `kid`. A build without the key (any local or fork build) offers no
//! upload.
//!
//! The shells take the photo with the system camera and hand Core a JPEG
//! re-encoded from the pixels, so no EXIF (and no location) leaves the
//! device.

use super::{CampusMapError, CampusMapService};
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const ENDPOINT: &str = "https://isctcomp.app/api/floor-photos";
const PROTOCOL: &str = "isct-floor-photo/2";
/// The inbox refuses more; shells downscale to about 2048 px first.
pub const MAX_PHOTO_BYTES: usize = 8 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(60);

/// The key, never in the binary as one piece: release builds keep it
/// masked by a pad derived at compile time, so a `strings` pass over the
/// app does not show it. The unmasked literal is only read by these
/// constants and is not emitted.
mod key {
    const RAW: &[u8] = match option_env!("ISCT_FLOOR_PHOTO_KEY") {
        Some(key) => key.as_bytes(),
        None => b"",
    };
    pub(super) const KID: Option<&str> = option_env!("ISCT_FLOOR_PHOTO_KID");
    const LEN: usize = RAW.len();

    const fn pad() -> [u8; LEN] {
        let mut out = [0u8; LEN];
        // xorshift64, seeded by the crate version so each release differs.
        let seed = env!("CARGO_PKG_VERSION").as_bytes();
        let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut index = 0;
        while index < seed.len() {
            state = (state ^ seed[index] as u64).wrapping_mul(0x0100_0000_01b3);
            index += 1;
        }
        let mut at = 0;
        while at < LEN {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            out[at] = (state >> 24) as u8;
            at += 1;
        }
        out
    }

    const PAD: [u8; LEN] = pad();
    const MASKED: [u8; LEN] = {
        let mut out = [0u8; LEN];
        let mut at = 0;
        while at < LEN {
            out[at] = RAW[at] ^ PAD[at];
            at += 1;
        }
        out
    };

    /// The key id and key, in a build that carries one.
    pub(super) fn get() -> Option<(&'static str, Vec<u8>)> {
        let kid = KID.filter(|kid| !kid.is_empty())?;
        if LEN == 0 {
            return None;
        }
        let key = MASKED
            .iter()
            .zip(PAD.iter())
            .map(|(masked, pad)| masked ^ pad)
            .collect();
        Some((kid, key))
    }
}

/// Whether this build can send floor photos at all.
pub(crate) fn available() -> bool {
    key::KID.is_some_and(|kid| !kid.is_empty()) && key::get().is_some()
}

/// What a request's signature covers.
pub(super) struct Meta<'a> {
    pub kid: &'a str,
    pub time: u64,
    pub nonce: &'a str,
    pub building: &'a str,
    pub sha256: &'a str,
}

/// One field per line, in the inbox's order.
pub(super) fn canonical(meta: &Meta<'_>) -> String {
    [
        PROTOCOL,
        meta.kid,
        &meta.time.to_string(),
        meta.nonce,
        meta.building,
        meta.sha256,
    ]
    .join("\n")
}

/// The signature as lowercase hex; HMAC takes a key of any length, so
/// `None` never happens in practice.
pub(super) fn sign(key: &[u8], meta: &Meta<'_>) -> Option<String> {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key).ok()?;
    mac.update(canonical(meta).as_bytes());
    Some(data_encoding::HEXLOWER.encode(&mac.finalize().into_bytes()))
}

impl CampusMapService {
    /// Sends one photo of `building_id` (a building the map has whose data
    /// is not marked complete). `jpeg` is the shell's re-encoded JPEG, at
    /// most [`MAX_PHOTO_BYTES`].
    pub async fn upload_photo(
        &self,
        building_id: &str,
        jpeg: Vec<u8>,
    ) -> Result<(), CampusMapError> {
        let (kid, key) = key::get().ok_or(CampusMapError::PhotoUnavailable)?;
        let map = self.map()?.ok_or(CampusMapError::Unavailable)?;
        let building = map
            .buildings
            .get(building_id)
            .ok_or_else(|| CampusMapError::PhotoInvalid("an unknown building".into()))?;
        if map.export.buildings[*building].complete {
            return Err(CampusMapError::PhotoInvalid(
                "a building marked complete".into(),
            ));
        }
        if jpeg.len() > MAX_PHOTO_BYTES {
            return Err(CampusMapError::PhotoTooLarge);
        }
        if !jpeg.starts_with(&[0xff, 0xd8, 0xff]) {
            return Err(CampusMapError::PhotoInvalid("not a JPEG".into()));
        }
        let sha256 = data_encoding::HEXLOWER.encode(&Sha256::digest(&jpeg));
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let meta = Meta {
            kid,
            time,
            nonce: &nonce,
            building: building_id,
            sha256: &sha256,
        };
        let signature = sign(&key, &meta).ok_or(CampusMapError::PhotoUnavailable)?;
        let client = crate::net::client_builder()
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| CampusMapError::Network(error.to_string()))?;
        let length = jpeg.len();
        let response = client
            .post(ENDPOINT)
            .header(reqwest::header::CONTENT_TYPE, "image/jpeg")
            .header(reqwest::header::CONTENT_LENGTH, length)
            .header("X-Isct-Key", kid)
            .header("X-Isct-Time", time)
            .header("X-Isct-Nonce", &nonce)
            .header("X-Isct-Building", building_id)
            .header("X-Isct-Sha256", &sha256)
            .header("X-Isct-Signature", signature)
            .body(jpeg)
            .send()
            .await
            .map_err(|error| CampusMapError::Network(error.without_url().to_string()))?;
        match response.status().as_u16() {
            201 => Ok(()),
            status => Err(CampusMapError::PhotoRefused(status)),
        }
    }
}

#[cfg(test)]
#[path = "../tests/campus_map__photos.rs"]
mod tests;
