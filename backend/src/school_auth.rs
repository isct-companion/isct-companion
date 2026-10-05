use crate::lifecycle::SchoolSite;
use crate::secrets::SecretVault;
use ring::hmac;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use url::Url;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SchoolAuthProfile {
    Science,
    TokyoTech,
}

impl SchoolAuthProfile {
    pub const fn account(self) -> &'static str {
        match self {
            Self::Science => "school-science",
            Self::TokyoTech => "school-tokyo-tech",
        }
    }

    pub const fn start_site(self) -> SchoolSite {
        match self {
            Self::Science => SchoolSite::SciencePortal,
            Self::TokyoTech => SchoolSite::TokyoPortal,
        }
    }

    /// The profile whose sign-in form lives on this exact origin.
    pub fn for_auth_host(host: &str) -> Option<Self> {
        match host {
            "isct.ex-tic.com" => Some(Self::Science),
            "portal.titech.ac.jp" | "portal.nap.gsic.titech.ac.jp" => Some(Self::TokyoTech),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SchoolCredentials {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub otp_secret: String,
    #[serde(default)]
    pub matrix: String,
}

impl SchoolCredentials {
    fn validate(&mut self, profile: SchoolAuthProfile) -> Result<(), String> {
        self.username = self.username.trim().to_string();
        self.otp_secret = self.otp_secret.trim().replace(' ', "");
        self.matrix = self
            .matrix
            .chars()
            .filter(|character| character.is_ascii_alphabetic())
            .map(|character| character.to_ascii_uppercase())
            .collect();
        if self.username.is_empty() || self.username.len() > 128 {
            return Err("School username is invalid.".into());
        }
        if self.password.is_empty() || self.password.len() > 512 {
            return Err("School password is invalid.".into());
        }
        if self.otp_secret.len() > 1_024 {
            return Err("OTP setup value is too large.".into());
        }
        match profile {
            SchoolAuthProfile::Science => {
                if self.otp_secret.is_empty() {
                    return Err("Science Tokyo OTP setup link or secret is required.".into());
                }
                otp_config(&self.otp_secret)?;
                if !self.matrix.is_empty() {
                    return Err("Matrix credentials belong to the Tokyo Tech account.".into());
                }
            }
            SchoolAuthProfile::TokyoTech => {
                if self.matrix.len() != 70 {
                    return Err("Matrix must contain 7 rows × 10 columns.".into());
                }
                if !self.otp_secret.is_empty() {
                    return Err("The Tokyo Tech account uses Matrix authentication.".into());
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolCredentialStatus {
    pub science: CredentialSummary,
    pub tokyo_tech: CredentialSummary,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialSummary {
    pub saved: bool,
    pub username: String,
    pub has_otp: bool,
    pub has_matrix: bool,
}

const SCHOOL_CREDENTIAL_ACCOUNT: &str = "school-accounts";
#[derive(Deserialize, Serialize)]
struct SchoolCredentialBundle {
    version: u8,
    accounts: HashMap<SchoolAuthProfile, SchoolCredentials>,
}

pub struct SchoolAuthService<V: SecretVault> {
    vault: V,
    // Only this native process holds unlocked credentials; never persisted here.
    // Missing accounts are cached too, avoiding repeated legacy-keychain probes.
    unlocked: Mutex<HashMap<SchoolAuthProfile, Option<SchoolCredentials>>>,
    status_path: Option<std::path::PathBuf>,
}

impl<V: SecretVault> SchoolAuthService<V> {
    pub fn new(vault: V) -> Self {
        Self {
            vault,
            unlocked: Mutex::new(HashMap::new()),
            status_path: None,
        }
    }

    pub fn with_status_cache(vault: V, path: std::path::PathBuf) -> Self {
        Self { vault, unlocked: Mutex::new(HashMap::new()), status_path: Some(path) }
    }
    fn cached_status(&self) -> Option<SchoolCredentialStatus> {
        let path = self.status_path.as_ref()?;
        let text = std::fs::read_to_string(path).ok()?;
        if text.len() > 4096 { return None; }
        serde_json::from_str(&text).ok()
    }
    fn record_summary(&self, profile: SchoolAuthProfile, credentials: Option<&SchoolCredentials>) {
        if self.status_path.is_none() { return; }
        let Some(mut status) = self.cached_status() else { return; };
        let summary = credentials.map(|c| CredentialSummary { saved: true, username: c.username.clone(), has_otp: !c.otp_secret.is_empty(), has_matrix: !c.matrix.is_empty() }).unwrap_or_default();
        match profile { SchoolAuthProfile::Science => status.science = summary, SchoolAuthProfile::TokyoTech => status.tokyo_tech = summary }
        self.write_status(&status);
    }
    fn write_status(&self, status: &SchoolCredentialStatus) {
        let Some(path) = &self.status_path else { return; };
        let result = (|| -> Result<(), std::io::Error> {
            std::fs::create_dir_all(path.parent().unwrap())?;
            let temp = path.with_extension("tmp");
            std::fs::write(&temp, serde_json::to_vec(&status)?)?;
            std::fs::rename(temp, path)
        })();
        if result.is_err() { let _ = std::fs::remove_file(path); }
    }

    // The mutex covers the complete read-modify-write operation, so saving one
    // account cannot overwrite a concurrent update to the other account.
    fn unlock_accounts(&self, unlocked: &mut HashMap<SchoolAuthProfile, Option<SchoolCredentials>>) -> Result<(), String> {
        if unlocked.len() == 2 { return Ok(()); }
        let accounts = if let Some(payload) = self.vault.load(SCHOOL_CREDENTIAL_ACCOUNT)? {
            let bundle: SchoolCredentialBundle = serde_json::from_str(&payload)
                .map_err(|_| "Stored school credentials are invalid.".to_string())?;
            if bundle.version != 1 { return Err("Stored school credentials version is unsupported.".into()); }
            bundle.accounts
        } else {
            let mut accounts = HashMap::new();
            for profile in [SchoolAuthProfile::Science, SchoolAuthProfile::TokyoTech] {
                if let Some(payload) = self.vault.load(profile.account())? {
                    let mut credentials: SchoolCredentials = serde_json::from_str(&payload)
                        .map_err(|_| "Stored school credentials are invalid.".to_string())?;
                    credentials.validate(profile)?;
                    accounts.insert(profile, credentials);
                }
            }
            if !accounts.is_empty() {
                self.write_accounts(&accounts)?;
                // Verify the replacement before touching either legacy entry.
                let payload = self.vault.load(SCHOOL_CREDENTIAL_ACCOUNT)?.ok_or("Credential migration could not be verified")?;
                let stored: SchoolCredentialBundle = serde_json::from_str(&payload).map_err(|_| "Credential migration could not be verified")?;
                if stored.version != 1 || stored.accounts != accounts { return Err("Credential migration could not be verified".into()); }
                for profile in accounts.keys() { let _ = self.vault.delete(profile.account()); }
            }
            accounts
        };
        let mut validated = HashMap::new();
        for profile in [SchoolAuthProfile::Science, SchoolAuthProfile::TokyoTech] {
            let credentials = accounts.get(&profile).cloned().map(|mut credentials| {
                credentials.validate(profile)?;
                Ok::<_, String>(credentials)
            }).transpose()?;
            validated.insert(profile, credentials);
        }
        *unlocked = validated;
        Ok(())
    }
    fn write_accounts(&self, accounts: &HashMap<SchoolAuthProfile, SchoolCredentials>) -> Result<(), String> {
        let payload = serde_json::to_string(&SchoolCredentialBundle { version: 1, accounts: accounts.clone() })
            .map_err(|_| "School credentials could not be encoded.".to_string())?;
        self.vault.store(SCHOOL_CREDENTIAL_ACCOUNT, &payload)
    }

    pub fn save(
        &self,
        profile: SchoolAuthProfile,
        credentials: SchoolCredentials,
    ) -> Result<bool, String> {
        let credentials = self.resolve(profile, credentials)?;
        self.store(profile, &credentials)?;
        Ok(true)
    }

    fn store(
        &self,
        profile: SchoolAuthProfile,
        credentials: &SchoolCredentials,
    ) -> Result<(), String> {
        let mut unlocked = self
            .unlocked
            .lock()
            .map_err(|_| "Credential cache unavailable")?;
        self.unlock_accounts(&mut unlocked)?;
        if unlocked.get(&profile).and_then(Option::as_ref) == Some(credentials) {
            return Ok(()); // Connect with unchanged credentials must not rewrite Keychain.
        }
        let mut accounts: HashMap<_, _> = unlocked.iter().filter_map(|(profile, value)| value.clone().map(|value| (*profile, value))).collect();
        accounts.insert(profile, credentials.clone());
        self.write_accounts(&accounts)?;
        unlocked.insert(profile, Some(credentials.clone()));
        self.record_summary(profile, Some(credentials));
        Ok(())
    }

    pub fn resolve(
        &self,
        profile: SchoolAuthProfile,
        mut credentials: SchoolCredentials,
    ) -> Result<SchoolCredentials, String> {
        if let Some(saved) = self.load(profile)? {
            if credentials.username.trim().is_empty() {
                credentials.username = saved.username;
            }
            if credentials.password.is_empty() {
                credentials.password = saved.password;
            }
            if credentials.otp_secret.trim().is_empty() {
                credentials.otp_secret = saved.otp_secret;
            }
            if credentials.matrix.trim().is_empty() {
                credentials.matrix = saved.matrix;
            }
        }
        credentials.validate(profile)?;
        Ok(credentials)
    }

    pub fn load(&self, profile: SchoolAuthProfile) -> Result<Option<SchoolCredentials>, String> {
        let mut unlocked = self
            .unlocked
            .lock()
            .map_err(|_| "Credential cache unavailable")?;
        self.unlock_accounts(&mut unlocked)?;
        let credentials = unlocked.get(&profile).cloned().flatten();
        self.record_summary(profile, credentials.as_ref());
        Ok(credentials)
    }

    pub fn delete(&self, profile: SchoolAuthProfile) -> Result<bool, String> {
        let mut unlocked = self.unlocked.lock().map_err(|_| "Credential cache unavailable")?;
        self.unlock_accounts(&mut unlocked)?;
        let mut accounts: HashMap<_, _> = unlocked.iter().filter_map(|(profile, value)| value.clone().map(|value| (*profile, value))).collect();
        accounts.remove(&profile);
        // Keep an empty bundle when forgetting the final account: an old legacy
        // entry whose cleanup failed must never resurrect a forgotten account.
        self.write_accounts(&accounts)?;
        let _ = self.vault.delete(profile.account());
        unlocked.insert(profile, None);
        self.record_summary(profile, None);
        Ok(true)
    }

    pub fn status(&self) -> Result<SchoolCredentialStatus, String> {
        // Metadata only: launch/session checks must not unlock Keychain merely to draw a badge.
        if let Some(status) = self.cached_status() { return Ok(status); }
        let summary = |credentials: Option<SchoolCredentials>| match credentials {
            Some(credentials) => CredentialSummary {
                saved: true,
                username: credentials.username,
                has_otp: !credentials.otp_secret.is_empty(),
                has_matrix: !credentials.matrix.is_empty(),
            },
            None => CredentialSummary::default(),
        };
        let status = SchoolCredentialStatus {
            science: summary(self.load(SchoolAuthProfile::Science)?),
            tokyo_tech: summary(self.load(SchoolAuthProfile::TokyoTech)?),
        };
        self.write_status(&status);
        Ok(status)
    }

    pub fn matrix_file(&self) -> Result<String, String> {
        let credentials = self
            .load(SchoolAuthProfile::TokyoTech)?
            .ok_or_else(|| "Save the Tokyo Tech Matrix before exporting it.".to_string())?;
        matrix_file_json(&credentials.matrix)
    }
}

const MATRIX_ROWS: usize = 7;
const MATRIX_COLUMNS: usize = 10;

/// Matrix files list the card column by column (A1…A7, B1…B7, …), while
/// credentials keep it row by row (A1…J1, A2…). Mirrors `parseMatrixFile` in
/// src/core/matrix-ocr.ts.
fn matrix_file_json(matrix: &str) -> Result<String, String> {
    let cells: Vec<char> = matrix.chars().collect();
    if cells.len() != MATRIX_ROWS * MATRIX_COLUMNS {
        return Err("Matrix must contain 7 rows × 10 columns.".into());
    }
    let by_column: Vec<String> = (0..MATRIX_COLUMNS)
        .flat_map(|column| (0..MATRIX_ROWS).map(move |row| row * MATRIX_COLUMNS + column))
        .map(|index| cells[index].to_string())
        .collect();
    serde_json::to_string_pretty(&by_column).map_err(|_| "Matrix could not be encoded.".to_string())
}

struct OtpConfig {
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

fn decode_base32(raw: &str) -> Result<Vec<u8>, String> {
    let mut output = Vec::with_capacity(raw.len() * 5 / 8);
    let mut buffer = 0u32;
    let mut bits = 0u8;
    for character in raw.chars().filter(|character| *character != '=') {
        let value = match character.to_ascii_uppercase() {
            'A'..='Z' => character.to_ascii_uppercase() as u8 - b'A',
            '2'..='7' => character as u8 - b'2' + 26,
            _ => return Err("OTP secret is not valid Base32.".into()),
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
        return Err("OTP secret is too short.".into());
    }
    Ok(output)
}

fn otp_config(raw: &str) -> Result<OtpConfig, String> {
    let mut secret = raw.to_string();
    let mut digits = 6;
    let mut period = 30;
    let mut algorithm = OtpAlgorithm::Sha1;
    if raw.starts_with("otpauth://") {
        let url = Url::parse(raw).map_err(|_| "OTP link is invalid.".to_string())?;
        if url.scheme() != "otpauth" || url.host_str() != Some("totp") {
            return Err("Only TOTP setup links are supported.".into());
        }
        let pairs: std::collections::HashMap<_, _> = url.query_pairs().collect();
        secret = pairs
            .get("secret")
            .map(|value| value.to_string())
            .ok_or_else(|| "OTP link has no secret.".to_string())?;
        if let Some(value) = pairs.get("digits") {
            digits = value.parse().map_err(|_| "OTP digits are invalid.")?;
        }
        if let Some(value) = pairs.get("period") {
            period = value.parse().map_err(|_| "OTP period is invalid.")?;
        }
        if let Some(value) = pairs.get("algorithm") {
            algorithm = match value.to_ascii_uppercase().as_str() {
                "SHA1" => OtpAlgorithm::Sha1,
                "SHA256" => OtpAlgorithm::Sha256,
                "SHA512" => OtpAlgorithm::Sha512,
                _ => return Err("OTP algorithm is unsupported.".into()),
            };
        }
    }
    if !(6..=8).contains(&digits) || period == 0 || period > 300 {
        return Err("OTP parameters are invalid.".into());
    }
    Ok(OtpConfig {
        secret: decode_base32(&secret.replace([' ', '-'], ""))?,
        digits,
        period,
        algorithm,
    })
}

fn otp_at(raw: &str, timestamp: u64) -> Result<String, String> {
    let config = otp_config(raw)?;
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
    Ok(format!(
        "{:0width$}",
        binary % modulus,
        width = config.digits as usize
    ))
}

/// A code and the Unix second its time step starts.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OtpWindow {
    pub from: u64,
    pub code: String,
}

/// Codes for the current step and the next `ahead` steps, so a page that sits
/// for a while before its code field appears can still submit a live code.
pub fn upcoming_otps(raw: &str, ahead: u64) -> Result<Vec<OtpWindow>, String> {
    let period = otp_config(raw)?.period;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "System time is unavailable.".to_string())?
        .as_secs();
    let start = now - now % period;
    (0..=ahead)
        .map(|step| {
            let from = start + step * period;
            Ok(OtpWindow {
                from,
                code: otp_at(raw, from)?,
            })
        })
        .collect()
}

#[cfg(test)]
pub fn current_otp(raw: &str) -> Result<String, String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "System time is unavailable.".to_string())?
        .as_secs();
    otp_at(raw, timestamp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemoryVault(Mutex<HashMap<String, String>>);

    impl SecretVault for MemoryVault {
        fn store(&self, account: &str, secret: &str) -> Result<(), String> {
            self.0
                .lock()
                .unwrap()
                .insert(account.to_string(), secret.to_string());
            Ok(())
        }

        fn load(&self, account: &str) -> Result<Option<String>, String> {
            Ok(self.0.lock().unwrap().get(account).cloned())
        }

        fn delete(&self, account: &str) -> Result<(), String> {
            self.0.lock().unwrap().remove(account);
            Ok(())
        }
    }

    #[test]
    fn validates_matrix_shape_and_normalizes_case() {
        let mut credentials = SchoolCredentials {
            username: " user ".into(),
            password: "password".into(),
            otp_secret: String::new(),
            matrix: "a".repeat(70),
        };
        credentials.validate(SchoolAuthProfile::TokyoTech).unwrap();
        assert_eq!(credentials.username, "user");
        assert_eq!(credentials.matrix, "A".repeat(70));
    }

    #[test]
    fn matrix_file_lists_the_saved_card_column_by_column() {
        let service = SchoolAuthService::new(MemoryVault::default());
        assert!(service.matrix_file().is_err());
        // Row-major A1…J1, A2…: cell i holds the i-th letter, wrapping after Z.
        let matrix: String = (0..70u8).map(|i| (b'A' + i % 26) as char).collect();
        service
            .save(
                SchoolAuthProfile::TokyoTech,
                SchoolCredentials {
                    username: "user".into(),
                    password: "password".into(),
                    otp_secret: String::new(),
                    matrix,
                },
            )
            .unwrap();
        let file: Vec<String> = serde_json::from_str(&service.matrix_file().unwrap()).unwrap();
        assert_eq!(file.len(), 70);
        assert_eq!(file[..7], ["A", "K", "U", "E", "O", "Y", "I"]); // A1…A7
        assert_eq!(file[7], "B"); // B1
        assert_eq!(file[69], "R"); // J7
    }

    #[test]
    fn generates_rfc_totp_shape() {
        let code = current_otp("JBSWY3DPEHPK3PXP").unwrap();
        assert_eq!(code.len(), 6);
        assert!(code.bytes().all(|byte| byte.is_ascii_digit()));
    }

    #[test]
    fn upcoming_codes_cover_consecutive_windows() {
        let windows = upcoming_otps("JBSWY3DPEHPK3PXP", 3).unwrap();
        assert_eq!(windows.len(), 4);
        assert!(windows
            .windows(2)
            .all(|pair| pair[1].from - pair[0].from == 30));
        for window in &windows {
            assert_eq!(
                window.code,
                otp_at("JBSWY3DPEHPK3PXP", window.from).unwrap()
            );
        }
    }

    #[test]
    fn matches_rfc_6238_sha1_vector() {
        let uri = "otpauth://totp/test?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&digits=8&period=30&algorithm=SHA1";
        assert_eq!(otp_at(uri, 59).unwrap(), "94287082");
    }

    #[test]
    fn stores_both_school_profiles_in_one_item() {
        let service = SchoolAuthService::new(MemoryVault::default());
        service
            .save(
                SchoolAuthProfile::Science,
                SchoolCredentials {
                    username: "science".into(),
                    password: "password".into(),
                    otp_secret: "JBSWY3DPEHPK3PXP".into(),
                    matrix: String::new(),
                },
            )
            .unwrap();
        service
            .save(
                SchoolAuthProfile::TokyoTech,
                SchoolCredentials {
                    username: "tokyo".into(),
                    password: "password".into(),
                    otp_secret: String::new(),
                    matrix: "A".repeat(70),
                },
            )
            .unwrap();
        let status = service.status().unwrap();
        assert!(status.science.has_otp);
        assert_eq!(status.science.username, "science");
        assert!(status.tokyo_tech.has_matrix);
        assert!(!status.tokyo_tech.has_otp);
        assert_eq!(service.vault.0.lock().unwrap().len(), 1);
        assert!(service.vault.load(SCHOOL_CREDENTIAL_ACCOUNT).unwrap().is_some());
        service.delete(SchoolAuthProfile::Science).unwrap();
        assert!(service.load(SchoolAuthProfile::Science).unwrap().is_none());
        assert_eq!(service.load(SchoolAuthProfile::TokyoTech).unwrap().unwrap().username, "tokyo");
    }

    #[test]
    fn blank_secret_fields_keep_the_saved_encrypted_values() {
        let service = SchoolAuthService::new(MemoryVault::default());
        service
            .save(
                SchoolAuthProfile::Science,
                SchoolCredentials {
                    username: "first-user".into(),
                    password: "password".into(),
                    otp_secret: "JBSWY3DPEHPK3PXP".into(),
                    matrix: String::new(),
                },
            )
            .unwrap();
        service
            .save(
                SchoolAuthProfile::Science,
                SchoolCredentials {
                    username: "renamed-user".into(),
                    password: String::new(),
                    otp_secret: String::new(),
                    matrix: String::new(),
                },
            )
            .unwrap();

        let credentials = service.load(SchoolAuthProfile::Science).unwrap().unwrap();
        assert_eq!(credentials.username, "renamed-user");
        assert_eq!(credentials.password, "password");
        assert_eq!(credentials.otp_secret, "JBSWY3DPEHPK3PXP");
    }

    #[test]
    fn persisted_account_status_does_not_unlock_keychain_or_persist_secrets() {
        let dir = std::env::temp_dir().join(format!("school-status-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("status.json");
        let vault = CountingVault::default();
        vault.inner.store(SchoolAuthProfile::Science.account(), &serde_json::to_string(&test_credentials()).unwrap()).unwrap();
        let service = SchoolAuthService::with_status_cache(vault, path.clone());
        assert!(service.status().unwrap().science.saved);
        let metadata = std::fs::read_to_string(&path).unwrap();
        assert!(!metadata.contains("test-password"));
        assert!(!metadata.contains("otpSecret"));
        let locked = CountingVault::default();
        locked.fail_read.store(true, SeqCst);
        let restarted = SchoolAuthService::with_status_cache(locked, path.clone());
        assert!(restarted.status().unwrap().science.saved);
        assert_eq!(restarted.vault.reads.load(SeqCst), 0);
        // Display metadata cannot authorize an actual login or bypass the vault.
        assert!(restarted.load(SchoolAuthProfile::Science).is_err());
        assert!(restarted.delete(SchoolAuthProfile::Science).is_err());
        assert!(restarted.status().unwrap().science.saved);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[derive(Default)]
    struct CountingVault {
        inner: MemoryVault,
        reads: std::sync::atomic::AtomicUsize,
        writes: std::sync::atomic::AtomicUsize,
        fail_read: std::sync::atomic::AtomicBool,
        fail_write: std::sync::atomic::AtomicBool,
    }
    use std::sync::atomic::Ordering::SeqCst;
    impl SecretVault for CountingVault {
        fn load(&self, account: &str) -> Result<Option<String>, String> {
            self.reads.fetch_add(1, SeqCst);
            if self.fail_read.load(SeqCst) {
                return Err("locked".into());
            }
            self.inner.load(account)
        }
        fn store(&self, account: &str, value: &str) -> Result<(), String> {
            self.writes.fetch_add(1, SeqCst);
            if self.fail_write.load(SeqCst) {
                return Err("write denied".into());
            }
            self.inner.store(account, value)
        }
        fn delete(&self, account: &str) -> Result<(), String> {
            self.inner.delete(account)
        }
    }
    fn test_credentials() -> SchoolCredentials {
        SchoolCredentials {
            username: "test-user".into(),
            password: "test-password".into(),
            otp_secret: "JBSWY3DPEHPK3PXP".into(),
            matrix: String::new(),
        }
    }
    #[test]
    fn repeated_login_reads_keychain_once_and_does_not_rewrite_unchanged_values() {
        let vault = CountingVault::default();
        vault
            .inner
            .store(
                SchoolAuthProfile::Science.account(),
                &serde_json::to_string(&test_credentials()).unwrap(),
            )
            .unwrap();
        let service = SchoolAuthService::new(vault);
        for _ in 0..3 {
            let saved = service.load(SchoolAuthProfile::Science).unwrap().unwrap();
            service
                .save(SchoolAuthProfile::Science, saved.clone())
                .unwrap();
            service.status().unwrap();
            service.resolve(SchoolAuthProfile::Science, saved).unwrap();
        }
        assert_eq!(service.vault.reads.load(SeqCst), 4); // bundle, two legacy items, verified replacement
        assert_eq!(service.vault.writes.load(SeqCst), 1); // migration only
    }
    #[test]
    fn failed_migration_keeps_legacy_credentials_and_can_retry() {
        let vault = CountingVault::default();
        vault.inner.store(SchoolAuthProfile::Science.account(), &serde_json::to_string(&test_credentials()).unwrap()).unwrap();
        vault.fail_write.store(true, SeqCst);
        let service = SchoolAuthService::new(vault);
        assert!(service.load(SchoolAuthProfile::Science).is_err());
        assert!(service.vault.inner.load(SchoolAuthProfile::Science.account()).unwrap().is_some());
        assert!(service.vault.inner.load(SCHOOL_CREDENTIAL_ACCOUNT).unwrap().is_none());
        service.vault.fail_write.store(false, SeqCst);
        assert_eq!(service.load(SchoolAuthProfile::Science).unwrap(), Some(test_credentials()));
        assert!(service.vault.inner.load(SchoolAuthProfile::Science.account()).unwrap().is_none());
        assert!(service.vault.inner.load(SCHOOL_CREDENTIAL_ACCOUNT).unwrap().is_some());
    }

    #[test]
    fn changed_credentials_update_cache_only_after_a_successful_write() {
        let service = SchoolAuthService::new(CountingVault::default());
        service
            .save(SchoolAuthProfile::Science, test_credentials())
            .unwrap();
        let mut changed = test_credentials();
        changed.password = "new-test-password".into();
        service.vault.fail_write.store(true, SeqCst);
        assert!(service
            .save(SchoolAuthProfile::Science, changed.clone())
            .is_err());
        assert!(service.load(SchoolAuthProfile::Science).unwrap().unwrap() == test_credentials());
        service.vault.fail_write.store(false, SeqCst);
        service
            .save(SchoolAuthProfile::Science, changed.clone())
            .unwrap();
        assert!(service.load(SchoolAuthProfile::Science).unwrap().unwrap() == changed);
    }
    #[test]
    fn denied_unlock_can_be_retried_and_forget_clears_the_session_copy() {
        let vault = CountingVault::default();
        vault
            .inner
            .store(
                SchoolAuthProfile::Science.account(),
                &serde_json::to_string(&test_credentials()).unwrap(),
            )
            .unwrap();
        let service = SchoolAuthService::new(vault);
        service.vault.fail_read.store(true, SeqCst);
        assert!(service.load(SchoolAuthProfile::Science).is_err());
        service.vault.fail_read.store(false, SeqCst);
        assert!(service.load(SchoolAuthProfile::Science).unwrap().is_some());
        service.delete(SchoolAuthProfile::Science).unwrap();
        assert!(service.load(SchoolAuthProfile::Science).unwrap().is_none());
        assert_eq!(service.vault.reads.load(SeqCst), 5);
        // A new application/service instance performs its own unlock.
        let relaunched = SchoolAuthService::new(service.vault);
        assert!(relaunched
            .load(SchoolAuthProfile::Science)
            .unwrap()
            .is_none());
        assert_eq!(relaunched.vault.reads.load(SeqCst), 6);
    }
}
