//! The service's `client-config`: the version gate, and the forms' choices
//! and limits. One answer serves both for as long as the service lets it be
//! cached, in a gate of its own so a form need not wait on an upload.
use super::{form, ExamChoices, ExamError, ExamShare};
use reqwest::Method;
use serde_json::Value;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

/// client-config's public Cache-Control max-age.
pub(super) const MAX_AGE: Duration = Duration::from_secs(300);

#[derive(Default)]
pub(super) struct ConfigCache(Mutex<Option<(Instant, Arc<Value>)>>);

/// The lowest app version the service accepts, if it names one. Read on
/// its own, so nothing else in the answer can close the gate.
pub(super) fn min_app_version(config: &Value) -> Result<Option<&str>, ExamError> {
    match config.get("minAppVersion") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(version)) => Ok(Some(version)),
        Some(_) => Err(ExamError::Response),
    }
}

impl ExamShare {
    pub(super) async fn config(&self) -> Result<Arc<Value>, ExamError> {
        let mut cached = self.config.0.lock().await;
        if let Some((_, config)) = cached
            .as_ref()
            .filter(|(fetched, _)| fetched.elapsed() < MAX_AGE)
        {
            return Ok(config.clone());
        }
        let config: Value = self
            .api
            .service(self.service(Method::GET, &["client-config"])?)
            .await?;
        let config = Arc::new(config);
        *cached = Some((Instant::now(), config.clone()));
        Ok(config)
    }

    pub(super) async fn choices(&self) -> Result<ExamChoices, ExamError> {
        form::choices(&*self.config().await?)
    }

    /// The service's limit for one material, which uploads and downloads
    /// keep to.
    pub(super) async fn upload_limit(&self) -> Result<u64, ExamError> {
        form::limits(&*self.config().await?)?
            .max_upload_bytes
            .map(u64::from)
            .ok_or(ExamError::Unsupported)
    }
}
