//! The account's lifecycle: signing in, the service's consent, resuming a
//! stored account, the requests sent for it, signing out and deleting it.
use super::*;

impl ExamShare {
    pub(super) async fn connect(
        &self,
        session: &mut Session,
        cancel: &CancellationToken,
    ) -> Result<ExamAnswer, ExamError> {
        let sign_in = self.sign_in.as_ref().ok_or(ExamError::Unavailable)?;
        self.clear(session).await?;
        self.check_version_for_action(session, cancel).await?;
        let token =
            google::connect(&self.api, &sign_in.client, sign_in.browser.as_ref(), cancel).await?;
        // A connection that did not get through keeps nothing, so a signed
        // out page never holds an account to resume later.
        if let Err(error) = interrupt(cancel, self.recheck(session, token)).await {
            self.clear(session).await?;
            return Err(error);
        }
        Ok(self.account_answer(session))
    }

    pub(super) async fn account(&self, session: &mut Session) -> Result<ExamAnswer, ExamError> {
        if session.saved.is_some() && session.token.is_none() {
            let result = self.resume(session).await;
            self.apply_session_error(session, &result).await?;
            if let Err(error) = result {
                let offline = error.temporarily_unavailable()
                    && session.saved.as_ref().is_some_and(|saved| saved.registered);
                let signed_out = matches!(
                    error,
                    ExamError::Expired | ExamError::Denied | ExamError::Administrator
                );
                if !offline && !signed_out {
                    return Err(error);
                }
            }
        }
        Ok(self.account_answer(session))
    }

    pub(super) async fn accept_terms(
        &self,
        session: &mut Session,
    ) -> Result<ExamAnswer, ExamError> {
        if !matches!(session.status, ExamAccountStatus::NeedsConsent) {
            return Err(ExamError::InvalidInput);
        }
        let expires = session.token.as_ref().ok_or(ExamError::SignedOut)?.expires;
        if expires <= crate::clock::now().timestamp() + 60 {
            self.renew(session).await?;
        }
        let register = |session: &Session| -> Result<_, ExamError> {
            let current = session.token.as_ref().ok_or(ExamError::SignedOut)?;
            Ok(self
                .service(Method::POST, &["auth", "register"])?
                .bearer_auth(&current.id_token)
                .json(&json!({})))
        };
        #[derive(Deserialize)]
        struct Registered {
            ok: bool,
        }
        session.register_attempted = true;
        let mut answer = self.api.service::<Registered>(register(session)?).await;
        if matches!(answer, Err(ExamError::Expired)) {
            self.renew(session).await?;
            answer = self.api.service(register(session)?).await;
        }
        if !answer?.ok {
            return Err(ExamError::Response);
        }
        // The token issued after register carries the grant.
        let saved = session
            .token
            .as_ref()
            .ok_or(ExamError::SignedOut)?
            .saved
            .clone();
        let token = self.refresh(session, &saved).await?;
        self.granted(session, token).await?;
        Ok(self.account_answer(session))
    }

    /// Refuse the terms: leave the account and, as the website does, ask the
    /// service to forget one that never registered.
    pub(super) async fn decline(
        &self,
        session: &mut Session,
        cancel: &CancellationToken,
    ) -> Result<ExamAnswer, ExamError> {
        if !matches!(session.status, ExamAccountStatus::NeedsConsent) {
            return Err(ExamError::InvalidInput);
        }
        // A token that expired on the consent page could not ask; signing
        // out goes ahead whatever the refresh gives.
        if session
            .token
            .as_ref()
            .is_some_and(|token| token.expires <= crate::clock::now().timestamp() + 60)
        {
            let _ = interrupt(cancel, self.renew(session)).await;
        }
        if let Some(token) = session
            .token
            .as_ref()
            .filter(|token| !token.saved.registered && !session.register_attempted)
        {
            let _ = interrupt(cancel, self.discard(token)).await;
        }
        self.clear(session).await?;
        Ok(self.account_answer(session))
    }

    /// `POST /api/auth/discard` for `token`'s account, which deletes an
    /// account that never registered. Its answer is not read; callers ignore
    /// its failure.
    pub(super) async fn discard(&self, token: &SessionToken) -> Result<(), ExamError> {
        self.api
            .fire(
                self.service(Method::POST, &["auth", "discard"])?
                    .bearer_auth(&token.id_token)
                    .json(&json!({}))
                    .timeout(DISCARD_TIMEOUT),
            )
            .await
    }

    /// The stored account after a restart: a new token, then the service's
    /// profile, asking `claims/refresh` again only if it refuses.
    pub(super) async fn resume(&self, session: &mut Session) -> Result<(), ExamError> {
        let saved = session.saved.clone().ok_or(ExamError::SignedOut)?;
        let token = self.refresh(session, &saved).await?;
        match self.check_me(session, token.clone()).await {
            Err(ExamError::Denied | ExamError::Consent) => self.recheck(session, token).await,
            result => result,
        }
    }

    /// A new ID token for `saved`'s account. Every refresh goes through here,
    /// so the rotated refresh token is always kept.
    pub(super) async fn refresh(
        &self,
        session: &mut Session,
        saved: &SavedSession,
    ) -> Result<SessionToken, ExamError> {
        let token = identity::refresh(&self.api, saved).await?;
        self.remember(session, &token);
        Ok(token)
    }

    /// `GET /api/me` with `token`: a profile connects the account, none
    /// (while the service grants before consent) leads to the consent page.
    pub(super) async fn check_me(
        &self,
        session: &mut Session,
        mut token: SessionToken,
    ) -> Result<(), ExamError> {
        #[derive(Deserialize)]
        struct Profile {
            profile: Option<UserRole>,
        }
        #[derive(Deserialize)]
        struct UserRole {
            role: String,
        }
        let answer: Profile = self
            .api
            .service(
                self.service(Method::GET, &["me"])?
                    .bearer_auth(&token.id_token),
            )
            .await?;
        match answer.profile {
            Some(profile) if profile.role == "admin" => return Err(ExamError::Administrator),
            Some(_) => {}
            None => {
                self.await_consent(session, token);
                return Ok(());
            }
        }
        token.saved.registered = true;
        session.status = ExamAccountStatus::Connected;
        self.remember(session, &token);
        session.token = Some(token);
        self.update_upload_account(session);
        Ok(())
    }

    /// `check_me` for a token the service has just granted. A refusal now is
    /// the grant not having reached every check yet: try later, keeping the
    /// account, so the account is asked about at most once per action.
    pub(super) async fn granted(
        &self,
        session: &mut Session,
        token: SessionToken,
    ) -> Result<(), ExamError> {
        match self.check_me(session, token).await {
            Err(ExamError::Denied | ExamError::Consent) => Err(ExamError::Unavailable),
            result => result,
        }
    }

    /// Ask `claims/refresh` about `token`'s account and bring the session in
    /// line with the answer: a granted account takes a new token and its
    /// profile, one that has to accept the terms waits on the consent page,
    /// and one the service refuses is told to forget an account that never
    /// registered before it is signed out.
    pub(super) async fn recheck(
        &self,
        session: &mut Session,
        token: SessionToken,
    ) -> Result<(), ExamError> {
        #[derive(Deserialize)]
        struct Claim {
            allowed: bool,
            #[serde(default)]
            eligible: bool,
        }
        let claim: Claim = self
            .api
            .service(
                self.service(Method::POST, &["auth", "claims", "refresh"])?
                    .bearer_auth(&token.id_token)
                    .json(&json!({})),
            )
            .await?;
        if claim.allowed {
            let updated = self.refresh(session, &token.saved).await?;
            return self.granted(session, updated).await;
        }
        if claim.eligible {
            self.await_consent(session, token);
            return Ok(());
        }
        // A registered account refused later (taken off the list) is only
        // signed out here: the service keeps it.
        if !token.saved.registered && !session.register_attempted {
            let _ = self.discard(&token).await;
        }
        Err(ExamError::Denied)
    }

    /// Hold the session on the consent page with `token` in memory.
    pub(super) fn await_consent(&self, session: &mut Session, mut token: SessionToken) {
        token.saved.registered = false;
        session.status = ExamAccountStatus::NeedsConsent;
        self.remember(session, &token);
        session.token = Some(token);
        self.update_upload_account(session);
    }

    /// Keep `token`'s grant: in memory, and in the vault only for a
    /// registered account. An unregistered one does not outlive the app, so
    /// a restart starts the connection over and no stale `registered` record
    /// stays behind.
    pub(super) fn remember(&self, session: &mut Session, token: &SessionToken) {
        session.saved = Some(token.saved.clone());
        // Token rotation remains usable in memory if the keychain is
        // temporarily locked; each successful refresh tries again.
        if !token.saved.registered {
            let _ = self.vault.delete(ACCOUNT);
        } else if let Ok(data) = serde_json::to_string(&token.saved) {
            let _ = self.vault.store(ACCOUNT, &data);
        }
    }

    pub(super) async fn apply_session_error<T>(
        &self,
        session: &mut Session,
        result: &Result<T, ExamError>,
    ) -> Result<(), ExamError> {
        match result {
            Err(ExamError::Expired | ExamError::Denied | ExamError::Administrator) => {
                self.clear(session).await?
            }
            Err(ExamError::Unsupported) => {
                session.status = ExamAccountStatus::Unsupported;
                // If the version gate opens again, re-establish the account
                // through its stored session instead of showing stale state.
                session.token = None;
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) async fn clear(&self, session: &mut Session) -> Result<(), ExamError> {
        session.token = None;
        session.register_attempted = false;
        session.status = ExamAccountStatus::Disconnected;
        session.version_check = None;
        self.upload.settled(false);
        self.vault.delete(ACCOUNT).map_err(|_| ExamError::Storage)?;
        if let Some(saved) = session.saved.take() {
            cache::erase(&self.account_folder(&saved.uid)).await?;
        }
        Ok(())
    }

    /// Send `request` for the connected account. An expired token is renewed
    /// and the request sent again; a refusal as not allowed or not
    /// registered asks `claims/refresh` once, and the request is sent once
    /// more if the account is still connected. Each send is a fresh copy of
    /// `request`, at most three.
    pub(super) async fn call<T: DeserializeOwned>(
        &self,
        session: &mut Session,
        request: reqwest::RequestBuilder,
    ) -> Result<T, ExamError> {
        if session.token.is_none() {
            self.resume(session).await?;
        }
        self.connected(session)?;
        let current = session.token.as_ref().ok_or(ExamError::SignedOut)?;
        if current.expires <= crate::clock::now().timestamp() + 60 {
            self.renew(session).await?;
        }
        let send = |session: &Session| -> Result<_, ExamError> {
            let current = session.token.as_ref().ok_or(ExamError::SignedOut)?;
            Ok(request
                .try_clone()
                .ok_or(ExamError::Response)?
                .bearer_auth(&current.id_token))
        };
        let mut result = self.api.service(send(session)?).await;
        if matches!(result, Err(ExamError::Expired)) {
            self.renew(session).await?;
            result = self.api.service(send(session)?).await;
        }
        if matches!(result, Err(ExamError::Denied | ExamError::Consent)) {
            let current = session.token.clone().ok_or(ExamError::SignedOut)?;
            self.recheck(session, current).await?;
            self.connected(session)?;
            result = match self.api.service(send(session)?).await {
                // Granted just now, yet refused again: try later.
                Err(ExamError::Denied | ExamError::Consent) => Err(ExamError::Unavailable),
                result => result,
            };
        }
        result
    }

    pub(super) fn connected(&self, session: &Session) -> Result<(), ExamError> {
        match session.status {
            ExamAccountStatus::Connected => Ok(()),
            ExamAccountStatus::NeedsConsent => Err(ExamError::Consent),
            ExamAccountStatus::Disconnected | ExamAccountStatus::Unsupported => {
                Err(ExamError::Unsupported)
            }
        }
    }

    /// A new ID token for the current account, keeping its state.
    pub(super) async fn renew(&self, session: &mut Session) -> Result<(), ExamError> {
        let saved = session
            .token
            .as_ref()
            .ok_or(ExamError::SignedOut)?
            .saved
            .clone();
        let token = self.refresh(session, &saved).await?;
        session.token = Some(token);
        Ok(())
    }

    /// Delete the account on the service, as its website's account page
    /// does, then forget it here with everything kept for it.
    pub(super) async fn delete_account(
        &self,
        session: &mut Session,
    ) -> Result<ExamAnswer, ExamError> {
        let _: Value = self
            .call(
                session,
                self.service(Method::POST, &["users", "delete"])?
                    .json(&json!({})),
            )
            .await?;
        let journal = session
            .saved
            .as_ref()
            .map(|saved| self.upload_journal(&saved.uid));
        self.clear(session).await?;
        if let Some(journal) = journal {
            match tokio::fs::remove_file(journal).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(ExamError::Storage),
            }
        }
        Ok(self.account_answer(session))
    }
}
