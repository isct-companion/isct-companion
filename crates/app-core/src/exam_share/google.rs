use super::{
    client::{self, Client},
    identity::{SavedSession, SessionToken},
    ExamError,
};
use crate::net::Host;
#[cfg(target_os = "ios")]
use crate::oauth::OAuthError;
use crate::oauth::{
    code_exchange::{self, CodeExchange, OAuthClient, Options},
    NativeBrowserAuth,
};
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use reqwest::Method;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;
#[cfg(target_os = "ios")]
use url::Url;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignIn {
    local_id: String,
    email: String,
    refresh_token: String,
    id_token: String,
}

#[derive(Deserialize)]
struct GoogleClaims {
    nonce: String,
}

async fn verify_google(
    api: &Client,
    token: &str,
    client_id: &str,
    nonce: &str,
) -> Result<(), ExamError> {
    // Google's Discovery document publishes this fixed JWKS endpoint. Use the
    // signed token's kid only to select one of its keys.
    let header = decode_header(token).map_err(|_| ExamError::Identity)?;
    let kid = header.kid.ok_or(ExamError::Identity)?;
    let response = api
        .http
        .get("https://www.googleapis.com/oauth2/v3/certs")
        .send()
        .await
        .map_err(client::network)?;
    if !response.status().is_success() {
        return Err(ExamError::Unavailable);
    }
    let jwks: JwkSet = client::read(response).await?;
    let key = jwks.find(&kid).ok_or(ExamError::Identity)?;
    let key = DecodingKey::from_jwk(key).map_err(|_| ExamError::Identity)?;
    let mut rules = Validation::new(Algorithm::RS256);
    rules.set_audience(&[client_id]);
    rules.set_issuer(&["https://accounts.google.com", "accounts.google.com"]);
    rules.set_required_spec_claims(&["iss", "aud", "exp", "sub"]);
    let claims = decode::<GoogleClaims>(token, &key, &rules).map_err(|_| ExamError::Identity)?;
    if claims.claims.nonce != nonce {
        return Err(ExamError::Identity);
    }
    Ok(())
}

pub(super) async fn connect(
    api: &Client,
    client: &OAuthClient,
    browser: &dyn NativeBrowserAuth,
    cancel: &CancellationToken,
) -> Result<SessionToken, ExamError> {
    let nonce = code_exchange::random_urlsafe::<32>()?;
    #[cfg(target_os = "ios")]
    let redirect = client.ios_redirect()?;
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "android"))]
    let listener = crate::oauth::loopback::Loopback::bind(0).await?;
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "android"))]
    let redirect = listener.redirect().clone();
    let pending = code_exchange::begin(
        &client.id,
        redirect.clone(),
        Options {
            scope: "openid email profile",
            prompt: "select_account",
            offline: false,
            hosted_domain: Some("m.isct.ac.jp"),
            nonce: Some(&nonce),
        },
    )?;
    #[cfg(target_os = "ios")]
    let callback = {
        use crate::oauth::BrowserCallback;
        let authorize =
            browser.authenticate(&pending.url, BrowserCallback::Scheme(redirect.scheme()));
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(OAuthError::Cancelled),
            answer = authorize => answer,
        };
        if matches!(result, Err(OAuthError::Cancelled)) {
            browser.cancel().await;
        }
        Url::parse(&result?).map_err(|_| OAuthError::Callback)?
    };
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "android"))]
    let callback = listener.authenticate(browser, &pending, cancel).await?;
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(ExamError::Cancelled),
        answer = async {
            let token = CodeExchange::new()?.exchange(client, &pending, &callback).await?
                .id_token.ok_or(ExamError::Identity)?;
            verify_google(api, &token, &client.id, &nonce).await?;
            let post_body: String = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("id_token", &token).append_pair("providerId", "google.com").finish();
            let request = api.request(Host::FirebaseIdentityToolkit, Method::POST, &["v1", "accounts:signInWithIdp"])?
                .json(&serde_json::json!({"requestUri":"http://localhost","postBody":post_body,"returnSecureToken":true}));
            let answer: SignIn = api.firebase(request).await?;
            SessionToken::new(SavedSession { uid: answer.local_id, email: answer.email, refresh_token: answer.refresh_token, registered: false }, answer.id_token)
        } => answer,
    }
}
