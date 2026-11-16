use super::{
    client::{Client, FIREBASE_PROJECT},
    ExamError,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::Method;
use serde::{Deserialize, Serialize};

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
    pub(super) fn new(saved: SavedSession, id_token: String) -> Result<Self, ExamError> {
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
struct Refresh {
    user_id: String,
    refresh_token: String,
    id_token: String,
}

pub(super) async fn refresh(api: &Client, saved: &SavedSession) -> Result<SessionToken, ExamError> {
    let request = api
        .request(super::client::Endpoint::SecureToken, Method::POST, &["v1", "token"])?
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
