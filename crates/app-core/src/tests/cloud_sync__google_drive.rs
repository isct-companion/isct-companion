use super::*;
use crate::ports::SecretError;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
struct MemoryVault(Mutex<HashMap<String, String>>);

impl SecretVault for MemoryVault {
    fn store(&self, account: &str, secret: &str) -> Result<(), SecretError> {
        self.0.lock().unwrap().insert(account.into(), secret.into());
        Ok(())
    }

    fn load(&self, account: &str) -> Result<Option<String>, SecretError> {
        Ok(self.0.lock().unwrap().get(account).cloned())
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        self.0.lock().unwrap().remove(account);
        Ok(())
    }
}

fn desktop() -> Pkce<MemoryVault> {
    Pkce::new(
        MemoryVault::default(),
        OAuthClient {
            id: "desktop.apps.googleusercontent.com".into(),
            secret: Some("desktop-secret".into()),
        },
    )
    .unwrap()
}

const LOOPBACK: &str = "http://127.0.0.1:42813/oauth/callback";
const IOS: &str = "com.googleusercontent.apps.ios:/oauthredirect";

/// A callback must answer the authorization it was started for: the exact
/// redirect, the same state, a code and no error. A forged or replayed
/// redirect must never reach the token endpoint.
#[test]
fn callback_answers_only_its_own_authorization() {
    let pkce = desktop();
    let pending = pkce
        .begin("desktop.apps.googleusercontent.com", LOOPBACK)
        .unwrap();
    let url = Url::parse(&pending.url).unwrap();
    let query: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query["scope"], SCOPE);
    assert_eq!(query["redirect_uri"], LOOPBACK);
    assert_eq!(query["code_challenge_method"], "S256");
    assert_eq!(query["access_type"], "offline");
    let state = &query["state"];
    assert!(state.len() >= 32);
    assert!((43..=128).contains(&pending.verifier.len()));

    let answer = |target: &str| callback_code(&pending, target);
    assert_eq!(
        answer(&format!("{LOOPBACK}?state={state}&code=abc")).unwrap(),
        "abc"
    );
    for wrong in [
        format!("{LOOPBACK}?state=other&code=abc"),
        format!("{LOOPBACK}?state={state}&error=access_denied"),
        format!("{LOOPBACK}?state={state}"),
        format!("http://127.0.0.1:1/oauth/callback?state={state}&code=abc"),
        format!("http://127.0.0.1:42813/elsewhere?state={state}&code=abc"),
        "not a url".into(),
    ] {
        assert!(answer(&wrong).is_err(), "{wrong}");
    }

    let pending = pkce.begin("ios.apps.googleusercontent.com", IOS).unwrap();
    let state = Url::parse(&pending.url)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "state")
        .unwrap()
        .1
        .into_owned();
    assert_eq!(
        callback_code(&pending, &format!("{IOS}?state={state}&code=ios")).unwrap(),
        "ios"
    );
    assert!(pkce.begin("", LOOPBACK).is_err());
}

/// The desktop client's secret goes only with that client: a development
/// override or the iOS client must not carry it to Google.
#[test]
fn client_secret_is_sent_only_with_its_own_client() {
    let pkce = desktop();
    assert_eq!(
        pkce.secret_for("desktop.apps.googleusercontent.com"),
        Some("desktop-secret")
    );
    assert_eq!(pkce.secret_for("override.apps.googleusercontent.com"), None);
    let ios = Pkce::new(
        MemoryVault::default(),
        OAuthClient {
            id: "ios.apps.googleusercontent.com".into(),
            secret: None,
        },
    )
    .unwrap();
    assert_eq!(ios.secret_for("ios.apps.googleusercontent.com"), None);
}

/// A desktop's saved Drive authorization (the vault format the shell wrote
/// before Drive moved into Core) is still recognized, refreshed only when
/// expired, and disconnecting removes only the Drive account.
#[tokio::test]
async fn saved_authorization_is_read_and_disconnect_removes_only_drive() {
    let pkce = desktop();
    assert!(!pkce.authorized().unwrap());
    pkce.vault()
        .store(
            VAULT_ACCOUNT,
            &serde_json::json!({
                "access_token": "access",
                "refresh_token": "refresh",
                "expires_at": now() + 600,
                "client_id": "desktop.apps.googleusercontent.com",
            })
            .to_string(),
        )
        .unwrap();
    pkce.vault().store("icloud", "connected").unwrap();
    assert!(pkce.authorized().unwrap());
    assert_eq!(pkce.access_token(false).await.unwrap(), "access");

    DriveAuthorization::disconnect(&pkce).unwrap();
    assert!(!pkce.authorized().unwrap());
    assert_eq!(
        pkce.vault().load("icloud").unwrap().as_deref(),
        Some("connected")
    );
    assert!(pkce.access_token(false).await.is_err());
}

/// A shell with several stores routes each provider to its own; a provider
/// it has no store for is an error, not another provider's data.
#[tokio::test]
async fn providers_route_each_provider_to_its_own_store() {
    struct Named(&'static str);
    impl WorkspaceTransport for Named {
        fn read<'a>(&'a self, _: CloudProvider) -> BoxFuture<'a, Option<CloudWorkspaceFile>> {
            Box::pin(async move { Err(DocumentError::Transport(self.0.into())) })
        }
        fn write<'a>(
            &'a self,
            _: CloudProvider,
            _: &'a CloudWorkspaceFile,
            _: Option<u64>,
        ) -> BoxFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }
        fn disconnect(&self, _: CloudProvider) -> Result<(), DocumentError> {
            Ok(())
        }
    }
    let routed = document::Providers::default()
        .with(CloudProvider::Icloud, Named("icloud"))
        .with(CloudProvider::GoogleDrive, Named("drive"));
    for (provider, name) in [
        (CloudProvider::Icloud, "icloud"),
        (CloudProvider::GoogleDrive, "drive"),
    ] {
        assert_eq!(
            routed.read(provider).await.unwrap_err(),
            DocumentError::Transport(name.into())
        );
    }
    // A provider with no transport routed refuses rather than doing nothing.
    assert!(document::Providers::default()
        .with(CloudProvider::Icloud, Named("icloud"))
        .disconnect(CloudProvider::GoogleDrive)
        .is_err());
}
