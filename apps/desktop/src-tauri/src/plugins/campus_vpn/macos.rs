//! The macOS campus VPN helper: a launchd daemon inside the app bundle,
//! registered with SMAppService and allowed once by the person in System
//! Settings (Login Items, where Touch ID works). launchd runs the helper in
//! place inside the copy of the app that registered it, so an install always
//! registers afresh from this copy (`run_install`). An app update brings the
//! new helper with it; the running one hands over by itself
//! (crates/vpn-helper/src/unix.rs). The app reaches it on a root:staff Unix
//! socket.
use super::helper::{converse, io_error, Install};
use app_core::vpn::{HelperReport, VpnError};
use objc2_foundation::NSString;
use objc2_service_management::{SMAppService, SMAppServiceStatus};
use std::{io::ErrorKind, os::unix::net::UnixStream, path::Path, path::PathBuf, time::Duration};
use vpn_protocol::{Request, SOCKET_PATH};

/// The helper binary in the app's `resources/vpn`.
pub(crate) const HELPER: &str = "isct-vpn-helper";

/// The daemon's plist in `Contents/Library/LaunchDaemons`, from
/// crates/vpn-helper/assets; `tauri.mjs` puts it there with the payload.
const DAEMON_PLIST: &str = "app.isctcompanion.vpn.plist";

pub(crate) struct HelperSocket {
    path: PathBuf,
}

impl HelperSocket {
    pub(crate) fn installed() -> Self {
        Self {
            path: PathBuf::from(SOCKET_PATH),
        }
    }

    pub(crate) fn exchange(
        &self,
        request: &Request,
        timeout: Duration,
    ) -> Result<HelperReport, VpnError> {
        let stream = UnixStream::connect(&self.path).map_err(connect_error)?;
        stream
            .set_read_timeout(Some(timeout))
            .and_then(|()| stream.set_write_timeout(Some(timeout)))
            .map_err(io_error)?;
        converse(stream, request)
    }
}

/// The connection standby lives on. A helper writes on it every
/// `STANDBY_HEARTBEAT_SECS`; one silent far longer than that has hung.
pub(crate) fn standby_stream() -> Result<UnixStream, VpnError> {
    let stream = UnixStream::connect(SOCKET_PATH).map_err(connect_error)?;
    let silent = Duration::from_secs(vpn_protocol::STANDBY_HEARTBEAT_SECS * 5);
    stream.set_read_timeout(Some(silent)).map_err(io_error)?;
    Ok(stream)
}

fn daemon() -> objc2::rc::Retained<SMAppService> {
    unsafe { SMAppService::daemonServiceWithPlistName(&NSString::from_str(DAEMON_PLIST)) }
}

/// No socket, or a socket nobody listens on (a helper that died leaves its
/// file behind), means no helper is running. The daemon's registration says
/// why: never set up, waiting for the person's approval, or registered and
/// allowed yet not started by launchd. The last is a daemon bound to a copy
/// of the app that has been moved or deleted (`run_install`), which only a
/// new registration from this app repairs.
fn connect_error(error: std::io::Error) -> VpnError {
    match error.kind() {
        ErrorKind::NotFound | ErrorKind::ConnectionRefused => match unsafe { daemon().status() } {
            SMAppServiceStatus::RequiresApproval => VpnError::HelperNeedsApproval,
            SMAppServiceStatus::Enabled => VpnError::HelperNotStarting,
            _ => VpnError::HelperNotInstalled,
        },
        _ => io_error(error),
    }
}

/// Registers the daemon from this copy of the app. The first time macOS asks
/// the person to allow it; this opens Login Items for that. Once allowed it
/// stays allowed, through app updates and new registrations, until the app
/// is deleted. The payload path is fixed by the plist, so `_resources` is
/// only the shared signature.
///
/// A registration belongs to the copy of the app that made it: launchd
/// resolves `BundleProgram` inside that copy and pins the helper's code
/// requirement then. Registering an enabled daemon again changes neither.
/// Local builds and disk images share the release's bundle identifier, so a
/// daemon registered from `target/debug/bundle` or a mounted image keeps
/// running the helper from there; once that copy is deleted launchd cannot
/// find the program ("Invalid or missing Program", EX_CONFIG) and gives up,
/// while the status still reads enabled. A helper that no longer meets the
/// pinned requirement fails the same way ("needs LWCR update"). So every
/// install takes the old registration away, waits until the system has let
/// go of it, and registers afresh from here.
pub(crate) fn run_install(_resources: &Path) -> Result<Install, String> {
    let service = daemon();
    if unsafe { service.status() } != SMAppServiceStatus::NotRegistered {
        if let Err(error) = unsafe { service.unregisterAndReturnError() } {
            // An entry whose app copy is gone may refuse to go; registering
            // below still points it at this copy.
            log::warn!(
                "[vpn] the old helper registration did not go: {}",
                error.localizedDescription()
            );
        }
        // Unregistering finishes in the background. Registering before it
        // has can leave launchd holding the old copy's path.
        for _ in 0..REGISTER_ATTEMPTS {
            if unsafe { service.status() } == SMAppServiceStatus::NotRegistered {
                break;
            }
            std::thread::sleep(SETTLE);
        }
    }
    // A daemon that still needs approval in Login Items is registered all
    // the same, yet register fails with EPERM ("Operation not permitted");
    // an already registered one fails too. The status says where it stands.
    // Right after an unregister register can still fail with EPERM while the
    // status reads not registered; it settles within a few seconds.
    let mut registered = Ok(());
    for _ in 0..REGISTER_ATTEMPTS {
        registered = unsafe { service.registerAndReturnError() };
        match unsafe { service.status() } {
            SMAppServiceStatus::Enabled => return Ok(Install::Ready),
            SMAppServiceStatus::RequiresApproval => {
                unsafe { SMAppService::openSystemSettingsLoginItems() };
                return Ok(Install::NeedsApproval);
            }
            _ => std::thread::sleep(SETTLE),
        }
    }
    Err(match registered {
        Err(error) => format!(
            "the campus VPN helper could not be registered: {}",
            error.localizedDescription()
        ),
        Ok(()) => "the campus VPN helper could not be registered".into(),
    })
}

/// Status reads and register attempts while an unregister settles.
const REGISTER_ATTEMPTS: usize = 10;
const SETTLE: Duration = Duration::from_millis(500);
