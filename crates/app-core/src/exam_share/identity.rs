use super::{
    client::{self, Client, FIREBASE_PROJECT},
    ExamError,
};
use crate::net::Host;
#[cfg(target_os = "ios")]
use crate::oauth::OAuthError;
use crate::oauth::{
    code_exchange::{self, CodeExchange, OAuthClient, Options},
    NativeBrowserAuth,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
#[cfg(target_os = "ios")]
use url::Url;

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct SavedSession {
    pub uid: String,
    pub email: String,
    pub refresh_token: String,
    pub registered: bool,
}

pub(super) struct SessionToken {
    pub saved: SavedSession,
    pub id_token: String,
    pub expires: i64,
    pub allowed: bool,
}

#[derive(Deserialize)]
struct FirebaseClaims {
    sub: String,
    aud: String,
    iss: String,
    exp: i64,
    #[serde(default)]
    admin: bool,
    #[serde(default)]
    allowed: bool,
}

impl SessionToken {
    fn new(saved: SavedSession, id_token: String) -> Result<Self, ExamError> {
        // Firebase issued this token directly over HTTPS. Never use claims
        // from a token read out of the vault or supplied by a shell.
        let claims = id_token
            .split('.')
            .nth(1)
            .and_then(|part| URL_SAFE_NO_PAD.decode(part).ok())
            .and_then(|data| serde_json::from_slice::<FirebaseClaims>(&data).ok())
            .ok_or(ExamError::Response)?;
        if claims.admin {
            return Err(ExamError::Administrator);
        }
        if claims.sub != saved.uid
            || claims.aud != FIREBASE_PROJECT
            || claims.iss != format!("https://securetoken.google.com/{FIREBASE_PROJECT}")
            || claims.exp <= chrono::Utc::now().timestamp()
        {
            return Err(ExamError::Response);
        }
        Ok(Self {
            saved,
            id_token,
            expires: claims.exp,
            allowed: claims.allowed,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignIn {
    local_id: String,
    email: String,
    refresh_token: String,
    id_token: String,
}

#[derive(Deserialize)]
struct Refresh {
    user_id: String,
    refresh_token: String,
    id_token: String,
}

pub(super) async fn refresh(api: &Client, saved: &SavedSession) -> Result<SessionToken, ExamError> {
    let request = api
        .request(Host::FirebaseSecureToken, Method::POST, &["v1", "token"])?
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", saved.refresh_token.as_str()),
        ]);
    let answer: Refresh = api.firebase(request).await?;
    if answer.user_id != saved.uid {
        return Err(ExamError::Response);
    }
    SessionToken::new(
        SavedSession {
            uid: saved.uid.clone(),
            email: saved.email.clone(),
            refresh_token: answer.refresh_token,
            registered: saved.registered,
        },
        answer.id_token,
    )
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

#[cfg(any(
    target_os = "ios",
    target_os = "macos",
    target_os = "windows",
    target_os = "android"
))]
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
