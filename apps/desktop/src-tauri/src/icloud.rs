//! Small CloudKit private-database adapter used by the Apple-native backup
//! provider. The workspace is a CKAsset so the same format and size limit can
//! be shared with Drive and OneDrive without exposing it in iCloud Drive.

use block2::RcBlock;
use isct_core::cloud_sync::document::DocumentError;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::AnyThread;
use objc2_cloud_kit::{
    CKAccountStatus, CKAsset, CKContainer, CKDatabase, CKRecord, CKRecordID, CKRecordValue,
};
use objc2_foundation::{NSError, NSString, NSURL};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

const CONTAINER_ID: &str = "iCloud.app.isctcompanion";
const RECORD_TYPE: &str = "IsctCompanionBackup";
const ASSET_KEY: &str = "payload";

fn cloud_database() -> Retained<CKDatabase> {
    unsafe {
        CKContainer::containerWithIdentifier(&NSString::from_str(CONTAINER_ID))
            .privateCloudDatabase()
    }
}

fn record_id(name: &str) -> Retained<CKRecordID> {
    unsafe { CKRecordID::initWithRecordName(CKRecordID::alloc(), &NSString::from_str(name)) }
}

unsafe fn error_text(error: *mut NSError, fallback: &str) -> String {
    error
        .as_ref()
        .map(|error| error.localizedDescription().to_string())
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

unsafe fn is_missing(error: *mut NSError) -> bool {
    error.as_ref().is_some_and(|error| error.code() == 11)
}

#[cfg_attr(all(target_os = "macos", debug_assertions), allow(dead_code))]
fn account_request() -> oneshot::Receiver<Result<(), String>> {
    let container =
        unsafe { CKContainer::containerWithIdentifier(&NSString::from_str(CONTAINER_ID)) };
    let (tx, rx) = oneshot::channel();
    let sender = Arc::new(Mutex::new(Some(tx)));
    let callback = RcBlock::new(move |status: CKAccountStatus, error: *mut NSError| {
        let result = if status == CKAccountStatus::Available {
            Ok(())
        } else if !error.is_null() {
            Err(unsafe { error_text(error, "iCloud account is unavailable.") })
        } else {
            Err(match status {
                CKAccountStatus::NoAccount => "Sign in to iCloud to use iCloud sync.",
                CKAccountStatus::Restricted => "iCloud access is restricted on this device.",
                CKAccountStatus::TemporarilyUnavailable => "iCloud is temporarily unavailable.",
                _ => "iCloud account status could not be determined.",
            }
            .to_string())
        };
        if let Some(sender) = sender.lock().ok().and_then(|mut sender| sender.take()) {
            let _ = sender.send(result);
        }
    });
    unsafe { container.accountStatusWithCompletionHandler(&callback) };
    drop(callback);
    drop(container);
    rx
}

// Every public entry point checks the account first. macOS debug builds
// (`tauri dev`) are unsigned and lack the iCloud container entitlement, and
// CloudKit traps (SIGILL) instead of returning an error, so they never touch it.
pub async fn account_available() -> Result<(), String> {
    #[cfg(all(target_os = "macos", debug_assertions))]
    return Err("iCloud is unavailable in macOS debug builds.".into());
    #[cfg(not(all(target_os = "macos", debug_assertions)))]
    account_request()
        .await
        .map_err(|_| "iCloud account check did not complete.".to_string())?
}

fn read_request(name: &str) -> oneshot::Receiver<Result<Option<Vec<u8>>, String>> {
    let database = cloud_database();
    let id = record_id(name);
    let (tx, rx) = oneshot::channel();
    let sender = Arc::new(Mutex::new(Some(tx)));
    let callback = RcBlock::new(move |record: *mut CKRecord, error: *mut NSError| {
        let result = if record.is_null() {
            if unsafe { is_missing(error) } {
                Ok(None)
            } else {
                Err(unsafe { error_text(error, "iCloud backup read failed.") })
            }
        } else {
            let key = NSString::from_str(ASSET_KEY);
            let asset = unsafe { record.as_ref() }
                .and_then(|record| unsafe { record.objectForKey(&key) })
                .and_then(|value| value.downcast::<CKAsset>().ok());
            match asset.and_then(|asset| unsafe { asset.fileURL() }) {
                Some(url) => match url.path() {
                    Some(path) => std::fs::read(path.to_string())
                        .map(Some)
                        .map_err(|_| "iCloud returned an unreadable backup asset.".to_string()),
                    None => Err("iCloud returned an invalid backup asset.".to_string()),
                },
                None => Err("iCloud backup record has no payload.".to_string()),
            }
        };
        if let Some(sender) = sender.lock().ok().and_then(|mut sender| sender.take()) {
            let _ = sender.send(result);
        }
    });
    unsafe { database.fetchRecordWithID_completionHandler(&id, &callback) };
    drop(callback);
    drop(id);
    drop(database);
    rx
}

pub async fn read(name: &str) -> Result<Option<Vec<u8>>, String> {
    account_available().await?;
    read_request(name)
        .await
        .map_err(|_| "iCloud backup read did not complete.".to_string())?
}

fn write_request(
    name: String,
    temp_path: PathBuf,
    expected: Option<Option<u64>>,
) -> oneshot::Receiver<Result<(), DocumentError>> {
    let database = cloud_database();
    let id = record_id(&name);
    let (tx, rx) = oneshot::channel();
    let sender = Arc::new(Mutex::new(Some(tx)));
    let database_for_fetch = database.clone();
    let temp_for_fetch = temp_path.clone();
    let record_name = name;
    let callback = RcBlock::new(move |record: *mut CKRecord, error: *mut NSError| {
        let record = if record.is_null() {
            if unsafe { !is_missing(error) } {
                if let Some(sender) = sender.lock().ok().and_then(|mut sender| sender.take()) {
                    let _ = sender.send(Err(DocumentError::Transport(unsafe {
                        error_text(error, "iCloud backup lookup failed.")
                    })));
                }
                let _ = std::fs::remove_file(&temp_for_fetch);
                return;
            }
            unsafe {
                CKRecord::initWithRecordType_recordID(
                    CKRecord::alloc(),
                    &NSString::from_str(RECORD_TYPE),
                    &record_id(&record_name),
                )
            }
        } else {
            unsafe { Retained::retain(record).expect("CloudKit returned a live record") }
        };
        if let Some(expected) = expected {
            let current = unsafe { record.objectForKey(&NSString::from_str(ASSET_KEY)) }
                .and_then(|value| value.downcast::<CKAsset>().ok())
                .and_then(|asset| unsafe { asset.fileURL() })
                .and_then(|url| url.path())
                .and_then(|path| std::fs::read(path.to_string()).ok())
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .and_then(|value| value["revision"].as_u64());
            if current != expected {
                if let Some(sender) = sender.lock().ok().and_then(|mut s| s.take()) {
                    let _ = sender.send(Err(DocumentError::Conflict));
                }
                let _ = std::fs::remove_file(&temp_for_fetch);
                return;
            }
        }
        let url = NSURL::fileURLWithPath(&NSString::from_str(&temp_for_fetch.to_string_lossy()));
        let asset = unsafe { CKAsset::initWithFileURL(CKAsset::alloc(), &url) };
        let value: &ProtocolObject<dyn CKRecordValue> = ProtocolObject::from_ref(&*asset);
        unsafe { record.setObject_forKey(Some(value), &NSString::from_str(ASSET_KEY)) };

        let sender_for_save = sender.clone();
        let temp_for_save = temp_for_fetch.clone();
        let save_callback = RcBlock::new(move |_record: *mut CKRecord, error: *mut NSError| {
            let result = if error.is_null() {
                Ok(())
            } else if unsafe { error.as_ref().is_some_and(|e| e.code() == 14) } {
                // CKErrorServerRecordChanged: the conditional save lost a race.
                Err(DocumentError::Conflict)
            } else {
                Err(DocumentError::Transport(unsafe {
                    error_text(error, "iCloud backup write failed.")
                }))
            };
            let _ = std::fs::remove_file(&temp_for_save);
            if let Some(sender) = sender_for_save
                .lock()
                .ok()
                .and_then(|mut sender| sender.take())
            {
                let _ = sender.send(result);
            }
        });
        unsafe { database_for_fetch.saveRecord_completionHandler(&record, &save_callback) };
    });
    unsafe { database.fetchRecordWithID_completionHandler(&id, &callback) };
    drop(callback);
    drop(id);
    drop(database);
    rx
}

pub async fn write(name: &str, bytes: Vec<u8>) -> Result<(), String> {
    account_available().await?;
    let temp_path = temp_asset_path();
    std::fs::write(&temp_path, bytes)
        .map_err(|_| "iCloud backup asset could not be prepared.".to_string())?;
    write_request(name.to_string(), temp_path, None)
        .await
        .map_err(|_| "iCloud backup write did not complete.".to_string())?
        .map_err(|error| error.to_string())
}

fn temp_asset_path() -> PathBuf {
    std::env::temp_dir().join(format!("isct-companion-{}.json", uuid::Uuid::new_v4()))
}

pub async fn write_checked(
    name: &str,
    bytes: Vec<u8>,
    expected: Option<u64>,
) -> Result<(), DocumentError> {
    account_available()
        .await
        .map_err(DocumentError::Transport)?;
    let temp = temp_asset_path();
    std::fs::write(&temp, bytes).map_err(|e| DocumentError::Transport(e.to_string()))?;
    write_request(name.to_string(), temp, Some(expected))
        .await
        .map_err(|_| DocumentError::Transport("iCloud write did not complete".into()))?
}
