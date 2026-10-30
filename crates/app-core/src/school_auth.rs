mod otp_enrollment;
mod otp_import;
pub(crate) mod otp_setup;
pub use otp_import::{import_otp, OtpImportAccount, OtpImportError};
pub use otp_setup::{OtpSetupError, SchoolOtpSetup};

use crate::lifecycle::SchoolSite;
use crate::ports::{SecretError, SecretVault};
use crate::school_browser::SchoolSignInProblem;
use ring::hmac;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use url::Url;

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum SchoolAuthError {
    #[error("{0}")]
    InvalidInput(String),
    #[error(transparent)]
    Vault(#[from] SecretError),
    #[error("{0}")]
    Compatibility(String),
    #[error("{0}")]
    Storage(String),
}

impl SchoolAuthError {
    fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidInput(message.into())
    }

    fn compatibility(message: impl Into<String>) -> Self {
        Self::Compatibility(message.into())
    }

    fn storage(message: impl Into<String>) -> Self {
        Self::Storage(message.into())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SchoolAuthProfile {
    Science,
    TokyoTech,
}

impl SchoolAuthProfile {
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
    fn validate(&mut self, profile: SchoolAuthProfile) -> Result<(), SchoolAuthError> {
        self.username = self.username.trim().to_string();
        self.otp_secret = self.otp_secret.trim().replace(' ', "");
        self.matrix = self
            .matrix
            .chars()
            .filter(|character| character.is_ascii_alphabetic())
            .map(|character| character.to_ascii_uppercase())
            .collect();
        if self.username.is_empty() || self.username.len() > 128 {
            return Err(SchoolAuthError::invalid("School username is invalid."));
        }
        if self.password.is_empty() || self.password.len() > 512 {
            return Err(SchoolAuthError::invalid("School password is invalid."));
        }
        if self.otp_secret.len() > 1_024 {
            return Err(SchoolAuthError::invalid("OTP setup value is too large."));
        }
        match profile {
            SchoolAuthProfile::Science => {
                if self.otp_secret.is_empty() {
                    return Err(SchoolAuthError::invalid(
                        "Science Tokyo OTP setup link or secret is required.",
                    ));
                }
                otp_config(&self.otp_secret)?;
                if !self.matrix.is_empty() {
                    return Err(SchoolAuthError::invalid(
                        "Matrix credentials belong to the Tokyo Tech account.",
                    ));
                }
            }
            SchoolAuthProfile::TokyoTech => {
                if self.matrix.len() != 70 {
                    return Err(SchoolAuthError::invalid(
                        "Matrix must contain 7 rows × 10 columns.",
                    ));
                }
                if !self.otp_secret.is_empty() {
                    return Err(SchoolAuthError::invalid(
                        "The Tokyo Tech account uses Matrix authentication.",
                    ));
                }
            }
        }
        Ok(())
    }
}

// Undergraduate student numbers identify the guide cohort (e.g. 25Bxxxxx).
// Graduate numbers identify a different degree's entry, so cannot supply this cohort.
fn student_admission_year(username: &str) -> Option<i32> {
    let number = username.trim().as_bytes();
    if number.len() != 8
        || !matches!(number[2], b'B' | b'b')
        || !number[..2]
            .iter()
            .chain(number[3..].iter())
            .all(u8::is_ascii_digit)
    {
        return None;
    }
    Some(2000 + i32::from(number[0] - b'0') * 10 + i32::from(number[1] - b'0'))
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolCredentialStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_admission_year: Option<i32>,
    pub science: CredentialSummary,
    pub tokyo_tech: CredentialSummary,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialSummary {
    #[serde(default)]
    pub otp_only: bool,
    /// Authoritative vault state. `saved` is retained for older clients, but
    /// new callers must render this state instead of inferring availability.
    #[serde(default)]
    pub availability: CredentialAvailability,
    pub saved: bool,
    pub username: String,
    pub has_otp: bool,
    pub has_matrix: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CredentialAvailability {
    #[default]
    NotConfigured,
    /// The metadata cache says a secret exists; the vault has not been opened
    /// in this process yet.
    SavedLocked,
    Available,
    /// Metadata referred to a secret which no longer exists in the vault.
    Missing,
    /// The OS denied or failed vault access. This is deliberately distinct
    /// from a missing secret so the UI never asks the user to overwrite it.
    AccessDenied,
}

const SCHOOL_CREDENTIAL_ACCOUNT: &str = "school-accounts";

/// A Moodle mobile web-service token. It lives only in memory and in the OS
/// vault; `Debug` never prints it and it has no `Serialize`.
#[derive(Clone, Eq, PartialEq)]
pub struct MoodleToken(String);

impl MoodleToken {
    pub(crate) fn new(token: String) -> Self {
        Self(token)
    }

    /// The raw value, for the one form field or file query that carries it.
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for MoodleToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("MoodleToken([redacted])")
    }
}

/// The vault record of a token: whose it is, so a token never outlives the
/// account it was issued to. Device-only (see `ports::vault_item`).
#[derive(Deserialize, Serialize)]
struct StoredMoodleToken {
    version: u8,
    account: String,
    token: String,
}

/// Only the Science profile has an LMS.
fn moodle_token_account(profile: SchoolAuthProfile) -> Option<&'static str> {
    match profile {
        SchoolAuthProfile::Science => Some("lms-token-science"),
        SchoolAuthProfile::TokyoTech => None,
    }
}
/// The synchronized item: credentials the school accepted on some device.
#[derive(Deserialize, Serialize)]
struct SchoolCredentialBundle {
    version: u8,
    accounts: HashMap<SchoolAuthProfile, SchoolCredentials>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    science_otp: Option<otp_enrollment::EnrollmentOtp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending_science_otp: Option<otp_enrollment::EnrollmentOtp>,
}

/// Device-local credentials typed here that no sign-in has accepted yet.
/// Like a pending OTP enrollment, they never sync until the school accepts
/// them; unlike it, they live outside the synchronized item.
const SCHOOL_CANDIDATE_ACCOUNT: &str = "school-candidates";

#[derive(Default, Deserialize, Serialize)]
struct CandidateBundle {
    version: u8,
    accounts: HashMap<SchoolAuthProfile, Candidate>,
}

#[derive(Clone, Deserialize, Serialize)]
struct Candidate {
    credentials: SchoolCredentials,
    /// Digest of the verified credentials this candidate replaces. A different
    /// verified value means another device changed them: the candidate goes.
    base: String,
    /// Why the school turned these down, kept so a relaunch does not submit
    /// a rejected password again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    failure: Option<SchoolSignInProblem>,
}

/// The credentials a sign-in uses on this device, and why the school turned
/// them down if it already did.
#[derive(Clone, Debug)]
pub(crate) struct SignInCredentials {
    pub credentials: SchoolCredentials,
    pub failure: Option<SchoolSignInProblem>,
}

#[derive(Default)]
struct Accounts {
    /// The synchronized payload `verified` was parsed from; `None` before the
    /// first read in this process.
    payload: Option<Option<String>>,
    verified: HashMap<SchoolAuthProfile, SchoolCredentials>,
    /// Read once: only this process writes the device-local item.
    candidates: Option<HashMap<SchoolAuthProfile, Candidate>>,
    /// Another device may have replaced the synchronized item since it was
    /// read (`refresh`).
    stale: bool,
}

impl Accounts {
    fn candidates(&self) -> &HashMap<SchoolAuthProfile, Candidate> {
        static EMPTY: std::sync::OnceLock<HashMap<SchoolAuthProfile, Candidate>> =
            std::sync::OnceLock::new();
        self.candidates
            .as_ref()
            .unwrap_or_else(|| EMPTY.get_or_init(HashMap::new))
    }

    /// What this device signs in with: its own untested input first.
    fn active(&self, profile: SchoolAuthProfile) -> Option<&SchoolCredentials> {
        self.candidates()
            .get(&profile)
            .map(|candidate| &candidate.credentials)
            .or_else(|| self.verified.get(&profile))
    }
}

fn digest(credentials: Option<&SchoolCredentials>) -> String {
    use sha2::{Digest, Sha256};
    let text = credentials
        .and_then(|credentials| serde_json::to_string(credentials).ok())
        .unwrap_or_default();
    data_encoding::HEXLOWER.encode(&Sha256::digest(text.as_bytes()))
}

pub struct SchoolAuthService {
    vault: Arc<dyn SecretVault>,
    // Only this native process holds unlocked credentials; never persisted here.
    accounts: Mutex<Accounts>,
    status_path: Option<std::path::PathBuf>,
}

impl SchoolAuthService {
    #[cfg(test)]
    pub fn new(vault: impl SecretVault + 'static) -> Self {
        Self {
            vault: Arc::new(vault),
            accounts: Mutex::new(Accounts::default()),
            status_path: None,
        }
    }

    pub fn with_status_cache(vault: impl SecretVault + 'static, path: std::path::PathBuf) -> Self {
        Self {
            vault: Arc::new(vault),
            accounts: Mutex::new(Accounts::default()),
            status_path: Some(path),
        }
    }
    fn cached_status(&self) -> Option<SchoolCredentialStatus> {
        let path = self.status_path.as_ref()?;
        let text = std::fs::read_to_string(path).ok()?;
        if text.len() > 4096 {
            return None;
        }
        serde_json::from_str(&text).ok()
    }
    fn record_summary(
        &self,
        profile: SchoolAuthProfile,
        credentials: Option<&SchoolCredentials>,
        availability: CredentialAvailability,
    ) {
        if self.status_path.is_none() {
            return;
        }
        let Some(mut status) = self.cached_status() else {
            return;
        };
        let previous = match profile {
            SchoolAuthProfile::Science => status.science.clone(),
            SchoolAuthProfile::TokyoTech => status.tokyo_tech.clone(),
        };
        let summary = credentials
            .map(|c| CredentialSummary {
                availability,
                saved: true,
                username: c.username.clone(),
                has_otp: !c.otp_secret.is_empty(),
                has_matrix: !c.matrix.is_empty(),
                otp_only: false,
            })
            .unwrap_or_else(|| {
                if availability == CredentialAvailability::AccessDenied
                    || (profile == SchoolAuthProfile::Science && previous.otp_only)
                {
                    CredentialSummary {
                        availability,
                        ..previous
                    }
                } else {
                    CredentialSummary {
                        availability,
                        ..CredentialSummary::default()
                    }
                }
            });
        match profile {
            SchoolAuthProfile::Science => status.science = summary,
            SchoolAuthProfile::TokyoTech => status.tokyo_tech = summary,
        }
        self.write_status(&status);
    }
    fn write_status(&self, status: &SchoolCredentialStatus) {
        let Some(path) = &self.status_path else {
            return;
        };
        let result = (|| -> Result<(), std::io::Error> {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent)?;
            }
            let temp = path.with_extension("tmp");
            std::fs::write(&temp, serde_json::to_vec(&status)?)?;
            std::fs::rename(temp, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(path);
        }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Accounts>, SchoolAuthError> {
        self.accounts
            .lock()
            .map_err(|_| SchoolAuthError::storage("Credential cache unavailable"))
    }

    /// The next use reads the synchronized item again, where another device
    /// can replace it. Core calls this when a session check starts, before a
    /// connect merges blank fields and after the school turns credentials
    /// down, not on every read.
    pub(crate) fn refresh(&self) {
        if let Ok(mut accounts) = self.accounts.lock() {
            accounts.stale = true;
        }
    }

    // The mutex covers the complete read-modify-write operation, so saving one
    // account cannot overwrite a concurrent update to the other account.
    fn unlock_accounts(&self, accounts: &mut Accounts) -> Result<(), SchoolAuthError> {
        if accounts.payload.is_none()
            || (accounts.stale && self.vault.shared(SCHOOL_CREDENTIAL_ACCOUNT))
        {
            let payload = self.vault.load(SCHOOL_CREDENTIAL_ACCOUNT)?;
            self.adopt(accounts, payload)?;
        }
        accounts.stale = false;
        Ok(())
    }

    /// Takes `payload` as the synchronized item's content, read or just
    /// written: writes are never read back, since on the macOS login keychain
    /// a read can ask for the password.
    fn adopt(
        &self,
        accounts: &mut Accounts,
        payload: Option<String>,
    ) -> Result<(), SchoolAuthError> {
        if accounts.payload.as_ref() != Some(&payload) {
            let parsed = match &payload {
                Some(payload) => {
                    let bundle: SchoolCredentialBundle =
                        serde_json::from_str(payload).map_err(|_| {
                            SchoolAuthError::compatibility("Stored school credentials are invalid.")
                        })?;
                    if bundle.version != 1 {
                        return Err(SchoolAuthError::compatibility(
                            "Stored school credentials version is unsupported.",
                        ));
                    }
                    bundle.accounts
                }
                None => HashMap::new(),
            };
            let mut verified = HashMap::new();
            for (profile, mut credentials) in parsed {
                credentials
                    .validate(profile)
                    .map_err(|error| SchoolAuthError::compatibility(error.to_string()))?;
                verified.insert(profile, credentials);
            }
            accounts.verified = verified;
            accounts.payload = Some(payload);
        }
        if accounts.candidates.is_none() {
            let mut candidates = match self.vault.load(SCHOOL_CANDIDATE_ACCOUNT)? {
                Some(payload) => serde_json::from_str::<CandidateBundle>(&payload)
                    .ok()
                    .filter(|bundle| bundle.version == 1)
                    .map(|bundle| bundle.accounts)
                    .unwrap_or_default(),
                None => HashMap::new(),
            };
            candidates
                .retain(|profile, candidate| candidate.credentials.validate(*profile).is_ok());
            accounts.candidates = Some(candidates);
        }
        // Newer verified credentials from another device replace what this
        // device was still trying.
        let stale: Vec<_> = accounts
            .candidates()
            .iter()
            .filter(|(profile, candidate)| candidate.base != digest(accounts.verified.get(profile)))
            .map(|(profile, _)| *profile)
            .collect();
        if !stale.is_empty() {
            let mut candidates = accounts.candidates().clone();
            for profile in &stale {
                candidates.remove(profile);
            }
            self.write_candidates(&candidates)?;
            accounts.candidates = Some(candidates);
        }
        Ok(())
    }

    fn write_candidates(
        &self,
        candidates: &HashMap<SchoolAuthProfile, Candidate>,
    ) -> Result<(), SchoolAuthError> {
        if candidates.is_empty() {
            return self
                .vault
                .delete(SCHOOL_CANDIDATE_ACCOUNT)
                .map_err(SchoolAuthError::from);
        }
        let payload = serde_json::to_string(&CandidateBundle {
            version: 1,
            accounts: candidates.clone(),
        })
        .map_err(|_| SchoolAuthError::storage("School credentials could not be encoded."))?;
        self.vault
            .store(SCHOOL_CANDIDATE_ACCOUNT, &payload)
            .map_err(SchoolAuthError::from)
    }

    fn write_accounts(
        &self,
        accounts: &HashMap<SchoolAuthProfile, SchoolCredentials>,
    ) -> Result<String, SchoolAuthError> {
        let stored = self.enrollment_bundle()?;
        self.write_credential_bundle(accounts, stored.science_otp, stored.pending_science_otp)
    }

    fn write_accounts_with_otp(
        &self,
        accounts: &HashMap<SchoolAuthProfile, SchoolCredentials>,
        science_otp: Option<otp_enrollment::EnrollmentOtp>,
    ) -> Result<String, SchoolAuthError> {
        self.write_credential_bundle(accounts, science_otp, None)
    }

    fn write_credential_bundle(
        &self,
        accounts: &HashMap<SchoolAuthProfile, SchoolCredentials>,
        science_otp: Option<otp_enrollment::EnrollmentOtp>,
        pending_science_otp: Option<otp_enrollment::EnrollmentOtp>,
    ) -> Result<String, SchoolAuthError> {
        let payload = serde_json::to_string(&SchoolCredentialBundle {
            version: 1,
            accounts: accounts.clone(),
            science_otp,
            pending_science_otp,
        })
        .map_err(|_| SchoolAuthError::storage("School credentials could not be encoded."))?;
        self.vault.store(SCHOOL_CREDENTIAL_ACCOUNT, &payload)?;
        Ok(payload)
    }

    /// Returns the validated native credential bundle for Rust-owned encrypted
    /// cloud sync. This value must never cross the WebView IPC boundary. Only
    /// credentials the school accepted leave the device.
    pub fn export_sync_payload(&self) -> Result<String, SchoolAuthError> {
        let mut accounts = self.lock()?;
        self.unlock_accounts(&mut accounts)?;
        serde_json::to_string(&SchoolCredentialBundle {
            version: 1,
            accounts: accounts.verified.clone(),
            science_otp: self.enrollment_otp()?.filter(|otp| otp.confirmed),
            pending_science_otp: self.enrollment_otp()?.filter(|otp| otp.confirmed),
        })
        .map_err(|_| SchoolAuthError::storage("School credentials could not be encoded."))
    }

    /// Validates and replaces the native credential bundle after Rust has
    /// authenticated and decrypted it. No plaintext is returned to the UI.
    pub fn import_sync_payload(&self, payload: &str) -> Result<usize, SchoolAuthError> {
        let mut bundle: SchoolCredentialBundle = serde_json::from_str(payload).map_err(|_| {
            SchoolAuthError::compatibility("Synced school credentials are invalid.")
        })?;
        if bundle.version != 1 {
            return Err(SchoolAuthError::compatibility(
                "Synced school credentials version is unsupported.",
            ));
        }
        for (profile, credentials) in &mut bundle.accounts {
            credentials.validate(*profile)?;
        }
        let mut accounts = self.lock()?;
        if let Some(otp) = &bundle.science_otp {
            if !otp.confirmed || otp.username.is_empty() || otp.username.len() > 128 {
                return Err(SchoolAuthError::invalid(
                    "Synced OTP enrollment is invalid.",
                ));
            }
            otp_config(&otp.setup)?;
        }
        self.unlock_accounts(&mut accounts)?;
        let payload = self.write_accounts_with_otp(&bundle.accounts, bundle.science_otp.clone())?;
        // Drops the candidates the imported accounts replace.
        self.adopt(&mut accounts, Some(payload))?;
        self.record_enrollment_summary(None);
        for profile in [SchoolAuthProfile::Science, SchoolAuthProfile::TokyoTech] {
            let credentials = accounts.active(profile).cloned();
            self.record_summary(
                profile,
                credentials.as_ref(),
                if credentials.is_some() {
                    CredentialAvailability::Available
                } else {
                    CredentialAvailability::NotConfigured
                },
            );
        }
        if !bundle.accounts.contains_key(&SchoolAuthProfile::Science) {
            self.record_enrollment_summary(bundle.science_otp.as_ref());
        }
        Ok(bundle.accounts.len()
            + usize::from(
                bundle.science_otp.is_some()
                    && !bundle.accounts.contains_key(&SchoolAuthProfile::Science),
            ))
    }

    /// Keeps `credentials` (blank fields filled as `resolve` does) as this
    /// device's candidate. Nothing reaches the synchronized item until a
    /// sign-in with them succeeds (`promote`). Returns what was kept.
    pub fn stage(
        &self,
        profile: SchoolAuthProfile,
        credentials: SchoolCredentials,
    ) -> Result<SchoolCredentials, SchoolAuthError> {
        // Blank fields fill from what the vault holds now, not a stale copy.
        self.refresh();
        let credentials = self.resolve(profile, credentials)?;
        let mut accounts = self.lock()?;
        self.unlock_accounts(&mut accounts)?;
        let mut candidates = accounts.candidates().clone();
        if accounts.verified.get(&profile) == Some(&credentials) {
            // Connect with the verified credentials must not rewrite Keychain.
            if candidates.remove(&profile).is_none() {
                return Ok(credentials);
            }
        } else {
            candidates.insert(
                profile,
                Candidate {
                    credentials: credentials.clone(),
                    base: digest(accounts.verified.get(&profile)),
                    // Connecting again is the person's decision to retry.
                    failure: None,
                },
            );
        }
        self.write_candidates(&candidates)?;
        accounts.candidates = Some(candidates);
        self.record_summary(
            profile,
            Some(&credentials),
            CredentialAvailability::Available,
        );
        Ok(credentials)
    }

    /// The school accepted `credentials`: when they are this device's
    /// candidate, they become the verified, synchronized credentials.
    pub(crate) fn promote(
        &self,
        profile: SchoolAuthProfile,
        credentials: &SchoolCredentials,
    ) -> Result<bool, SchoolAuthError> {
        let mut accounts = self.lock()?;
        self.unlock_accounts(&mut accounts)?;
        if accounts
            .candidates()
            .get(&profile)
            .is_none_or(|candidate| &candidate.credentials != credentials)
        {
            return Ok(false);
        }
        let mut verified = accounts.verified.clone();
        verified.insert(profile, credentials.clone());
        let payload = self.write_accounts(&verified)?;
        let mut candidates = accounts.candidates().clone();
        candidates.remove(&profile);
        self.write_candidates(&candidates)?;
        accounts.candidates = Some(candidates);
        self.adopt(&mut accounts, Some(payload))?;
        Ok(true)
    }

    /// The school turned `credentials` down for a reason that needs the
    /// person. A candidate keeps the reason across launches.
    pub(crate) fn record_failure(
        &self,
        profile: SchoolAuthProfile,
        credentials: &SchoolCredentials,
        failure: &SchoolSignInProblem,
    ) -> Result<(), SchoolAuthError> {
        let mut accounts = self.lock()?;
        self.unlock_accounts(&mut accounts)?;
        let mut candidates = accounts.candidates().clone();
        let Some(candidate) = candidates
            .get_mut(&profile)
            .filter(|candidate| &candidate.credentials == credentials)
        else {
            return Ok(());
        };
        if candidate.failure.as_ref() == Some(failure) {
            return Ok(());
        }
        candidate.failure = Some(failure.clone());
        self.write_candidates(&candidates)?;
        accounts.candidates = Some(candidates);
        Ok(())
    }

    /// Whether `credentials` are the verified ones, which sync to the
    /// person's other devices.
    pub(crate) fn is_verified(
        &self,
        profile: SchoolAuthProfile,
        credentials: &SchoolCredentials,
    ) -> Result<bool, SchoolAuthError> {
        let mut accounts = self.lock()?;
        self.unlock_accounts(&mut accounts)?;
        Ok(accounts.verified.get(&profile) == Some(credentials))
    }

    pub(crate) fn sign_in_credentials(
        &self,
        profile: SchoolAuthProfile,
    ) -> Result<Option<SignInCredentials>, SchoolAuthError> {
        let credentials = self.load(profile)?;
        let accounts = self.lock()?;
        Ok(credentials.map(|credentials| SignInCredentials {
            failure: accounts
                .candidates()
                .get(&profile)
                .filter(|candidate| candidate.credentials == credentials)
                .and_then(|candidate| candidate.failure.clone()),
            credentials,
        }))
    }

    /// Blank fields keep what this device signs in with now: its candidate,
    /// else the verified credentials as the vault holds them at this moment.
    pub fn resolve(
        &self,
        profile: SchoolAuthProfile,
        mut credentials: SchoolCredentials,
    ) -> Result<SchoolCredentials, SchoolAuthError> {
        let status_expected_saved = self
            .cached_status()
            .map(|status| match profile {
                SchoolAuthProfile::Science => status.science.saved,
                SchoolAuthProfile::TokyoTech => status.tokyo_tech.saved,
            })
            .unwrap_or(false);
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
        } else if status_expected_saved
            && (credentials.password.is_empty()
                || match profile {
                    SchoolAuthProfile::Science => credentials.otp_secret.trim().is_empty(),
                    SchoolAuthProfile::TokyoTech => credentials.matrix.trim().is_empty(),
                })
        {
            return Err(SchoolAuthError::InvalidInput(
                "Saved credentials were not found in the system credential vault. Enter them again."
                    .into(),
            ));
        }
        if profile == SchoolAuthProfile::Science && credentials.otp_secret.trim().is_empty() {
            if let Some(otp) = self.enrollment_otp()?.filter(|otp| {
                otp.confirmed
                    && (credentials.username.trim().is_empty()
                        || credentials.username.trim() == otp.username)
            }) {
                credentials.username = otp.username;
                credentials.otp_secret = otp.setup;
            }
        }
        credentials.validate(profile)?;
        Ok(credentials)
    }

    /// The credentials this device signs in with (see `Accounts::active`).
    pub fn load(
        &self,
        profile: SchoolAuthProfile,
    ) -> Result<Option<SchoolCredentials>, SchoolAuthError> {
        let expected_saved = self
            .cached_status()
            .map(|status| match profile {
                SchoolAuthProfile::Science => status.science.saved,
                SchoolAuthProfile::TokyoTech => status.tokyo_tech.saved,
            })
            .unwrap_or(false);
        let mut accounts = self.lock()?;
        if let Err(error) = self.unlock_accounts(&mut accounts) {
            self.record_summary(profile, None, CredentialAvailability::AccessDenied);
            return Err(error);
        }
        let credentials = accounts.active(profile).cloned();
        self.record_summary(
            profile,
            credentials.as_ref(),
            if credentials.is_some() {
                CredentialAvailability::Available
            } else if expected_saved {
                CredentialAvailability::Missing
            } else {
                CredentialAvailability::NotConfigured
            },
        );
        Ok(credentials)
    }

    pub fn delete(&self, profile: SchoolAuthProfile) -> Result<bool, SchoolAuthError> {
        let mut accounts = self.lock()?;
        self.unlock_accounts(&mut accounts)?;
        let mut verified = accounts.verified.clone();
        verified.remove(&profile);
        let payload = if profile == SchoolAuthProfile::Science {
            self.write_accounts_with_otp(&verified, None)?
        } else {
            self.write_accounts(&verified)?
        };
        let mut candidates = accounts.candidates().clone();
        if candidates.remove(&profile).is_some() {
            self.write_candidates(&candidates)?;
        }
        accounts.candidates = Some(candidates);
        self.adopt(&mut accounts, Some(payload))?;
        drop(accounts);
        if profile == SchoolAuthProfile::Science {
            self.record_enrollment_summary(None);
        }
        // Signing out also forgets the LMS token; students cannot revoke it.
        let _ = self.delete_moodle_token(profile);
        self.record_summary(profile, None, CredentialAvailability::NotConfigured);
        Ok(true)
    }

    /// Who a token belongs to: the saved username, or "" while the school
    /// web session was signed in without saved credentials.
    fn moodle_token_owner(&self, profile: SchoolAuthProfile) -> Result<String, SchoolAuthError> {
        Ok(self
            .load(profile)?
            .map(|credentials| credentials.username)
            .unwrap_or_default())
    }

    /// The account synced school data belongs to (`crate::sync_state::account`).
    pub fn sync_account(&self, profile: SchoolAuthProfile) -> Result<String, SchoolAuthError> {
        Ok(crate::sync_state::account(
            profile,
            &self.moodle_token_owner(profile)?,
        ))
    }

    /// The stored token of this profile's current account. A record for
    /// another account, or one this version cannot read, is deleted.
    pub fn load_moodle_token(
        &self,
        profile: SchoolAuthProfile,
    ) -> Result<Option<MoodleToken>, SchoolAuthError> {
        let Some(account) = moodle_token_account(profile) else {
            return Ok(None);
        };
        let Some(payload) = self.vault.load(account)? else {
            return Ok(None);
        };
        let stored = serde_json::from_str::<StoredMoodleToken>(&payload)
            .ok()
            .filter(|stored| stored.version == 1 && !stored.token.is_empty());
        let owner = self.moodle_token_owner(profile)?;
        match stored {
            Some(stored) if stored.account == owner => Ok(Some(MoodleToken(stored.token))),
            _ => {
                self.vault.delete(account)?;
                Ok(None)
            }
        }
    }

    pub fn store_moodle_token(
        &self,
        profile: SchoolAuthProfile,
        token: &MoodleToken,
    ) -> Result<(), SchoolAuthError> {
        let account = moodle_token_account(profile)
            .ok_or_else(|| SchoolAuthError::invalid("This school account has no LMS."))?;
        let payload = serde_json::to_string(&StoredMoodleToken {
            version: 1,
            account: self.moodle_token_owner(profile)?,
            token: token.0.clone(),
        })
        .map_err(|_| SchoolAuthError::storage("The LMS token could not be encoded."))?;
        self.vault
            .store(account, &payload)
            .map_err(SchoolAuthError::from)
    }

    pub fn delete_moodle_token(&self, profile: SchoolAuthProfile) -> Result<(), SchoolAuthError> {
        match moodle_token_account(profile) {
            Some(account) => self.vault.delete(account).map_err(SchoolAuthError::from),
            None => Ok(()),
        }
    }

    pub fn status(&self) -> Result<SchoolCredentialStatus, SchoolAuthError> {
        // Metadata only: launch/session checks must not unlock Keychain merely to draw a badge.
        if let Some(mut status) = self.cached_status() {
            for summary in [&mut status.science, &mut status.tokyo_tech] {
                if summary.saved && summary.availability != CredentialAvailability::AccessDenied {
                    summary.availability = CredentialAvailability::SavedLocked;
                } else if summary.availability != CredentialAvailability::Missing
                    && summary.availability != CredentialAvailability::AccessDenied
                {
                    summary.availability = CredentialAvailability::NotConfigured;
                }
            }
            status.suggested_admission_year = student_admission_year(&status.tokyo_tech.username);
            return Ok(status);
        }
        let summary = |credentials: Option<SchoolCredentials>| match credentials {
            Some(credentials) => CredentialSummary {
                availability: CredentialAvailability::Available,
                saved: true,
                username: credentials.username,
                has_otp: !credentials.otp_secret.is_empty(),
                has_matrix: !credentials.matrix.is_empty(),
                otp_only: false,
            },
            None => CredentialSummary::default(),
        };
        let mut status = SchoolCredentialStatus {
            science: summary(self.load(SchoolAuthProfile::Science)?),
            tokyo_tech: summary(self.load(SchoolAuthProfile::TokyoTech)?),
            ..Default::default()
        };
        if !status.science.saved {
            if let Some(otp) = self.enrollment_otp()?.filter(|otp| otp.confirmed) {
                status.science.username = otp.username;
                status.science.has_otp = true;
                status.science.otp_only = true;
            }
        }
        status.suggested_admission_year = student_admission_year(&status.tokyo_tech.username);
        self.write_status(&status);
        Ok(status)
    }

    /// Explicit export only. Native shells write the result directly to the
    /// clipboard instead of returning a saved secret to their WebView.
    pub fn otp_setup_text(&self, draft: Option<&str>) -> Result<String, SchoolAuthError> {
        let text = if let Some(value) = draft.filter(|value| !value.trim().is_empty()) {
            value.trim().replace(' ', "")
        } else {
            self.load(SchoolAuthProfile::Science)?
                .map(|credentials| credentials.otp_secret)
                .or(self
                    .enrollment_otp()?
                    .filter(|otp| otp.confirmed)
                    .map(|otp| otp.setup))
                .unwrap_or_default()
        };
        if text.is_empty() {
            return Err(SchoolAuthError::invalid(
                "Set up school verification codes before copying.",
            ));
        }
        if text.len() > 1_024 {
            return Err(SchoolAuthError::invalid("OTP setup is too long."));
        }
        otp_config(&text)?;
        Ok(text)
    }

    /// The same explicit export as an `otpauth://` link any authenticator
    /// accepts. A bare secret gets the school's issuer and the saved account
    /// name as its label. Mobile shells hand it to the OS to open one.
    pub fn otp_setup_link(&self, draft: Option<&str>) -> Result<String, SchoolAuthError> {
        let text = self.otp_setup_text(draft)?;
        let account = match self.load(SchoolAuthProfile::Science)? {
            Some(credentials) => credentials.username,
            None => self
                .enrollment_otp()?
                .map(|otp| otp.username)
                .unwrap_or_default(),
        };
        Ok(otp_setup_uri(&text, &account))
    }

    /// `otp_setup_link` as an SVG QR code, for a phone to scan from the desktop.
    pub fn otp_setup_qr(&self, draft: Option<&str>) -> Result<String, SchoolAuthError> {
        let code = qrcode::QrCode::new(self.otp_setup_link(draft)?)
            .map_err(|_| SchoolAuthError::invalid("OTP setup is too long."))?;
        Ok(code
            .render::<qrcode::render::svg::Color>()
            .min_dimensions(240, 240)
            .build())
    }

    pub fn matrix_text(&self) -> Result<String, SchoolAuthError> {
        let credentials = self.load(SchoolAuthProfile::TokyoTech)?.ok_or_else(|| {
            SchoolAuthError::invalid("Save the Tokyo Tech Matrix before copying it.")
        })?;
        matrix_plain_text(&credentials.matrix)
    }
}

const MATRIX_ROWS: usize = 7;
const MATRIX_COLUMNS: usize = 10;

/// A paste-friendly rendering in the same row-major order as the editor:
/// A1…J1, A2…J2, …, A7…J7.
fn matrix_plain_text(matrix: &str) -> Result<String, SchoolAuthError> {
    let cells: Vec<char> = matrix.chars().collect();
    if cells.len() != MATRIX_ROWS * MATRIX_COLUMNS {
        return Err(SchoolAuthError::invalid(
            "Matrix must contain 7 rows × 10 columns.",
        ));
    }
    Ok(cells
        .chunks(MATRIX_COLUMNS)
        .map(|row| row.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("\n"))
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

fn otp_config(raw: &str) -> Result<OtpConfig, SchoolAuthError> {
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
fn otp_setup_uri(setup: &str, account: &str) -> String {
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

fn otp_at(raw: &str, timestamp: u64) -> Result<String, SchoolAuthError> {
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
pub fn upcoming_otps(raw: &str) -> Result<Vec<OtpWindow>, SchoolAuthError> {
    let period = otp_config(raw)?.period;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| SchoolAuthError::storage("System time is unavailable."))?
        .as_secs();
    let start = now - now % period;
    // The login flow waits across the current window and at most three successors.
    (0..=3)
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
#[path = "tests/school_auth.rs"]
mod tests;
