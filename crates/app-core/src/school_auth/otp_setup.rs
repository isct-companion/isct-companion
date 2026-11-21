use super::{
    totp::{otp_config, otp_setup_uri},
    SchoolAuthError, SchoolAuthProfile, SchoolAuthService,
};

impl SchoolAuthService {
    pub(super) fn saved_otp(&self) -> Result<Option<(String, String)>, SchoolAuthError> {
        let saved = self
            .load(SchoolAuthProfile::Science)?
            .map(|credentials| (credentials.username, credentials.otp_secret));
        if saved.is_none() {
            return self.enrolled_otp();
        }
        Ok(saved)
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

    /// Add an OTP to another authenticator only after the school accepted
    /// the full account. Enrollment-only seeds are kept for Connect, not export.
    pub fn otp_setup_link(&self) -> Result<String, SchoolAuthError> {
        if self.sign_out_pending(SchoolAuthProfile::Science) {
            return Err(SchoolAuthError::SignOutIncomplete(
                "OTP export is unavailable while signing out.".into(),
            ));
        }
        let credentials = self.load(SchoolAuthProfile::Science)?.ok_or_else(|| {
            SchoolAuthError::invalid(
                "Connect the school account before adding its OTP to an authenticator.",
            )
        })?;
        Ok(otp_setup_uri(
            &credentials.otp_secret,
            &credentials.username,
        ))
    }

    /// `otp_setup_link` as an SVG QR code, for a phone to scan from the desktop.
    pub fn otp_setup_qr(&self) -> Result<String, SchoolAuthError> {
        let code = qrcode::QrCode::new(self.otp_setup_link()?)
            .map_err(|_| SchoolAuthError::invalid("OTP setup is too long."))?;
        Ok(code
            .render::<qrcode::render::svg::Color>()
            .min_dimensions(240, 240)
            .build())
    }
}

#[cfg(test)]
#[path = "../tests/school_auth__otp_setup.rs"]
mod tests;
