use crate::{types::*, JsonValue, SdkError};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, uniffi::Record)]
pub struct ScheduleSnapshot {
    pub values: Vec<WorkspaceQueryResult>,
    /// The changes the values read; re-query when an event names one.
    pub reads: Vec<WorkspaceChange>,
    /// The commit sequence read before the values; drop the answer when a
    /// `workspace-changed` with a larger `seq` has been heard.
    pub seq: u64,
}
#[derive(Clone, Debug, uniffi::Enum)]
// UniFFI transports records by value and does not expose Box<T> in this ABI.
#[allow(clippy::large_enum_variant)]
pub enum OperationResult {
    Committed {
        commit: WorkspaceCommit,
    },
    Schedule {
        snapshot: ScheduleSnapshot,
    },
    Finder {
        page: FinderPage,
    },
    Credentials {
        status: SchoolCredentialStatus,
    },
    SchoolSession {
        snapshot: SchoolSessionSnapshot,
    },
    SchoolWeb {
        result: SchoolWebResult,
    },
    SchoolWebPage {
        page: SchoolWebPage,
    },
    SchoolConnect {
        result: SchoolConnectResult,
    },
    Sync {
        snapshot: SyncSnapshot,
    },
    Cloud {
        state: SyncState,
    },
    File {
        file: LocalFile,
    },
    Files {
        files: Vec<LocalFile>,
    },
    Waiting {
        assignments: Vec<crate::services::WaitingHandIn>,
    },
    /// `checked`: whether a check was due and ran; `release`: a newer build
    /// for this platform, if it ran and found one.
    AppUpdate {
        checked: bool,
        release: Option<crate::services::AppRelease>,
    },
    Material {
        material: Material,
    },
    SchoolFile {
        file: crate::services::SchoolFile,
    },
    Recognitions {
        cells: Vec<Recognition>,
    },
    Document {
        value: JsonValue,
    },
    Text {
        value: String,
    },
    Flag {
        value: bool,
    },
    Done,
}
#[derive(Clone, Debug, uniffi::Enum)]
// Preserve the typed terminal result rather than reducing it to an opaque ID.
#[allow(clippy::large_enum_variant)]
pub enum OperationStatus {
    Pending,
    Completed { result: OperationResult },
    Failed { error: SdkError },
}
#[derive(uniffi::Object)]
pub struct Operation {
    pub(crate) id: u64,
    pub(crate) receiver: watch::Receiver<Option<Result<OperationResult, SdkError>>>,
    pub(crate) cancellation: CancellationToken,
    detached: CancellationToken,
    pub(crate) cancellable: bool,
}
impl Operation {
    pub(crate) fn new(
        id: u64,
        cancellable: bool,
    ) -> (
        Self,
        watch::Sender<Option<Result<OperationResult, SdkError>>>,
    ) {
        let (tx, rx) = watch::channel(None);
        (
            Self {
                id,
                receiver: rx,
                cancellation: CancellationToken::new(),
                detached: CancellationToken::new(),
                cancellable,
            },
            tx,
        )
    }
}
#[uniffi::export]
impl Operation {
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn status(&self) -> Result<OperationStatus, SdkError> {
        Ok(match self.receiver.borrow().as_ref() {
            None => OperationStatus::Pending,
            Some(Ok(value)) => OperationStatus::Completed {
                result: value.clone(),
            },
            Some(Err(error)) => OperationStatus::Failed {
                error: error.clone(),
            },
        })
    }
    /// Releases this operation's waiters; the admitted domain operation continues.
    pub fn detach(&self) -> Result<(), SdkError> {
        self.detached.cancel();
        Ok(())
    }
    /// Cooperative cancellation is capability-specific; sync reads are owned by Core.
    /// Transactions never abort because a foreign future was dropped or detached.
    pub fn cancel(&self) -> Result<bool, SdkError> {
        if !self.cancellable || self.receiver.borrow().is_some() {
            return Ok(false);
        }
        self.cancellation.cancel();
        Ok(true)
    }
    pub async fn await_result(&self) -> Result<OperationResult, SdkError> {
        let mut rx = self.receiver.clone();
        loop {
            if let Some(result) = rx.borrow_and_update().clone() {
                return result;
            }
            tokio::select! {
                biased;
                result=rx.changed()=>{result.map_err(|_|SdkError::Internal{detail:"Operation owner stopped without a result".into()})?;}
                _=self.detached.cancelled()=>return Err(SdkError::Cancelled),
            }
        }
    }
}
