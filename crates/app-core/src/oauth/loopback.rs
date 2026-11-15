//! The callback listener is owned by the authorization future. Dropping it or
//! cancelling its token closes the socket without a detached listener task.

use super::{code_exchange::PendingAuthorization, BrowserCallback, NativeBrowserAuth, OAuthError};
use std::{net::Ipv4Addr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};
use tokio_util::sync::CancellationToken;
use url::Url;

const PATH: &str = "/oauth/callback";
const MAX_REQUEST: usize = 8192;
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(2);
const AUTHORIZATION_TIMEOUT: Duration = Duration::from_secs(180);

pub struct Loopback {
    listener: TcpListener,
    redirect: Url,
}

impl Loopback {
    /// Port 0 selects an ephemeral port. Drive retains its registered port.
    pub async fn bind(port: u16) -> Result<Self, OAuthError> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|_| OAuthError::Loopback)?;
        let address = listener.local_addr().map_err(|_| OAuthError::Loopback)?;
        let redirect = Url::parse(&format!("http://{address}{PATH}"))
            .map_err(|_| OAuthError::Configuration)?;
        Ok(Self { listener, redirect })
    }

    pub fn redirect(&self) -> &Url {
        &self.redirect
    }

    pub async fn authenticate(
        self,
        browser: &dyn NativeBrowserAuth,
        pending: &PendingAuthorization,
        cancel: &CancellationToken,
    ) -> Result<Url, OAuthError> {
        let authenticate = async {
            browser
                .authenticate(&pending.url, BrowserCallback::Loopback)
                .await?;
            self.wait(pending).await
        };
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(OAuthError::Cancelled),
            result = timeout(AUTHORIZATION_TIMEOUT, authenticate) => {
                result.map_err(|_| OAuthError::Timeout)?
            }
        }
    }

    async fn wait(self, pending: &PendingAuthorization) -> Result<Url, OAuthError> {
        loop {
            let (mut stream, _) = self
                .listener
                .accept()
                .await
                .map_err(|_| OAuthError::Loopback)?;
            let callback = timeout(
                CONNECTION_TIMEOUT,
                read_callback(&mut stream, &self.redirect),
            )
            .await
            .ok()
            .flatten()
            .filter(|url| pending.is_answered_by(url));
            let response = if callback.is_some() {
                b"HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\nAuthorization ended. You may close this window.".as_slice()
            } else {
                b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
            };
            let _ = timeout(CONNECTION_TIMEOUT, stream.write_all(response)).await;
            if let Some(callback) = callback {
                return Ok(callback);
            }
        }
    }
}

async fn read_callback(stream: &mut TcpStream, redirect: &Url) -> Option<Url> {
    let mut bytes = [0; MAX_REQUEST];
    let mut used = 0;
    while used < bytes.len() {
        let size = stream.read(&mut bytes[used..]).await.ok()?;
        if size == 0 {
            return None;
        }
        used += size;
        let mut headers = [httparse::EMPTY_HEADER; 48];
        let mut request = httparse::Request::new(&mut headers);
        if request.parse(&bytes[..used]).ok()?.is_complete() {
            let (path, query) = request.path?.split_once('?')?;
            if request.method != Some("GET") || path != PATH || query.contains('#') {
                return None;
            }
            let mut callback = redirect.clone();
            callback.set_query(Some(query));
            return Some(callback);
        }
    }
    None
}

#[cfg(test)]
#[path = "../tests/oauth__loopback.rs"]
mod tests;
