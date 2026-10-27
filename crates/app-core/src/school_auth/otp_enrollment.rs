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
        let mut accounts = self.lock()?;
        self.unlock_accounts(&mut accounts)?;
        if accounts
            .active(SchoolAuthProfile::Science)
            .is_some_and(|saved| saved.username != username)
        {
            return Err(SchoolAuthError::invalid(
                "The school page is signed in to a different account.",
            ));
        }
        let stored = self.enrollment_bundle()?;
        let payload = self.write_credential_bundle(
            &accounts.verified,
            stored.science_otp,
            Some(EnrollmentOtp {
                username: username.into(),
                setup: setup.into(),
                operation: operation.into(),
                confirmed: false,
            }),
        )?;
        self.adopt(&mut accounts, Some(payload))
    }

    pub(crate) fn confirm_enrollment_otp(
        &self,
        operation: &str,
        username: &str,
    ) -> Result<(), SchoolAuthError> {
        let mut accounts = self.lock()?;
        self.unlock_accounts(&mut accounts)?;
        let mut candidate = self
            .enrollment_bundle()?
            .pending_science_otp
            .filter(|otp| otp.operation == operation && otp.username == username)
            .ok_or_else(|| {
                SchoolAuthError::invalid("The OTP enrollment no longer belongs to this operation.")
            })?;
        if accounts
            .active(SchoolAuthProfile::Science)
            .is_some_and(|saved| saved.username != username)
        {
            return Err(SchoolAuthError::invalid(
                "The school page is signed in to a different account.",
            ));
        }
        let mut verified = accounts.verified.clone();
        if let Some(saved) = verified.get_mut(&SchoolAuthProfile::Science) {
            saved.otp_secret = candidate.setup.clone();
        }
        candidate.confirmed = true;
        let payload = self.write_credential_bundle(&verified, Some(candidate.clone()), None)?;
        // A credential candidate keeps its other fields with the new secret,
        // against the verified value it now replaces.
        let mut candidates = accounts.candidates().clone();
        if let Some(pending) = candidates.get_mut(&SchoolAuthProfile::Science) {
            pending.credentials.otp_secret = candidate.setup.clone();
            pending.base = digest(verified.get(&SchoolAuthProfile::Science));
            pending.failure = None;
            self.write_candidates(&candidates)?;
        }
        accounts.candidates = Some(candidates);
        self.adopt(&mut accounts, Some(payload))?;
        let saved = accounts.active(SchoolAuthProfile::Science).cloned();
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
