use super::*;

#[test]
fn only_the_authorization_with_the_same_redirect_and_state_can_supply_a_code() {
    let cases = [
        (
            "http://127.0.0.1:42813/oauth/callback",
            Options {
                scope: "https://www.googleapis.com/auth/drive.appdata",
                prompt: "consent",
                offline: true,
                hosted_domain: None,
                nonce: None,
            },
        ),
        (
            "com.googleusercontent.apps.ios:/oauthredirect",
            Options {
                scope: "openid email profile",
                prompt: "select_account",
                offline: false,
                hosted_domain: Some("m.isct.ac.jp"),
                nonce: Some("login-nonce"),
            },
        ),
    ];
    for (redirect, options) in cases {
        let redirect_url = Url::parse(redirect).unwrap();
        assert!(matches!(
            begin("", redirect_url.clone(), options),
            Err(OAuthError::Configuration)
        ));
        let pending = begin("client.apps.googleusercontent.com", redirect_url, options).unwrap();
        let query = |key: &str| {
            pending
                .url
                .query_pairs()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.into_owned())
        };
        // Google must see a PKCE challenge, and the nonce that binds the ID
        // token to this sign-in when one was asked for.
        assert_eq!(query("code_challenge_method").as_deref(), Some("S256"));
        assert!(query("code_challenge").is_some_and(|challenge| challenge.len() >= 43));
        assert_eq!(query("nonce").as_deref(), options.nonce);
        assert_eq!(query("scope").as_deref(), Some(options.scope));
        let state = query("state").unwrap();
        let callback = |params: &str| Url::parse(&format!("{redirect}?{params}")).unwrap();
        let good = callback(&format!("state={state}&code=abc"));
        assert_eq!(pending.callback_code(&good).unwrap(), "abc");
        assert!(matches!(
            pending.callback_code(&callback("state=wrong&code=abc")),
            Err(OAuthError::State)
        ));
        assert!(matches!(
            pending.callback_code(&callback(&format!("state={state}&error=access_denied"))),
            Err(OAuthError::Cancelled)
        ));
        assert!(pending
            .callback_code(&callback(&format!("state={state}")))
            .is_err());
        let mut wrong_path = good.clone();
        wrong_path.set_path("/elsewhere");
        assert!(pending.callback_code(&wrong_path).is_err());
        if good.port().is_some() {
            let mut wrong_port = good.clone();
            wrong_port.set_port(Some(1)).unwrap();
            assert!(matches!(
                pending.callback_code(&wrong_port),
                Err(OAuthError::Callback)
            ));
        }
    }
}

#[test]
fn an_override_cannot_send_another_clients_secret() {
    let client = OAuthClient {
        id: "desktop".into(),
        secret: Some("secret".into()),
    };
    assert_eq!(client.secret_for("desktop"), Some("secret"));
    assert_eq!(client.secret_for("development"), None);
}
