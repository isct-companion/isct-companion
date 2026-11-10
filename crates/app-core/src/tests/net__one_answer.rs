//! One school answer for every request, as a maintenance window serves it.
use crate::net::{BoxFuture, Host, NetError, NetRequest, Response};

pub(crate) struct OneAnswer {
    pub(crate) status: u16,
    pub(crate) content_type: &'static str,
    pub(crate) body: &'static str,
}

impl crate::school_session::Transport for OneAnswer {
    fn send(&self, request: NetRequest) -> BoxFuture<'_, Result<Response, NetError>> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static(self.content_type),
        );
        let response = Response {
            status: self.status,
            url: request.url,
            headers,
            body: self.body.as_bytes().to_vec().into(),
        };
        Box::pin(async move { Ok(response) })
    }
    fn report_maintenance(&self, _host: Host) {}
    fn paused_until(&self, _host: Host) -> Option<i64> {
        Some(chrono::Utc::now().timestamp() + 300)
    }
}
