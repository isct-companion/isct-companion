//! Device-local OTP import. QR pixels and camera handles stay in the shell.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use base64::{engine::general_purpose, Engine};
use prost::Message;
use serde::Serialize;
use url::Url;

#[derive(Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct OtpImportAccount {
    pub label: String,
    pub issuer: String,
    /// Sensitive: only the selected value enters the unsaved credential draft.
    pub setup: String,
}
#[derive(Debug, thiserror::Error, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OtpImportError {
    #[error("This is not a supported authenticator QR code.")]
    Invalid,
    #[error("Export only your school account, then scan its QR code again.")]
    MultiplePages,
    #[error("This account uses an unsupported verification code format.")]
    Unsupported,
}

// Google Authenticator's transfer payload is protobuf, not an otpauth URL.
// prost owns wire decoding and rejects malformed/truncated fields.
#[derive(Clone, PartialEq, Message)]
struct Migration {
    #[prost(message, repeated, tag = "1")]
    accounts: Vec<Parameters>,
    #[prost(int32, tag = "2")]
    version: i32,
    #[prost(int32, tag = "3")]
    batch_size: i32,
    #[prost(int32, tag = "4")]
    batch_index: i32,
    #[prost(int32, tag = "5")]
    batch_id: i32,
}
#[derive(Clone, PartialEq, Message)]
struct Parameters {
    #[prost(bytes = "vec", tag = "1")]
    secret: Vec<u8>,
    #[prost(string, tag = "2")]
    name: String,
    #[prost(string, tag = "3")]
    issuer: String,
    #[prost(int32, tag = "4")]
    algorithm: i32,
    #[prost(int32, tag = "5")]
    digits: i32,
    #[prost(int32, tag = "6")]
    kind: i32,
    #[prost(int64, tag = "7")]
    counter: i64,
}

pub fn import_otp(raw: &str) -> Result<Vec<OtpImportAccount>, OtpImportError> {
    let raw = raw.trim();
    if raw.len() > 64 * 1024 {
        return Err(OtpImportError::Invalid);
    }
    let url = Url::parse(raw).map_err(|_| OtpImportError::Invalid)?;
    match (url.scheme(), url.host_str()) {
        ("otpauth", Some("totp")) => {
            super::otp_config(raw).map_err(|_| OtpImportError::Invalid)?;
            let label = percent_encoding::percent_decode_str(url.path().trim_start_matches('/'))
                .decode_utf8()
                .map_err(|_| OtpImportError::Invalid)?
                .into_owned();
            let issuer = url
                .query_pairs()
                .find(|(key, _)| key == "issuer")
                .map(|(_, v)| v.into_owned())
                .unwrap_or_default();
            Ok(vec![OtpImportAccount {
                label,
                issuer,
                setup: raw.into(),
            }])
        }
        ("otpauth-migration", Some("offline")) => {
            let encoded = url
                .query_pairs()
                .find(|(key, _)| key == "data")
                .map(|(_, v)| v.into_owned())
                .ok_or(OtpImportError::Invalid)?;
            let bytes = general_purpose::STANDARD
                .decode(&encoded)
                .or_else(|_| general_purpose::URL_SAFE.decode(&encoded))
                .map_err(|_| OtpImportError::Invalid)?;
            let payload =
                Migration::decode(bytes.as_slice()).map_err(|_| OtpImportError::Invalid)?;
            if payload.batch_size > 1 || payload.batch_index != 0 {
                return Err(OtpImportError::MultiplePages);
            }
            // Google Authenticator writes version 1, and version 2 since 2024
            // with the same account fields.
            if !(1..=2).contains(&payload.version)
                || payload.batch_size != 1
                || payload.accounts.is_empty()
                || payload.accounts.len() > 50
            {
                return Err(OtpImportError::Invalid);
            }
            payload
                .accounts
                .into_iter()
                .map(|entry| {
                    if entry.kind != 2 {
                        return Err(OtpImportError::Unsupported);
                    }
                    let algorithm = match entry.algorithm {
                        1 => "SHA1",
                        2 => "SHA256",
                        3 => "SHA512",
                        _ => return Err(OtpImportError::Unsupported),
                    };
                    let digits = match entry.digits {
                        1 => "6",
                        2 => "8",
                        _ => return Err(OtpImportError::Unsupported),
                    };
                    if !(10..=128).contains(&entry.secret.len())
                        || entry.name.len() > 512
                        || entry.issuer.len() > 512
                    {
                        return Err(OtpImportError::Invalid);
                    }
                    let mut setup =
                        Url::parse("otpauth://totp/").map_err(|_| OtpImportError::Invalid)?;
                    setup.set_path(&entry.name);
                    setup.query_pairs_mut().extend_pairs([
                        (
                            "secret",
                            data_encoding::BASE32_NOPAD.encode(&entry.secret).as_str(),
                        ),
                        ("issuer", entry.issuer.as_str()),
                        ("algorithm", algorithm),
                        ("digits", digits),
                        ("period", "30"),
                    ]);
                    let setup = setup.to_string();
                    super::otp_config(&setup).map_err(|_| OtpImportError::Invalid)?;
                    Ok(OtpImportAccount {
                        label: entry.name,
                        issuer: entry.issuer,
                        setup,
                    })
                })
                .collect()
        }
        _ => Err(OtpImportError::Invalid),
    }
}

#[cfg(test)]
#[path = "../tests/school_auth__otp_import.rs"]
mod tests;
