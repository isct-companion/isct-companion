//! A phone supplies only its current selection; Core owns the draft request.
use crate::{Operation, OperationResult, SdkError, Session};
use std::sync::Arc;

#[derive(uniffi::Object)]
pub struct UploadReceipt(app_core::school_browser::DraftUpload);

impl std::fmt::Debug for UploadReceipt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UploadReceipt")
            .field("id", &self.0.id)
            .finish()
    }
}

#[uniffi::export]
impl UploadReceipt {
    pub async fn dispatched(&self) -> bool {
        self.0.dispatched().await
    }
}

/// A file the shell received (shared into the app, dropped on an assignment),
/// by its path and the name it is handed in under.
#[derive(Clone, Debug, uniffi::Record)]
pub struct SharedFile {
    pub path: String,
    pub name: String,
}

/// What the LMS page says while Core fills its draft area, in the app's language.
#[derive(Clone, Debug, uniffi::Record)]
pub struct UploadMessages {
    pub waiting: String,
    pub queued: String,
    pub failed: String,
}

#[uniffi::export]
impl Session {
    /// Queues the selected files for the current assignment's LMS draft page.
    pub fn prepare_hand_in(
        &self,
        assignment_id: String,
        files: Vec<SharedFile>,
        messages: UploadMessages,
    ) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            let paths = files
                .into_iter()
                .map(|file| (std::path::PathBuf::from(file.path), file.name))
                .collect();
            let upload = i
                .core
                .school_browser()?
                .prepare_assignment_submission(
                    &i.core.store,
                    &assignment_id,
                    paths,
                    app_core::school_browser::SubmissionMessages {
                        waiting: messages.waiting,
                        queued: messages.queued,
                        failed: messages.failed,
                    },
                )
                .map_err(|error| crate::error::coded(&error))?;
            Ok(OperationResult::DraftUpload {
                id: upload.id.clone(),
                url: upload.url.clone(),
                receipt: Arc::new(UploadReceipt(upload)),
            })
        })
    }

    pub fn cancel_hand_in(&self, id: String) -> Result<(), SdkError> {
        self.inner.core.school_browser()?.cancel_submission(&id);
        Ok(())
    }
}
