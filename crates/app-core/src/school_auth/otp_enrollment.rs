//! Local vault items survive the gap between Extic's POST and our save.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;

const ENROLLMENT_ACCOUNT: &str = "school-otp-enrollment";
const SAVED_OTP_ACCOUNT: &str = "school-otp-only";
const SCIENCE: SchoolAuthProfile = SchoolAuthProfile::Science;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EnrollmentError {
    Account,
    Storage,
    Invalid,
    Stopped,
}

impl From<SchoolAuthError> for EnrollmentError {
    fn from(_: SchoolAuthError) -> Self {
        Self::Storage
    }
}

#[derive(Deserialize, Serialize)]
struct EnrollmentOtp {
    id: String,
    username: String,
    secret: String,
    /// The full credentials revision this operation began with, if any.
    base: Option<String>,
}

impl SchoolAuthService {
    fn read_enrollment(&self, account: &str) -> Result<Option<EnrollmentOtp>, SchoolAuthError> {
        self.vault
            .load(account)?
            .map(|value| {
                serde_json::from_str(&value).map_err(|_| {
                    SchoolAuthError::compatibility("Stored OTP enrollment is invalid.")
                })
            })
            .transpose()
    }

    fn write_enrollment(&self, account: &str, otp: &EnrollmentOtp) -> Result<(), SchoolAuthError> {
        let payload = serde_json::to_string(otp)
            .map_err(|_| SchoolAuthError::storage("OTP enrollment could not be encoded."))?;
        self.vault.store(account, &payload)?;
        Ok(())
    }

    pub(super) fn delete_enrollment(&self) -> Result<(), SchoolAuthError> {
        self.vault.delete(ENROLLMENT_ACCOUNT)?;
        self.vault.delete(SAVED_OTP_ACCOUNT)?;
        Ok(())
    }

    pub(super) fn enrolled_otp(&self) -> Result<Option<(String, String)>, SchoolAuthError> {
        Ok(self
            .read_enrollment(SAVED_OTP_ACCOUNT)?
            .map(|otp| (otp.username, otp.secret)))
    }

    pub(crate) fn enrollment_account(
        &self,
        username: &str,
        owner: &str,
    ) -> Result<Option<String>, EnrollmentError> {
        if owner.is_empty()
            || owner.len() > 128
            || !owner
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'@'))
            || (!username.is_empty() && username != owner)
        {
            return Err(EnrollmentError::Account);
        }
        let mut accounts = self.lock()?;
        accounts.stale = true;
        self.unlock_accounts(&mut accounts)?;
        if let Some(saved) = accounts.verified.get(&SCIENCE) {
            return (saved.username == owner)
                .then(|| Some(digest(saved)))
                .ok_or(EnrollmentError::Account);
        }
        if self
            .read_enrollment(SAVED_OTP_ACCOUNT)?
            .is_some_and(|otp| otp.username != owner)
        {
            return Err(EnrollmentError::Account);
        }
        Ok(None)
    }

    pub(crate) fn stage_enrollment(
        &self,
        id: &str,
        username: &str,
        base: Option<String>,
        secret: &str,
    ) -> Result<(), EnrollmentError> {
        if secret.len() > 1024 || secret.contains("://") || otp_config(secret).is_err() {
            return Err(EnrollmentError::Invalid);
        }
        if self.enrollment_account(username, username)? != base {
            return Err(EnrollmentError::Account);
        }
        self.write_enrollment(
            ENROLLMENT_ACCOUNT,
            &EnrollmentOtp {
                id: id.into(),
                username: username.into(),
                secret: secret.into(),
                base,
            },
        )?;
        Ok(())
    }

    /// Called only after Extic reports registration for this operation.
    /// Full credentials stay in the existing syncable item. Without a
    /// verified password, keep the OTP alone until a successful Connect.
    pub(crate) fn confirm_enrollment(&self, id: &str) -> Result<bool, EnrollmentError> {
        let mut accounts = self.lock()?;
        accounts.stale = true;
        self.unlock_accounts(&mut accounts)?;
        let pending = self
            .read_enrollment(ENROLLMENT_ACCOUNT)?
            .filter(|otp| otp.id == id)
            .ok_or(EnrollmentError::Stopped)?;
        let saved = accounts.verified.get(&SCIENCE);
        if saved.is_some_and(|saved| saved.username != pending.username)
            || (saved.map(digest) != pending.base
                && saved.is_none_or(|saved| saved.otp_secret != pending.secret))
        {
            return Err(EnrollmentError::Account);
        }
        if let Some(saved) = saved {
            let mut credentials = saved.clone();
            credentials.otp_secret.clone_from(&pending.secret);
            let mut verified = accounts.verified.clone();
            verified.insert(SCIENCE, credentials.clone());
            if &credentials != saved {
                let payload = self.write_accounts(&verified)?;
                self.adopt(&mut accounts, Some(payload))?;
            }
            self.note(&accounts);
            self.record_summary(
                SCIENCE,
                Some(&credentials),
                CredentialAvailability::Available,
            );
            // The verified bundle already took effect; a device-local cleanup
            // failure cannot turn it back into an unsaved credential.
            let _ = self.delete_enrollment();
            Ok(true)
        } else {
            let summary = Self::enrollment_summary(pending.username.clone());
            self.write_enrollment(SAVED_OTP_ACCOUNT, &pending)?;
            let _ = self.vault.delete(ENROLLMENT_ACCOUNT);
            let mut status = self.cached_status().unwrap_or_default();
            status.science = summary;
            self.write_status(&status);
            Ok(false)
        }
    }

    pub(super) fn enrollment_summary(username: String) -> CredentialSummary {
        CredentialSummary {
            username,
            has_otp: true,
            ..CredentialSummary::default()
        }
    }
}
