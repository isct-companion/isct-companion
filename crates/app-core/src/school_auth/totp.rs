use super::{OtpWindow, SchoolAuthError, SchoolAuthService, SchoolOtpCode};
use ring::hmac;
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

pub(super) struct OtpConfig {
    secret: Vec<u8>,
    digits: u32,
    period: u64,
    algorithm: OtpAlgorithm,
}

enum OtpAlgorithm {
    Sha1,
    Sha256,
    Sha512,
}

fn decode_base32(raw: &str) -> Result<Vec<u8>, SchoolAuthError> {
    let mut output = Vec::with_capacity(raw.len() * 5 / 8);
    let mut buffer = 0u32;
    let mut bits = 0u8;
    for character in raw.chars().filter(|character| *character != '=') {
        let value = match character.to_ascii_uppercase() {
            'A'..='Z' => character.to_ascii_uppercase() as u8 - b'A',
            '2'..='7' => character as u8 - b'2' + 26,
            _ => return Err(SchoolAuthError::invalid("OTP secret is not valid Base32.")),
        };
        buffer = (buffer << 5) | u32::from(value);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            output.push((buffer >> bits) as u8);
            buffer &= (1u32 << bits).saturating_sub(1);
        }
    }
    if output.len() < 10 {
        return Err(SchoolAuthError::invalid("OTP secret is too short."));
    }
    Ok(output)
}

pub(super) fn otp_config(raw: &str) -> Result<OtpConfig, SchoolAuthError> {
    let mut secret = raw.to_string();
    let mut digits = 6;
    let mut period = 30;
    let mut algorithm = OtpAlgorithm::Sha1;
    if raw.starts_with("otpauth://") {
        let url = Url::parse(raw).map_err(|_| SchoolAuthError::invalid("OTP link is invalid."))?;
        if url.scheme() != "otpauth" || url.host_str() != Some("totp") {
            return Err(SchoolAuthError::invalid(
                "Only TOTP setup links are supported.",
            ));
        }
        let pairs: std::collections::HashMap<_, _> = url.query_pairs().collect();
        secret = pairs
            .get("secret")
            .map(|value| value.to_string())
            .ok_or_else(|| SchoolAuthError::invalid("OTP link has no secret."))?;
        if let Some(value) = pairs.get("digits") {
            digits = value
                .parse()
                .map_err(|_| SchoolAuthError::invalid("OTP digits are invalid."))?;
        }
        if let Some(value) = pairs.get("period") {
            period = value
                .parse()
                .map_err(|_| SchoolAuthError::invalid("OTP period is invalid."))?;
        }
        if let Some(value) = pairs.get("algorithm") {
            algorithm = match value.to_ascii_uppercase().as_str() {
                "SHA1" => OtpAlgorithm::Sha1,
                "SHA256" => OtpAlgorithm::Sha256,
                "SHA512" => OtpAlgorithm::Sha512,
                _ => return Err(SchoolAuthError::invalid("OTP algorithm is unsupported.")),
            };
        }
    }
    if !(6..=8).contains(&digits) || period == 0 || period > 300 {
        return Err(SchoolAuthError::invalid("OTP parameters are invalid."));
    }
    Ok(OtpConfig {
        secret: decode_base32(&secret.replace([' ', '-'], ""))?,
        digits,
        period,
        algorithm,
    })
}

const OTP_ISSUER: &str = "Science Tokyo";

/// An `otpauth://` setup link passes through unchanged; a bare secret uses
/// the TOTP defaults `otp_config` assumes for it.
pub(super) fn otp_setup_uri(setup: &str, account: &str) -> String {
    use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
    if setup.starts_with("otpauth://") {
        return setup.to_string();
    }
    let issuer = utf8_percent_encode(OTP_ISSUER, NON_ALPHANUMERIC);
    let label = if account.is_empty() {
        issuer.to_string()
    } else {
        format!(
            "{issuer}:{}",
            utf8_percent_encode(account, NON_ALPHANUMERIC)
        )
    };
    let secret = setup.replace([' ', '-', '='], "").to_ascii_uppercase();
    format!("otpauth://totp/{label}?secret={secret}&issuer={issuer}")
}

fn otp_at(config: &OtpConfig, timestamp: u64) -> String {
    let counter = timestamp / config.period;
    let algorithm = match config.algorithm {
        OtpAlgorithm::Sha1 => hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
        OtpAlgorithm::Sha256 => hmac::HMAC_SHA256,
        OtpAlgorithm::Sha512 => hmac::HMAC_SHA512,
    };
    let key = hmac::Key::new(algorithm, &config.secret);
    let digest = hmac::sign(&key, &counter.to_be_bytes());
    let bytes = digest.as_ref();
    let offset = usize::from(bytes[bytes.len() - 1] & 0x0f);
    let binary = (u32::from(bytes[offset] & 0x7f) << 24)
        | (u32::from(bytes[offset + 1]) << 16)
        | (u32::from(bytes[offset + 2]) << 8)
        | u32::from(bytes[offset + 3]);
    let modulus = 10u32.pow(config.digits);
    format!(
        "{:0width$}",
        binary % modulus,
        width = config.digits as usize
    )
}

fn unix_seconds() -> Result<u64, SchoolAuthError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .map_err(|_| SchoolAuthError::storage("System time is unavailable."))
}

pub(super) fn current_otp_at(raw: &str, now: u64) -> Result<SchoolOtpCode, SchoolAuthError> {
    let config = otp_config(raw)?;
    let start = now - now % config.period;
    Ok(SchoolOtpCode {
        otp: otp_at(&config, start),
        expires_at: start + config.period,
        period: config.period,
    })
}

impl SchoolAuthService {
    pub(crate) fn current_otp(&self) -> Result<Option<SchoolOtpCode>, SchoolAuthError> {
        let Some((_, secret)) = self.saved_otp()?.filter(|(_, secret)| !secret.is_empty()) else {
            return Ok(None);
        };
        current_otp_at(&secret, unix_seconds()?).map(Some)
    }
}

/// Codes for the current step and up to three successors for school sign-in.
pub(crate) fn upcoming_otps(raw: &str) -> Result<Vec<OtpWindow>, SchoolAuthError> {
    let config = otp_config(raw)?;
    let now = unix_seconds()?;
    let start = now - now % config.period;
    Ok((0..=3)
        .map(|step| {
            let from = start + step * config.period;
            OtpWindow {
                from,
                code: otp_at(&config, from),
            }
        })
        .collect())
}
