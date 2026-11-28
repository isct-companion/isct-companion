//! Stateless Google OAuth code exchange. Callers own any token storage.

use super::OAuthError;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ring::rand::SecureRandom;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use url::Url;

const AUTHORIZE_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";

/// Installed desktop clients require their public client secret; iOS clients
/// have no secret.
#[derive(Clone)]
pub struct OAuthClient {
    pub id: String,
    pub secret: Option<String>,
}

impl OAuthClient {
    pub(crate) fn secret_for(&self, id: &str) -> Option<&str> {
        self.secret
            .as_deref()
            .filter(|secret| id == self.id && !secret.is_empty())
    }

    #[cfg(target_os = "ios")]
    pub(crate) fn ios_redirect(&self) -> Result<Url, OAuthError> {
        let scheme = self.id.rsplit('.').collect::<Vec<_>>().join(".");
        Url::parse(&format!("{scheme}:/oauthredirect")).map_err(|_| OAuthError::Configuration)
    }
}

/// The installed-app ("Desktop") client the desktop and Android builds sign
/// in to Google with. The build scripts supply it from
/// `config/oauth-clients.yaml`; Google requires its secret in token requests
/// even with PKCE and documents it as not confidential for installed apps.
#[cfg(not(target_os = "ios"))]
pub fn installed_client() -> Option<OAuthClient> {
    let id = option_env!("ISCT_GOOGLE_DESKTOP_CLIENT_ID").filter(|id| !id.is_empty())?;
    Some(OAuthClient {
        id: id.into(),
        secret: option_env!("ISCT_GOOGLE_DESKTOP_CLIENT_SECRET").map(str::to_owned),
    })
}

#[derive(Clone, Copy)]
pub struct Options<'a> {
    pub scope: &'a str,
    pub prompt: &'a str,
    pub offline: bool,
    pub(crate) hosted_domain: Option<&'a str>,
    pub(crate) nonce: Option<&'a str>,
}

pub(crate) struct PendingAuthorization {
    pub url: Url,
    redirect: Url,
    verifier: String,
    state: String,
    client_id: String,
}

#[derive(Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub(crate) refresh_token: Option<String>,
    pub(crate) id_token: Option<String>,
    pub(crate) expires_in: Option<u64>,
}

pub(crate) struct CodeExchange(reqwest::Client);

impl CodeExchange {
    pub(crate) fn new() -> Result<Self, OAuthError> {
        crate::net::client_builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(45))
            .build()
            .map(Self)
            .map_err(|_| OAuthError::Configuration)
    }

    pub(crate) async fn exchange(
        &self,
        client: &OAuthClient,
        pending: &PendingAuthorization,
        callback: &Url,
    ) -> Result<Tokens, OAuthError> {
        let code = pending.callback_code(callback)?;
        let mut form = vec![
            ("client_id", pending.client_id.as_str()),
            ("code", code.as_str()),
            ("code_verifier", pending.verifier.as_str()),
            ("redirect_uri", pending.redirect.as_str()),
            ("grant_type", "authorization_code"),
        ];
        if let Some(secret) = client.secret_for(&pending.client_id) {
            form.push(("client_secret", secret));
        }
        self.request(&form).await
    }

    pub(crate) async fn refresh(
        &self,
        client: &OAuthClient,
        client_id: &str,
        refresh_token: &str,
    ) -> Result<Tokens, OAuthError> {
        let mut form = vec![
            ("client_id", client_id),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ];
        if let Some(secret) = client.secret_for(client_id) {
            form.push(("client_secret", secret));
        }
        self.request(&form).await
    }

    async fn request(&self, form: &[(&str, &str)]) -> Result<Tokens, OAuthError> {
        let response = self
            .0
            .post(TOKEN_ENDPOINT)
            .form(form)
            .send()
            .await
            .map_err(|_| OAuthError::Request)?;
        let status = response.status();
        if !status.is_success() {
            let detail = crate::net::read_response_limited(response, 64 * 1024)
                .await
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Rejection>(&bytes).ok())
                .map_or("", |body| match body.error.as_str() {
                    "invalid_client" => " invalid_client",
                    "invalid_grant" => " invalid_grant",
                    "unauthorized_client" => " unauthorized_client",
                    "redirect_uri_mismatch" => " redirect_uri_mismatch",
                    "invalid_request" => " invalid_request",
                    _ => "",
                });
            return Err(OAuthError::Rejected {
                status: status.as_u16(),
                detail,
            });
        }
        let bytes = crate::net::read_response_limited(response, 1024 * 1024)
            .await
            .map_err(|_| OAuthError::Response)?;
        let token: Tokens = serde_json::from_slice(&bytes).map_err(|_| OAuthError::Response)?;
        if token.access_token.is_empty() {
            return Err(OAuthError::Response);
        }
        Ok(token)
    }
}

#[derive(Deserialize)]
struct Rejection {
    error: String,
}

pub(crate) fn begin(
    client_id: &str,
    redirect: Url,
    options: Options<'_>,
) -> Result<PendingAuthorization, OAuthError> {
    if !valid_client_id(client_id) {
        return Err(OAuthError::Configuration);
    }
    let verifier = random_urlsafe::<48>()?;
    let state = random_urlsafe::<32>()?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url = Url::parse(AUTHORIZE_ENDPOINT).map_err(|_| OAuthError::Configuration)?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", redirect.as_str())
            .append_pair("response_type", "code")
            .append_pair("scope", options.scope)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state)
            .append_pair("prompt", options.prompt);
        if options.offline {
            query.append_pair("access_type", "offline");
        }
        if let Some(domain) = options.hosted_domain {
            query.append_pair("hd", domain);
        }
        if let Some(nonce) = options.nonce {
            query.append_pair("nonce", nonce);
        }
    }
    Ok(PendingAuthorization {
        url,
        redirect,
        verifier,
        state,
        client_id: client_id.into(),
    })
}

impl PendingAuthorization {
    fn callback_code(&self, callback: &Url) -> Result<String, OAuthError> {
        let mut target = callback.clone();
        target.set_query(None);
        if target != self.redirect {
            return Err(OAuthError::Callback);
        }
        let mut states = callback.query_pairs().filter(|(key, _)| key == "state");
        if states.next().is_none_or(|(_, state)| state != self.state) || states.next().is_some() {
            return Err(OAuthError::State);
        }
        if callback.query_pairs().any(|(key, _)| key == "error") {
            return Err(OAuthError::Cancelled);
        }
        let mut codes = callback.query_pairs().filter(|(key, _)| key == "code");
        match (codes.next(), codes.next()) {
            (Some((_, code)), None) if !code.is_empty() => Ok(code.into_owned()),
            _ => Err(OAuthError::Callback),
        }
    }

    #[cfg(not(target_os = "ios"))]
    pub(super) fn is_answered_by(&self, callback: &Url) -> bool {
        matches!(
            self.callback_code(callback),
            Ok(_) | Err(OAuthError::Cancelled)
        )
    }
}

pub(crate) fn random_urlsafe<const N: usize>() -> Result<String, OAuthError> {
    let mut value = [0; N];
    ring::rand::SystemRandom::new()
        .fill(&mut value)
        .map_err(|_| OAuthError::Random)?;
    Ok(URL_SAFE_NO_PAD.encode(value))
}

pub(crate) fn valid_client_id(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}

#[cfg(test)]
#[path = "../tests/oauth__code_exchange.rs"]
mod tests;
