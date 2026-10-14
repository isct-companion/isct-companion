use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::runtime::Runtime;
use tokio::task::{AbortHandle, JoinHandle};
uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum ProbeError {
    #[error("runtime closed")]
    Closed,
    #[error("request cancelled")]
    Cancelled,
    #[error("request already exists")]
    Duplicate,
    #[error("callback failed: {detail}")]
    Callback { detail: String },
    #[error("runtime failed: {detail}")]
    Runtime { detail: String },
}
impl From<uniffi::UnexpectedUniFFICallbackError> for ProbeError {
    fn from(value: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Callback {
            detail: value.to_string(),
        }
    }
}
#[uniffi::export(rust, foreign)]
pub trait ProbeListener: Send + Sync {
    fn completed(&self, request: u64) -> Result<(), ProbeError>;
}
#[derive(uniffi::Object)]
pub struct Probe {
    runtime: Mutex<Option<Runtime>>,
    jobs: Arc<Mutex<HashMap<u64, AbortHandle>>>,
}
// The bridge owns its task. Dropping a foreign future releases the listener.
struct OwnedTask {
    task: JoinHandle<Result<u64, ProbeError>>,
    jobs: Arc<Mutex<HashMap<u64, AbortHandle>>>,
    request: u64,
}
impl Drop for OwnedTask {
    fn drop(&mut self) {
        self.task.abort();
        if let Ok(mut jobs) = self.jobs.lock() {
            jobs.remove(&self.request);
        }
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        if let Ok(runtime) = self.runtime.get_mut() {
            if let Some(runtime) = runtime.take() {
                runtime.shutdown_background();
            }
        }
    }
}
#[uniffi::export]
impl Probe {
    #[uniffi::constructor]
    pub fn new() -> Result<Arc<Self>, ProbeError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|error| ProbeError::Runtime {
                detail: error.to_string(),
            })?;
        Ok(Arc::new(Self {
            runtime: Mutex::new(Some(runtime)),
            jobs: Arc::new(Mutex::new(HashMap::new())),
        }))
    }
    pub async fn run(
        &self,
        request: u64,
        delay_ms: u64,
        listener: Arc<dyn ProbeListener>,
    ) -> Result<u64, ProbeError> {
        let task = {
            let runtime = self.runtime.lock().map_err(|_| ProbeError::Closed)?;
            let mut jobs = self.jobs.lock().map_err(|_| ProbeError::Closed)?;
            if jobs.contains_key(&request) {
                return Err(ProbeError::Duplicate);
            }
            let task = runtime
                .as_ref()
                .ok_or(ProbeError::Closed)?
                .spawn(async move {
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    listener.completed(request)?;
                    Ok(request)
                });
            jobs.insert(request, task.abort_handle());
            task
        };
        let mut task = OwnedTask {
            task,
            jobs: self.jobs.clone(),
            request,
        };
        (&mut task.task).await.map_err(|error| {
            if error.is_cancelled() {
                ProbeError::Cancelled
            } else {
                ProbeError::Runtime {
                    detail: error.to_string(),
                }
            }
        })?
    }
    pub fn cancel(&self, request: u64) -> Result<bool, ProbeError> {
        let jobs = self.jobs.lock().map_err(|_| ProbeError::Closed)?;
        if let Some(task) = jobs.get(&request) {
            task.abort();
            Ok(true)
        } else {
            Ok(false)
        }
    }
    pub fn pending(&self) -> Result<u64, ProbeError> {
        Ok(self.jobs.lock().map_err(|_| ProbeError::Closed)?.len() as u64)
    }
    pub fn shutdown(&self) -> Result<(), ProbeError> {
        let runtime = self.runtime.lock().map_err(|_| ProbeError::Closed)?.take();
        if let Some(runtime) = runtime {
            runtime.shutdown_background();
        }
        Ok(())
    }
}
// Intentional test-only panic: verify generated scaffolding catches unwinding.
#[uniffi::export]
pub fn injected_panic() -> Result<(), ProbeError> {
    panic!("FFI smoke-test panic");
}
