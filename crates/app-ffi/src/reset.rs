//! Settings → reset for the native apps. Core decides what each scope
//! removes; the shell lends the files directory. The web view's school
//! cookies are cleared through the shell's `NativeWebCookies`.
use crate::{SdkError, Session};
use app_core::{
    reset::ResetDevice,
    plan::{ResetReport, ResetScope},
};
use std::{path::PathBuf, sync::Arc};

/// `app_core::plan::ResetScope` across the ABI. (The FFI build does
/// not project `plan`, whose snapshot types carry timestamps.)
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum ResetRequest {
    /// Clear every school cache and read it again. Plans, notes, marks,
    /// overrides, settings and saved logins stay.
    SchoolCaches {
        sign_out_web: bool,
        delete_downloads: bool,
    },
    /// Return this device to its first-launch state. Cloud copies stay.
    Everything,
}

/// `app_core::plan::ResetReport` across the ABI.
#[derive(Clone, Debug, uniffi::Record)]
pub struct ResetSummary {
    pub removed_rows: u64,
    pub freed_bytes: u64,
}

struct MobileReset {
    files: PathBuf,
}

impl ResetDevice for MobileReset {
    fn course_files_root(&self) -> Option<PathBuf> {
        Some(self.files.clone())
    }
}

impl From<ResetRequest> for ResetScope {
    fn from(request: ResetRequest) -> Self {
        match request {
            ResetRequest::SchoolCaches {
                sign_out_web,
                delete_downloads,
            } => Self::SchoolCaches {
                sign_out_web,
                delete_downloads,
            },
            ResetRequest::Everything => Self::Everything,
        }
    }
}

impl From<ResetReport> for ResetSummary {
    fn from(report: ResetReport) -> Self {
        Self {
            removed_rows: report.removed_rows,
            freed_bytes: report.freed_bytes,
        }
    }
}

#[uniffi::export]
impl Session {
    /// Clears what `request` names and returns what went. A repeat while a
    /// reset of the same kind runs joins it. A full reset first stops cloud
    /// sync; no cloud copy changes.
    pub async fn reset(&self, request: ResetRequest) -> Result<ResetSummary, SdkError> {
        let inner = self.inner.clone();
        if inner.tasks.is_closed() {
            return Err(SdkError::Closed);
        }
        let scope = ResetScope::from(request);
        inner
            .handle
            .clone()
            .spawn(async move {
                if scope == ResetScope::Everything {
                    if let Ok((sync, services)) = inner.cloud() {
                        sync.prepare_reset(services.clone()).await?;
                    }
                }
                let device: Arc<dyn ResetDevice> = Arc::new(MobileReset {
                    files: inner.files.clone(),
                });
                Ok::<ResetSummary, SdkError>(
                    inner.core.reset_with(scope, Some(device)).await?.into(),
                )
            })
            .await
            .map_err(app_core::runtime_tasks::RuntimeTaskError::from)?
    }

    /// What `reset(request)` would remove now, for the confirmation screen.
    pub async fn reset_preview(&self, request: ResetRequest) -> Result<ResetSummary, SdkError> {
        let inner = self.inner.clone();
        if inner.tasks.is_closed() {
            return Err(SdkError::Closed);
        }
        let scope = ResetScope::from(request);
        inner
            .handle
            .clone()
            .spawn(async move {
                let device: Arc<dyn ResetDevice> = Arc::new(MobileReset {
                    files: inner.files.clone(),
                });
                Ok::<ResetSummary, SdkError>(
                    inner.core.reset_preview(scope, Some(device)).await?.into(),
                )
            })
            .await
            .map_err(app_core::runtime_tasks::RuntimeTaskError::from)?
    }
}
