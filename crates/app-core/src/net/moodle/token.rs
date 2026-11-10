//! The Moodle mobile launch: one cookie-authenticated GET of `launch.php`,
//! answered with `302 Location: moodlemobile://token=<base64>`.
//!
//! The Location is parsed here by hand. A generic URL parser reads the token
//! as the URL's host, which is how a probe once wrote a token into a file.
//! Nothing in this module logs, formats or stores the decoded value; the
//! token leaves only as a [`MoodleToken`], whose `Debug` is redacted.
use crate::school_auth::MoodleToken;
use base64::{
    alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
    Engine,
};
use md5::{Digest, Md5};
use url::Url;

pub(crate) const SERVICE: &str = "moodle_mobile_app";
pub(crate) const URL_SCHEME: &str = "moodlemobile";

const LENIENT_BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LaunchError {
    /// The Location is not our app scheme (a login page, another page).
    NotLaunch,
    /// Our scheme, but not `signature:::token[:::privatetoken]`.
    Malformed,
    /// The signature is not md5(wwwroot + passport): not an answer to the
    /// launch this client started.
    SignatureMismatch,
}

/// A fresh, unpredictable passport for one launch.
pub(crate) fn new_passport() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

pub(crate) fn launch_url(wwwroot: &str, passport: &str) -> Result<Url, url::ParseError> {
    let mut url = Url::parse(&format!("{wwwroot}/admin/tool/mobile/launch.php"))?;
    url.query_pairs_mut()
        .append_pair("service", SERVICE)
        .append_pair("passport", passport)
        .append_pair("urlscheme", URL_SCHEME);
    Ok(url)
}

/// md5(wwwroot + passport) in lowercase hex, as `launch.php` computes it.
pub(crate) fn launch_signature(wwwroot: &str, passport: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(wwwroot.as_bytes());
    hasher.update(passport.as_bytes());
    data_encoding::HEXLOWER.encode(&hasher.finalize())
}

/// Reads the raw `Location` of a launch response. The optional third part
/// (`privatetoken`) is dropped here and never leaves this function.
pub(crate) fn parse_launch_location(
    location: &str,
    wwwroot: &str,
    passport: &str,
) -> Result<MoodleToken, LaunchError> {
    let location = location.trim();
    let prefix = format!("{URL_SCHEME}://token=");
    let head = location.get(..prefix.len()).ok_or(LaunchError::NotLaunch)?;
    if !head.eq_ignore_ascii_case(&prefix) {
        return Err(LaunchError::NotLaunch);
    }
    let encoded = percent_encoding::percent_decode_str(&location[prefix.len()..])
        .decode_utf8()
        .map_err(|_| LaunchError::Malformed)?;
    let decoded = LENIENT_BASE64
        .decode(encoded.trim().as_bytes())
        .map_err(|_| LaunchError::Malformed)?;
    let text = std::str::from_utf8(&decoded).map_err(|_| LaunchError::Malformed)?;
    let mut parts = text.split(":::");
    let signature = parts.next().ok_or(LaunchError::Malformed)?;
    let token = parts.next().ok_or(LaunchError::Malformed)?;
    // At most the private token follows; anything more is not Moodle's format.
    let _private = parts.next();
    if parts.next().is_some()
        || signature.len() != 32
        || !signature.bytes().all(|b| b.is_ascii_hexdigit())
        || token.is_empty()
        || token.len() > 128
        || !token.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return Err(LaunchError::Malformed);
    }
    if !signature.eq_ignore_ascii_case(&launch_signature(wwwroot, passport)) {
        return Err(LaunchError::SignatureMismatch);
    }
    Ok(MoodleToken::new(token.to_string()))
}

#[cfg(test)]
#[path = "../../tests/net__moodle__token.rs"]
mod tests;
