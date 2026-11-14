mod otp_enrollment;
pub(crate) use otp_enrollment::EnrollmentError;
mod otp_import;
pub use otp_import::{import_otp, OtpImportAccount, OtpImportError};

use crate::lifecycle::SchoolSite;
use crate::ports::{SecretError, SecretVault};
use crate::school_session::SchoolSignInProblem;
use crate::secret_parts;
use ring::hmac;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
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
    /// Signing out did not finish (what is left is named); the credentials
    /// stay saved, and signing out again redoes every step.
    #[error("Signing out did not finish: {0}")]
    SignOutIncomplete(String),
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
    fn pending_key(self) -> &'static str {
        match self {
            Self::Science => "school-sign-out-science",
            Self::TokyoTech => "school-sign-out-tokyo-tech",
        }
    }

    fn acceptance_key(self) -> &'static str {
        match self {
            Self::Science => "school-accepted-science",
            Self::TokyoTech => "school-accepted-tokyo-tech",
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
    /// Credentials to keep: everything a sign-in on Core's own needs, the
    /// Science Tokyo App OTP setup included. Only a sign-in the school
    /// accepted saves them, and at Science Tokyo that one always sent it.
    fn validate(&mut self, profile: SchoolAuthProfile) -> Result<(), SchoolAuthError> {
        self.validate_input(profile)?;
        if profile == SchoolAuthProfile::Science && self.otp_secret.is_empty() {
            return Err(SchoolAuthError::invalid(
                "Science Tokyo OTP setup link or secret is required.",
            ));
        }
        Ok(())
    }

    /// What a Connect may send: the Science Tokyo OTP setup may be left
    /// out, and the school's pages then say what they need (an emailed code
    /// for an account without App OTP, the App OTP otherwise).
    fn validate_input(&mut self, profile: SchoolAuthProfile) -> Result<(), SchoolAuthError> {
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
                if !self.otp_secret.is_empty() {
                    otp_config(&self.otp_secret)?;
                }
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

/// The vault record of a token. Device-only (see `ports::vault_item`).
#[derive(Deserialize, Serialize)]
struct StoredMoodleToken {
    version: u8,
    token: String,
}

const MOODLE_TOKEN_VERSION: u8 = 2;

/// Only the Science profile has an LMS.
fn moodle_token_account(profile: SchoolAuthProfile) -> Option<&'static str> {
    match profile {
        SchoolAuthProfile::Science => Some("lms-token-science"),
        SchoolAuthProfile::TokyoTech => None,
    }
}

/// The device-only item that keeps the account's school session
/// (`school_session::store`).
fn session_account(profile: SchoolAuthProfile) -> &'static str {
    match profile {
        SchoolAuthProfile::Science => "school-session-science",
        SchoolAuthProfile::TokyoTech => "school-session-tokyo-tech",
    }
}

/// The synchronized item: credentials the school accepted on some device.
#[derive(Deserialize, Serialize)]
struct SchoolCredentialBundle {
    version: u8,
    accounts: HashMap<SchoolAuthProfile, SchoolCredentials>,
}

/// Device-local: the school's refusal of one exact credentials value, kept
/// so a relaunch never sends a refused password again. Never synchronized:
/// another device hears it from the school itself.
const SCHOOL_REFUSAL_ACCOUNT: &str = "school-refusals";
const REFUSAL_VERSION: u8 = 1;
/// The device item `school-refusals` replaced. It may still hold a typed
/// password and OTP secret no sign-in accepted: deleted once per launch,
/// with the first device-local read, and never read.
const RETIRED_CANDIDATE_ACCOUNT: &str = "school-candidates";

#[derive(Default, Deserialize, Serialize)]
struct RefusalBundle {
    version: u8,
    accounts: HashMap<SchoolAuthProfile, Refusal>,
}

#[derive(Clone, Deserialize, Serialize)]
struct Refusal {
    /// `digest` of the credentials the school refused; other credentials
    /// (changed here or on another device) are not refused.
    credentials: String,
    problem: SchoolSignInProblem,
    /// It came after an acceptance whose marker could not be removed: the
    /// marker says nothing about this refusal, and a relaunch keeps it. Only
    /// an acceptance that rewrites the record ends it then, so a marker stuck
    /// twice holds accepted credentials back until the next Retry: never the
    /// other way round, sending a refused password again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    after_marker: bool,
}

/// The verified credentials a sign-in on its own uses, and why the school
/// turned them down if it already did.
#[derive(Clone, Debug)]
pub(crate) struct SignInCredentials {
    pub credentials: SchoolCredentials,
    pub refusal: Option<SchoolSignInProblem>,
}

#[derive(Default)]
struct Accounts {
    /// The synchronized payload `verified` was parsed from; `None` before the
    /// first read in this process.
    payload: Option<Option<String>>,
    verified: HashMap<SchoolAuthProfile, SchoolCredentials>,
    /// Read once: only this process writes the device-local item.
    refusals: Option<HashMap<SchoolAuthProfile, Refusal>>,
    /// Another device may have replaced the synchronized item since it was
    /// read (`refresh`).
    stale: bool,
    unsaved_refusals: HashSet<SchoolAuthProfile>,
}

impl Accounts {
    fn refusals(&self) -> HashMap<SchoolAuthProfile, Refusal> {
        self.refusals.clone().unwrap_or_default()
    }

    /// The refusal of the verified credentials, if one is kept.
    fn refusal(&self, profile: SchoolAuthProfile) -> Option<&Refusal> {
        let verified = self.verified.get(&profile)?;
        self.refusals
            .as_ref()?
            .get(&profile)
            .filter(|refusal| refusal.credentials == digest(verified))
    }
}

fn digest(credentials: &SchoolCredentials) -> String {
    use sha2::{Digest, Sha256};
    let text = serde_json::to_string(credentials).unwrap_or_default();
    data_encoding::HEXLOWER.encode(&Sha256::digest(text.as_bytes()))
}

pub struct SchoolAuthService {
    vault: Arc<dyn SecretVault>,
    // Only this native process holds unlocked credentials; never persisted here.
    accounts: Mutex<Accounts>,
    /// The refusal each account shows, as the vault was last read. Never
    /// locked across vault I/O, so drawing it never waits for the vault.
    shown: Mutex<HashMap<SchoolAuthProfile, (SchoolSignInProblem, bool)>>,
    /// Held across a saved session's vault I/O: two writes spread over
    /// several items must not interleave (`secret_parts`).
    session_items: Mutex<()>,
    status_path: Option<std::path::PathBuf>,
}

impl SchoolAuthService {
    #[cfg(test)]
    pub fn new(vault: impl SecretVault + 'static) -> Self {
        Self {
            vault: Arc::new(vault),
            accounts: Mutex::new(Accounts::default()),
            shown: Mutex::default(),
            session_items: Mutex::default(),
            status_path: None,
        }
    }

    pub fn with_status_cache(vault: impl SecretVault + 'static, path: std::path::PathBuf) -> Self {
        Self {
            vault: Arc::new(vault),
            accounts: Mutex::new(Accounts::default()),
            shown: Mutex::default(),
            session_items: Mutex::default(),
            status_path: Some(path),
        }
    }
    // The desktop and mobile shells have a local metadata directory. Pathless
    // embedders keep these non-secret markers in their device-local vault.
    fn marker(&self, key: &str) -> Result<bool, SchoolAuthError> {
        match &self.status_path {
            Some(path) => path
                .with_file_name(key)
                .try_exists()
                .map_err(|error| SchoolAuthError::storage(error.to_string())),
            None => Ok(self.vault.load(key)?.is_some()),
        }
    }

    fn set_marker(&self, key: &str, present: bool) -> Result<(), SchoolAuthError> {
        if let Some(path) = &self.status_path {
            let path = path.with_file_name(key);
            if present {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|error| SchoolAuthError::storage(error.to_string()))?;
                }
                return crate::storage::durable_replace(&path, b"1")
                    .map_err(|error| SchoolAuthError::storage(error.to_string()));
            }
            return match std::fs::remove_file(path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    Err(SchoolAuthError::storage(error.to_string()))
                }
                _ => Ok(()),
            };
        }
        if present {
            self.vault.store(key, "1")?;
        } else {
            self.vault.delete(key)?;
        }
        Ok(())
    }

    pub(crate) fn sign_out_pending(&self, profile: SchoolAuthProfile) -> bool {
        self.marker(profile.pending_key()).unwrap_or_else(|error| {
            log::warn!("Cannot read local sign-out state: {error}");
            true
        })
    }

    pub(crate) fn mark_sign_out(
        &self,
        profile: SchoolAuthProfile,
        pending: bool,
    ) -> Result<(), SchoolAuthError> {
        self.set_marker(profile.pending_key(), pending)
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
            })
            .unwrap_or_else(|| {
                if availability == CredentialAvailability::AccessDenied
                    || (profile == SchoolAuthProfile::Science
                        && availability == CredentialAvailability::NotConfigured
                        && previous.has_otp
                        && !previous.saved)
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
        let result = (|| -> Result<(), String> {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            let bytes = serde_json::to_vec(&status).map_err(|error| error.to_string())?;
            crate::storage::durable_replace(path, &bytes).map_err(|error| error.to_string())
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
    /// can replace it. Core calls this only when the person retries, saves
    /// credentials or imports a credential file: the moments a password
    /// changed elsewhere is found (`approach/school-credential-change-discovery`).
    pub(crate) fn refresh(&self) {
        if let Ok(mut accounts) = self.accounts.lock() {
            accounts.stale = true;
        }
    }

    // The mutex covers the complete read-modify-write operation, so saving one
    // account cannot overwrite a concurrent update to the other account. The
    // synchronized item is read at its first use and again after `refresh`,
    // on every platform; nothing reads it at launch.
    fn unlock_accounts(&self, accounts: &mut Accounts) -> Result<(), SchoolAuthError> {
        if accounts.payload.is_none() || accounts.stale {
            let payload = self.vault.load(SCHOOL_CREDENTIAL_ACCOUNT)?;
            self.adopt(accounts, payload)?;
        }
        accounts.stale = false;
        Ok(())
    }

    /// Keeps the refusals `refusal` shows in step with `accounts`: checked
    /// against the verified credentials once those were read, as recorded
    /// before that.
    fn note(&self, accounts: &Accounts) {
        let shown = [SchoolAuthProfile::Science, SchoolAuthProfile::TokyoTech]
            .into_iter()
            .filter_map(|profile| {
                let refusal = if accounts.payload.is_some() {
                    accounts.refusal(profile)
                } else {
                    accounts.refusals.as_ref()?.get(&profile)
                };
                Some((
                    profile,
                    (
                        refusal?.problem.clone(),
                        accounts.unsaved_refusals.contains(&profile),
                    ),
                ))
            })
            .collect();
        if let Ok(mut current) = self.shown.lock() {
            *current = shown;
        }
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
        self.read_refusals(accounts)?;
        // A refusal of credentials that are no longer the verified ones
        // says nothing any more.
        let mut refusals = accounts.refusals();
        let before = refusals.len();
        refusals.retain(|profile, refusal| {
            accounts
                .verified
                .get(profile)
                .is_some_and(|verified| digest(verified) == refusal.credentials)
        });
        if refusals.len() != before {
            self.drop_refusals(accounts, refusals);
        }
        Ok(())
    }

    /// Keeps `refusals`, fewer than before, in memory at once and writes
    /// them. A failed write overturns nothing already saved and holds no
    /// credentials back: what is left on disk is about other credentials,
    /// or rewritten whole by the next write of the refusals (an accepted
    /// Retry included).
    fn drop_refusals(
        &self,
        accounts: &mut Accounts,
        refusals: HashMap<SchoolAuthProfile, Refusal>,
    ) {
        if let Err(error) = self.write_refusals(&refusals) {
            log::warn!(target: "app_core::school_auth", "a refusal that no longer applies was not deleted: {error}");
        } else {
            accounts.unsaved_refusals.clear();
        }
        accounts
            .unsaved_refusals
            .retain(|profile| refusals.contains_key(profile));
        accounts.refusals = Some(refusals);
    }

    fn read_refusals(&self, accounts: &mut Accounts) -> Result<(), SchoolAuthError> {
        if accounts.refusals.is_some() {
            return Ok(());
        }
        if let Err(error) = self.vault.delete(RETIRED_CANDIDATE_ACCOUNT) {
            log::warn!(target: "app_core::school_auth", "the retired candidates item was not deleted: {error}");
        }
        // A record of another version is not read: the next refusal
        // replaces it.
        let refusals = match self.vault.load(SCHOOL_REFUSAL_ACCOUNT)? {
            Some(payload) => serde_json::from_str::<RefusalBundle>(&payload)
                .ok()
                .filter(|bundle| bundle.version == REFUSAL_VERSION)
                .map(|bundle| bundle.accounts)
                .unwrap_or_default(),
            None => HashMap::new(),
        };
        let mut active = refusals;
        for profile in [SchoolAuthProfile::Science, SchoolAuthProfile::TokyoTech] {
            let liftable = active
                .get(&profile)
                .is_some_and(|refusal| !refusal.after_marker);
            if liftable && self.marker(profile.acceptance_key())? {
                active.remove(&profile);
            }
        }
        accounts.refusals = Some(active);
        Ok(())
    }

    /// The matching credentials were accepted. If the refusal item cannot
    /// be updated, persist the acceptance before reporting success.
    fn accepted(
        &self,
        accounts: &mut Accounts,
        profile: SchoolAuthProfile,
    ) -> Result<(), SchoolAuthError> {
        let mut remaining = accounts.refusals();
        if remaining.remove(&profile).is_some() {
            if self.write_refusals(&remaining).is_err() {
                self.set_marker(profile.acceptance_key(), true)?;
                accounts.unsaved_refusals.remove(&profile);
            } else {
                accounts.unsaved_refusals.clear();
            }
            accounts.refusals = Some(remaining);
        }
        self.note(accounts);
        Ok(())
    }

    fn write_refusals(
        &self,
        refusals: &HashMap<SchoolAuthProfile, Refusal>,
    ) -> Result<(), SchoolAuthError> {
        if refusals.is_empty() {
            return self
                .vault
                .delete(SCHOOL_REFUSAL_ACCOUNT)
                .map_err(SchoolAuthError::from);
        }
        let payload = serde_json::to_string(&RefusalBundle {
            version: REFUSAL_VERSION,
            accounts: refusals.clone(),
        })
        .map_err(|_| SchoolAuthError::storage("The school's refusal could not be encoded."))?;
        self.vault
            .store(SCHOOL_REFUSAL_ACCOUNT, &payload)
            .map_err(SchoolAuthError::from)
    }

    fn write_accounts(
        &self,
        accounts: &HashMap<SchoolAuthProfile, SchoolCredentials>,
    ) -> Result<String, SchoolAuthError> {
        let payload = serde_json::to_string(&SchoolCredentialBundle {
            version: 1,
            accounts: accounts.clone(),
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
        })
        .map_err(|_| SchoolAuthError::storage("School credentials could not be encoded."))
    }

    /// Validates and replaces the native credential bundle after Rust has
    /// authenticated and decrypted it. No plaintext is returned to the UI.
    pub(crate) fn import_sync_payload(
        &self,
        payload: &str,
    ) -> Result<Vec<SchoolAuthProfile>, SchoolAuthError> {
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
        accounts.stale = true;
        self.unlock_accounts(&mut accounts)?;
        let payload = self.write_accounts(&bundle.accounts)?;
        // Drops the refusals of the credentials the import replaced.
        self.adopt(&mut accounts, Some(payload))?;
        self.note(&accounts);
        for profile in [SchoolAuthProfile::Science, SchoolAuthProfile::TokyoTech] {
            let credentials = accounts.verified.get(&profile).cloned();
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
        Ok(bundle.accounts.into_keys().collect())
    }

    /// The school accepted `credentials`: they become the verified,
    /// synchronized credentials, and a refusal of the profile goes. Only the
    /// credentials' write decides the result: a refusal that could not be
    /// deleted is gone from memory all the same (`drop_refusals`).
    pub(crate) fn promote(
        &self,
        profile: SchoolAuthProfile,
        credentials: &SchoolCredentials,
    ) -> Result<(), SchoolAuthError> {
        // Only what a later sign-in on its own can use is kept.
        credentials.clone().validate(profile)?;
        let mut accounts = self.lock()?;
        self.unlock_accounts(&mut accounts)?;
        if accounts.verified.get(&profile) != Some(credentials) {
            let mut verified = accounts.verified.clone();
            verified.insert(profile, credentials.clone());
            let payload = self.write_accounts(&verified)?;
            self.adopt(&mut accounts, Some(payload))?;
        }
        self.accepted(&mut accounts, profile)?;
        if profile == SchoolAuthProfile::Science {
            let _ = self.delete_enrollment();
        }
        self.record_summary(
            profile,
            Some(credentials),
            CredentialAvailability::Available,
        );
        Ok(())
    }

    /// The school turned `credentials` down, or wants the person on its
    /// page: kept with exactly these credentials across launches, while they
    /// are the verified ones. Returns whether they are the verified ones. The
    /// refusal holds them back in memory first, under the same lock, and
    /// then goes to the vault: an `Err` is a refusal held in this process
    /// that the vault did not keep (a relaunch may not know it).
    pub(crate) fn record_refusal(
        &self,
        profile: SchoolAuthProfile,
        credentials: &SchoolCredentials,
        problem: &SchoolSignInProblem,
    ) -> Result<bool, SchoolAuthError> {
        let mut accounts = self.lock()?;
        self.unlock_accounts(&mut accounts)?;
        if accounts.verified.get(&profile) != Some(credentials) {
            return Ok(false);
        }
        let mut refusals = accounts.refusals();
        let mut refusal = Refusal {
            credentials: digest(credentials),
            problem: problem.clone(),
            after_marker: false,
        };
        refusals.insert(profile, refusal.clone());
        accounts.refusals = Some(refusals.clone());
        accounts.unsaved_refusals.insert(profile);
        self.note(&accounts);
        // An acceptance marker left by an earlier acceptance would lift the
        // refusal at the next launch. One that cannot be removed (or read)
        // goes on record with it, so it lifts nothing.
        let key = profile.acceptance_key();
        if self.marker(key).unwrap_or(true) {
            if let Err(error) = self.set_marker(key, false) {
                log::warn!(target: "app_core::school_auth", "an old acceptance marker was not removed: {error}");
                refusal.after_marker = true;
                refusals.insert(profile, refusal);
                accounts.refusals = Some(refusals.clone());
            }
        }
        // The whole record: once it lands, every refusal in it is kept.
        self.write_refusals(&refusals)?;
        accounts.unsaved_refusals.clear();
        self.note(&accounts);
        Ok(true)
    }

    /// The school accepted `credentials` without a Connect (a sign-in on its
    /// own, a Retry): a refusal kept for exactly them goes.
    pub(crate) fn clear_refusal(
        &self,
        profile: SchoolAuthProfile,
        credentials: &SchoolCredentials,
    ) -> Result<(), SchoolAuthError> {
        let mut accounts = self.lock()?;
        self.unlock_accounts(&mut accounts)?;
        let refusals = accounts.refusals();
        if refusals
            .get(&profile)
            .is_none_or(|refusal| refusal.credentials != digest(credentials))
        {
            return Ok(());
        }
        self.accepted(&mut accounts, profile)
    }

    /// The refusal of the profile's credentials, as the vault was last read.
    /// Never touches the vault.
    pub(crate) fn refusal(&self, profile: SchoolAuthProfile) -> Option<SchoolSignInProblem> {
        self.shown
            .lock()
            .ok()?
            .get(&profile)
            .map(|(problem, _)| problem.clone())
    }

    pub(crate) fn refusal_unsaved(&self, profile: SchoolAuthProfile) -> bool {
        self.shown
            .lock()
            .ok()
            .and_then(|shown| shown.get(&profile).map(|(_, unsaved)| *unsaved))
            .unwrap_or(false)
    }

    /// Reads the device-local refusals, without the synchronized item.
    pub(crate) fn load_refusals(&self) -> Result<(), SchoolAuthError> {
        let mut accounts = self.lock()?;
        self.read_refusals(&mut accounts)?;
        self.note(&accounts);
        Ok(())
    }

    /// The verified credentials and their refusal, read now.
    pub(crate) fn sign_in_credentials(
        &self,
        profile: SchoolAuthProfile,
    ) -> Result<Option<SignInCredentials>, SchoolAuthError> {
        let credentials = self.load(profile)?;
        let accounts = self.lock()?;
        self.note(&accounts);
        Ok(credentials.map(|credentials| SignInCredentials {
            refusal: accounts
                .refusal(profile)
                .map(|refusal| refusal.problem.clone()),
            credentials,
        }))
    }

    /// Blank fields keep the verified credentials as the vault holds them at
    /// this moment.
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
            if let Some((owner, secret)) = self.enrolled_otp()? {
                if credentials.username.trim().is_empty() {
                    credentials.username.clone_from(&owner);
                }
                if credentials.username.trim() == owner {
                    credentials.otp_secret = secret;
                }
            }
        }
        credentials.validate_input(profile)?;
        Ok(credentials)
    }

    /// The verified credentials.
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
        let credentials = accounts.verified.get(&profile).cloned();
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
        if profile == SchoolAuthProfile::Science {
            self.delete_enrollment()?;
            if let Some(mut status) = self.cached_status() {
                if !status.science.saved {
                    status.science = CredentialSummary::default();
                    self.write_status(&status);
                }
            }
        }
        let payload = self.write_accounts(&verified)?;
        // Drops the profile's refusal with its credentials.
        self.adopt(&mut accounts, Some(payload))?;
        self.note(&accounts);
        drop(accounts);
        // Signing out also forgets the LMS token; students cannot revoke it.
        let _ = self.delete_moodle_token(profile);
        self.record_summary(profile, None, CredentialAvailability::NotConfigured);
        Ok(true)
    }

    /// The profile's stored token. A record this version cannot read is
    /// deleted, and so is one of an account with no saved credentials (a
    /// sign-out that did not finish, a deletion on another device): a token
    /// is never used for an account that is not saved.
    pub fn load_moodle_token(
        &self,
        profile: SchoolAuthProfile,
    ) -> Result<Option<MoodleToken>, SchoolAuthError> {
        let Some(account) = moodle_token_account(profile) else {
            return Ok(None);
        };
        if !self.configured(profile) {
            self.vault.delete(account)?;
            return Ok(None);
        }
        let Some(payload) = self.vault.load(account)? else {
            return Ok(None);
        };
        let stored = serde_json::from_str::<StoredMoodleToken>(&payload)
            .ok()
            .filter(|stored| stored.version == MOODLE_TOKEN_VERSION && !stored.token.is_empty());
        match stored {
            Some(stored) => Ok(Some(MoodleToken(stored.token))),
            None => {
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
            version: MOODLE_TOKEN_VERSION,
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

    /// The account's saved school session, as `school_session` wrote it.
    pub(crate) fn load_session(
        &self,
        profile: SchoolAuthProfile,
    ) -> Result<Option<String>, SchoolAuthError> {
        let _items = self.session_items.lock();
        Ok(secret_parts::load(
            self.vault.as_ref(),
            session_account(profile),
        )?)
    }

    pub(crate) fn store_session(
        &self,
        profile: SchoolAuthProfile,
        session: &str,
    ) -> Result<(), SchoolAuthError> {
        let _items = self.session_items.lock();
        Ok(secret_parts::store(
            self.vault.as_ref(),
            session_account(profile),
            session,
        )?)
    }

    pub(crate) fn delete_session(&self, profile: SchoolAuthProfile) -> Result<(), SchoolAuthError> {
        let _items = self.session_items.lock();
        Ok(secret_parts::delete(
            self.vault.as_ref(),
            session_account(profile),
        )?)
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
            },
            None => CredentialSummary::default(),
        };
        let mut science = summary(self.load(SchoolAuthProfile::Science)?);
        if !science.saved {
            if let Some((owner, _)) = self.enrolled_otp()? {
                science = Self::enrollment_summary(owner);
            }
        }
        let mut status = SchoolCredentialStatus {
            science,
            tokyo_tech: summary(self.load(SchoolAuthProfile::TokyoTech)?),
            ..Default::default()
        };
        status.suggested_admission_year = student_admission_year(&status.tokyo_tech.username);
        self.write_status(&status);
        Ok(status)
    }

    /// Whether the profile's credentials are saved, as `status` says. Only
    /// metadata: reading it never unlocks the vault.
    pub(crate) fn configured(&self, profile: SchoolAuthProfile) -> bool {
        let Ok(status) = self.status() else {
            return false;
        };
        let summary = match profile {
            SchoolAuthProfile::Science => status.science,
            SchoolAuthProfile::TokyoTech => status.tokyo_tech,
        };
        summary.saved
            && !matches!(
                summary.availability,
                CredentialAvailability::NotConfigured | CredentialAvailability::Missing
            )
    }

    fn saved_otp(&self) -> Result<Option<(String, String)>, SchoolAuthError> {
        match self.load(SchoolAuthProfile::Science)? {
            Some(credentials) => Ok(Some((credentials.username, credentials.otp_secret))),
            None => self.enrolled_otp(),
        }
    }

    /// Explicit export only. Native shells write the result directly to the
    /// clipboard instead of returning a saved secret to their WebView.
    pub fn otp_setup_text(&self, draft: Option<&str>) -> Result<String, SchoolAuthError> {
        let text = if let Some(value) = draft.filter(|value| !value.trim().is_empty()) {
            value.trim().replace(' ', "")
        } else {
            self.saved_otp()?.map(|(_, otp)| otp).unwrap_or_default()
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
        let account = if draft.is_some_and(|value| !value.trim().is_empty()) {
            self.status()?.science.username
        } else {
            self.saved_otp()?
                .map(|(owner, _)| owner)
                .unwrap_or_default()
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
