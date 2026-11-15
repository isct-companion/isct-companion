use super::*;
use crate::oauth::code_exchange::{begin, Options};

struct Browser;
impl NativeBrowserAuth for Browser {
    fn authenticate<'a>(
        &'a self,
        _: &'a Url,
        _: BrowserCallback,
    ) -> crate::net::BoxFuture<'a, Result<(), OAuthError>> {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn unrelated_requests_do_not_consume_a_callback_and_success_closes_the_socket() {
    let listener = Loopback::bind(0).await.unwrap();
    let address = listener.listener.local_addr().unwrap();
    let pending = begin(
        "client",
        listener.redirect().clone(),
        Options {
            scope: "openid",
            prompt: "select_account",
            offline: false,
            hosted_domain: None,
            nonce: None,
        },
    )
    .unwrap();
    let state = pending
        .url
        .query_pairs()
        .find(|(key, _)| key == "state")
        .unwrap()
        .1;
    let request = |target: String| async move {
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream
            .write_all(format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        response
    };
    let cancel = CancellationToken::new();
    let (callback, ()) = tokio::join!(listener.authenticate(&Browser, &pending, &cancel), async {
        assert!(request(format!("/wrong?state={state}&code=bad"))
            .await
            .starts_with("HTTP/1.1 400"));
        assert!(request(format!("{PATH}?state=wrong&code=bad"))
            .await
            .starts_with("HTTP/1.1 400"));
        assert!(request(format!("{PATH}?state={state}&code=good"))
            .await
            .starts_with("HTTP/1.1 200"));
    });
    assert!(callback
        .unwrap()
        .query_pairs()
        .any(|(key, value)| key == "code" && value == "good"));
    assert!(TcpStream::connect(address).await.is_err());
}
