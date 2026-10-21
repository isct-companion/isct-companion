mod otp_enrollment;
mod otp_import;
pub(crate) mod otp_setup;
pub use otp_import::{import_otp, OtpImportAccount, OtpImportError};
pub use otp_setup::{OtpSetupError, SchoolOtpSetup};

use crate::lifecycle::SchoolSite;
use crate::ports::{SecretError, SecretVault};
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
#[derive(Deserialize, Serialize)]
struct SchoolCredentialBundle {
    version: u8,
    accounts: HashMap<SchoolAuthProfile, SchoolCredentials>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    science_otp: Option<otp_enrollment::EnrollmentOtp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending_science_otp: Option<otp_enrollment::EnrollmentOtp>,
}

pub struct SchoolAuthService {
    vault: Arc<dyn SecretVault>,
    // Only this native process holds unlocked credentials; never persisted here.
    // Missing accounts are cached too, avoiding repeated legacy-keychain probes.
    unlocked: Mutex<HashMap<SchoolAuthProfile, Option<SchoolCredentials>>>,
    status_path: Option<std::path::PathBuf>,
}

impl SchoolAuthService {
    #[cfg(test)]
    pub fn new(vault: impl SecretVault + 'static) -> Self {
        Self {
            vault: Arc::new(vault),
            unlocked: Mutex::new(HashMap::new()),
            status_path: None,
        }
    }

    pub fn with_status_cache(vault: impl SecretVault + 'static, path: std::path::PathBuf) -> Self {
        Self {
            vault: Arc::new(vault),
            unlocked: Mutex::new(HashMap::new()),
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

    // The mutex covers the complete read-modify-write operation, so saving one
    // account cannot overwrite a concurrent update to the other account.
    fn unlock_accounts(
        &self,
        unlocked: &mut HashMap<SchoolAuthProfile, Option<SchoolCredentials>>,
    ) -> Result<(), SchoolAuthError> {
        if unlocked.len() == 2 {
            return Ok(());
        }
        let accounts = if let Some(payload) = self.vault.load(SCHOOL_CREDENTIAL_ACCOUNT)? {
            let bundle: SchoolCredentialBundle = serde_json::from_str(&payload).map_err(|_| {
                SchoolAuthError::compatibility("Stored school credentials are invalid.")
            })?;
            if bundle.version != 1 {
                return Err(SchoolAuthError::compatibility(
                    "Stored school credentials version is unsupported.",
                ));
            }
            bundle.accounts
        } else {
            let mut accounts = HashMap::new();
            for profile in [SchoolAuthProfile::Science, SchoolAuthProfile::TokyoTech] {
                if let Some(payload) = self.vault.load(profile.account())? {
                    let mut credentials: SchoolCredentials = serde_json::from_str(&payload)
                        .map_err(|_| {
                            SchoolAuthError::compatibility("Stored school credentials are invalid.")
                        })?;
                    credentials
                        .validate(profile)
                        .map_err(|error| SchoolAuthError::compatibility(error.to_string()))?;
                    accounts.insert(profile, credentials);
                }
            }
            if !accounts.is_empty() {
                // The bundle was absent above; there is no enrollment to preserve.
                self.write_accounts_with_otp(&accounts, None)?;
                // Verify the replacement before touching either legacy entry.
                let payload = self.vault.load(SCHOOL_CREDENTIAL_ACCOUNT)?.ok_or_else(|| {
                    SchoolAuthError::storage("Credential migration could not be verified")
                })?;
                let stored: SchoolCredentialBundle =
                    serde_json::from_str(&payload).map_err(|_| {
                        SchoolAuthError::storage("Credential migration could not be verified")
                    })?;
                if stored.version != 1 || stored.accounts != accounts {
                    return Err(SchoolAuthError::storage(
                        "Credential migration could not be verified",
                    ));
                }
                for profile in accounts.keys() {
                    let _ = self.vault.delete(profile.account());
                }
            }
            accounts
        };
        let mut validated = HashMap::new();
        for profile in [SchoolAuthProfile::Science, SchoolAuthProfile::TokyoTech] {
            let credentials = accounts
                .get(&profile)
                .cloned()
                .map(|mut credentials| {
                    credentials
                        .validate(profile)
                        .map_err(|error| SchoolAuthError::compatibility(error.to_string()))?;
                    Ok::<_, SchoolAuthError>(credentials)
                })
                .transpose()?;
            validated.insert(profile, credentials);
        }
        *unlocked = validated;
        Ok(())
    }
    fn write_accounts(
        &self,
        accounts: &HashMap<SchoolAuthProfile, SchoolCredentials>,
    ) -> Result<(), SchoolAuthError> {
        let stored = self.enrollment_bundle()?;
        self.write_credential_bundle(accounts, stored.science_otp, stored.pending_science_otp)
    }

    fn write_accounts_with_otp(
        &self,
        accounts: &HashMap<SchoolAuthProfile, SchoolCredentials>,
        science_otp: Option<otp_enrollment::EnrollmentOtp>,
    ) -> Result<(), SchoolAuthError> {
        self.write_credential_bundle(accounts, science_otp, None)
    }

    fn write_credential_bundle(
        &self,
        accounts: &HashMap<SchoolAuthProfile, SchoolCredentials>,
        science_otp: Option<otp_enrollment::EnrollmentOtp>,
        pending_science_otp: Option<otp_enrollment::EnrollmentOtp>,
    ) -> Result<(), SchoolAuthError> {
        let payload = serde_json::to_string(&SchoolCredentialBundle {
            version: 1,
            accounts: accounts.clone(),
            science_otp,
            pending_science_otp,
        })
        .map_err(|_| SchoolAuthError::storage("School credentials could not be encoded."))?;
        self.vault
            .store(SCHOOL_CREDENTIAL_ACCOUNT, &payload)
            .map_err(SchoolAuthError::from)
    }

    /// Returns the validated native credential bundle for Rust-owned encrypted
    /// cloud sync. This value must never cross the WebView IPC boundary.
    pub fn export_sync_payload(&self) -> Result<String, SchoolAuthError> {
        let mut unlocked = self
            .unlocked
            .lock()
            .map_err(|_| SchoolAuthError::storage("Credential cache unavailable"))?;
        self.unlock_accounts(&mut unlocked)?;
        let accounts = unlocked
            .iter()
            .filter_map(|(profile, value)| value.clone().map(|value| (*profile, value)))
            .collect();
        serde_json::to_string(&SchoolCredentialBundle {
            version: 1,
            accounts,
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
        let mut unlocked = self
            .unlocked
            .lock()
            .map_err(|_| SchoolAuthError::storage("Credential cache unavailable"))?;
        if let Some(otp) = &bundle.science_otp {
            if !otp.confirmed || otp.username.is_empty() || otp.username.len() > 128 {
                return Err(SchoolAuthError::invalid(
                    "Synced OTP enrollment is invalid.",
                ));
            }
            otp_config(&otp.setup)?;
        }
        self.write_accounts_with_otp(&bundle.accounts, bundle.science_otp.clone())?;
        self.record_enrollment_summary(None);
        for profile in [SchoolAuthProfile::Science, SchoolAuthProfile::TokyoTech] {
            let credentials = bundle.accounts.get(&profile).cloned();
            unlocked.insert(profile, credentials.clone());
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

    pub fn save(
        &self,
        profile: SchoolAuthProfile,
        credentials: SchoolCredentials,
    ) -> Result<bool, SchoolAuthError> {
        let credentials = self.resolve(profile, credentials)?;
        self.store(profile, &credentials)?;
        Ok(true)
    }

    fn store(
        &self,
        profile: SchoolAuthProfile,
        credentials: &SchoolCredentials,
    ) -> Result<(), SchoolAuthError> {
        let mut unlocked = self
            .unlocked
            .lock()
            .map_err(|_| SchoolAuthError::storage("Credential cache unavailable"))?;
        self.unlock_accounts(&mut unlocked)?;
        if unlocked.get(&profile).and_then(Option::as_ref) == Some(credentials) {
            return Ok(()); // Connect with unchanged credentials must not rewrite Keychain.
        }
        let mut accounts: HashMap<_, _> = unlocked
            .iter()
            .filter_map(|(profile, value)| value.clone().map(|value| (*profile, value)))
            .collect();
        accounts.insert(profile, credentials.clone());
        self.write_accounts(&accounts)?;
        unlocked.insert(profile, Some(credentials.clone()));
        self.record_summary(
            profile,
            Some(credentials),
            CredentialAvailability::Available,
        );
        Ok(())
    }

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
        let mut unlocked = self
            .unlocked
            .lock()
            .map_err(|_| SchoolAuthError::storage("Credential cache unavailable"))?;
        if let Err(error) = self.unlock_accounts(&mut unlocked) {
            self.record_summary(profile, None, CredentialAvailability::AccessDenied);
            return Err(error);
        }
        let credentials = unlocked.get(&profile).cloned().flatten();
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
        let mut unlocked = self
            .unlocked
            .lock()
            .map_err(|_| SchoolAuthError::storage("Credential cache unavailable"))?;
        self.unlock_accounts(&mut unlocked)?;
        let mut accounts: HashMap<_, _> = unlocked
            .iter()
            .filter_map(|(profile, value)| value.clone().map(|value| (*profile, value)))
            .collect();
        accounts.remove(&profile);
        // Keep an empty bundle when forgetting the final account: an old legacy
        // entry whose cleanup failed must never resurrect a forgotten account.
        if profile == SchoolAuthProfile::Science {
            self.write_accounts_with_otp(&accounts, None)?;
        } else {
            self.write_accounts(&accounts)?;
        }
        let _ = self.vault.delete(profile.account());
        unlocked.insert(profile, None);
        drop(unlocked);
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
