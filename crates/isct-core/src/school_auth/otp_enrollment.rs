//! Enrollment candidates are durable in the existing encrypted credential
//! bundle, but never become active until the school confirms registration.
use super::*;

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct EnrollmentOtp {
    pub username: String,
    pub setup: String,
    pub operation: String,
    pub confirmed: bool,
}

impl SchoolAuthService {
    pub(super) fn enrollment_bundle(&self) -> Result<SchoolCredentialBundle, SchoolAuthError> {
        match self.vault.load(SCHOOL_CREDENTIAL_ACCOUNT)? {
            Some(payload) => {
                let bundle: SchoolCredentialBundle =
                    serde_json::from_str(&payload).map_err(|_| {
                        SchoolAuthError::compatibility("Stored school credentials are invalid.")
                    })?;
                if bundle.version != 1 {
                    return Err(SchoolAuthError::compatibility(
                        "Stored school credentials version is unsupported.",
                    ));
                }
                Ok(bundle)
            }
            None => Ok(SchoolCredentialBundle {
                version: 1,
                accounts: HashMap::new(),
                science_otp: None,
                pending_science_otp: None,
            }),
        }
    }

    pub(super) fn enrollment_otp(&self) -> Result<Option<EnrollmentOtp>, SchoolAuthError> {
        Ok(self.enrollment_bundle()?.science_otp)
    }

    pub(crate) fn check_enrollment_owner(&self, username: &str) -> Result<(), SchoolAuthError> {
        if username.is_empty() || username.len() > 128 {
            return Err(SchoolAuthError::invalid(
                "School account identity could not be read.",
            ));
        }
        let saved = self.load(SchoolAuthProfile::Science)?;
        let otp = self.enrollment_otp()?;
        if saved
            .as_ref()
            .is_some_and(|saved| saved.username != username)
            || (saved.is_none() && otp.as_ref().is_some_and(|otp| otp.username != username))
        {
            return Err(SchoolAuthError::invalid(
                "The school page is signed in to a different account.",
            ));
        }
        Ok(())
    }

    /// The native enrollment owner stages a candidate before submitting the
    /// school's form. Neither pending nor active secrets cross the UI IPC.
    pub(crate) fn stage_enrollment_otp(
        &self,
        username: &str,
        setup: &str,
        operation: &str,
    ) -> Result<(), SchoolAuthError> {
        otp_config(setup)?;
        self.check_enrollment_owner(username)?;
        if username.is_empty()
            || username.len() > 128
            || !username
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'@'))
        {
            return Err(SchoolAuthError::invalid(
                "School account identity could not be read.",
            ));
        }
        let mut unlocked = self
            .unlocked
            .lock()
            .map_err(|_| SchoolAuthError::storage("Credential cache unavailable"))?;
        self.unlock_accounts(&mut unlocked)?;
        if unlocked
            .get(&SchoolAuthProfile::Science)
            .and_then(Option::as_ref)
            .is_some_and(|saved| saved.username != username)
        {
            return Err(SchoolAuthError::invalid(
                "The school page is signed in to a different account.",
            ));
        }
        let stored = self.enrollment_bundle()?;
        let accounts = unlocked
            .iter()
            .filter_map(|(profile, value)| value.clone().map(|value| (*profile, value)))
            .collect();
        self.write_credential_bundle(
            &accounts,
            stored.science_otp,
            Some(EnrollmentOtp {
                username: username.into(),
                setup: setup.into(),
                operation: operation.into(),
                confirmed: false,
            }),
        )
    }

    pub(crate) fn confirm_enrollment_otp(
        &self,
        operation: &str,
        username: &str,
    ) -> Result<(), SchoolAuthError> {
        let mut unlocked = self
            .unlocked
            .lock()
            .map_err(|_| SchoolAuthError::storage("Credential cache unavailable"))?;
        self.unlock_accounts(&mut unlocked)?;
        let mut candidate = self
            .enrollment_bundle()?
            .pending_science_otp
            .filter(|otp| otp.operation == operation && otp.username == username)
            .ok_or_else(|| {
                SchoolAuthError::invalid("The OTP enrollment no longer belongs to this operation.")
            })?;
        let mut accounts: HashMap<_, _> = unlocked
            .iter()
            .filter_map(|(profile, value)| value.clone().map(|value| (*profile, value)))
            .collect();
        if let Some(saved) = accounts.get_mut(&SchoolAuthProfile::Science) {
            if saved.username != username {
                return Err(SchoolAuthError::invalid(
                    "The school page is signed in to a different account.",
                ));
            }
            saved.otp_secret = candidate.setup.clone();
        }
        candidate.confirmed = true;
        self.write_credential_bundle(&accounts, Some(candidate.clone()), None)?;
        let saved = accounts.get(&SchoolAuthProfile::Science).cloned();
        unlocked.insert(SchoolAuthProfile::Science, saved.clone());
        if let Some(saved) = saved {
            self.record_summary(
                SchoolAuthProfile::Science,
                Some(&saved),
                CredentialAvailability::Available,
            );
        } else {
            self.record_enrollment_summary(Some(&candidate));
        }
        Ok(())
    }

    pub(super) fn record_enrollment_summary(&self, otp: Option<&EnrollmentOtp>) {
        let mut status = self.cached_status().unwrap_or_default();
        if let Some(otp) = otp {
            status.science.username = otp.username.clone();
            status.science.has_otp = true;
            status.science.otp_only = true;
        } else if status.science.otp_only {
            status.science = CredentialSummary::default();
        }
        self.write_status(&status);
    }
}
