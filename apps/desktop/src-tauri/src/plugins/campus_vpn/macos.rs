//! The macOS campus VPN helper: a launchd daemon inside the app bundle,
//! registered with SMAppService and allowed once by the person in System
//! Settings (Login Items, where Touch ID works). An app update brings the new
//! helper with it; the running one hands over by itself
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

/// No socket means no helper is running; the daemon's registration says
/// whether it was never set up or waits for the person's approval.
fn connect_error(error: std::io::Error) -> VpnError {
    let waiting = || unsafe { daemon().status() } == SMAppServiceStatus::RequiresApproval;
    match error.kind() {
        ErrorKind::NotFound | ErrorKind::ConnectionRefused if waiting() => {
            VpnError::HelperNeedsApproval
        }
        ErrorKind::NotFound => VpnError::HelperNotInstalled,
        ErrorKind::ConnectionRefused => {
            VpnError::HelperUnreachable("the helper is not running".into())
        }
        _ => io_error(error),
    }
}

/// Registers the daemon. The first time macOS asks the person to allow it;
/// this opens Login Items for that. Once allowed it stays allowed, through
/// app updates, until the app is deleted. The payload path is fixed by the
/// plist, so `_resources` is only the shared signature.
pub(crate) fn run_install(_resources: &Path) -> Result<Install, String> {
    let service = daemon();
    // Registering an enabled daemon again changes nothing: launchd keeps the
    // code requirement it took from the helper the first time. A helper that
    // no longer meets it (an ad hoc one is pinned to its cdhash) never starts
    // ("needs LWCR update", EX_CONFIG) and the old one keeps answering, so an
    // update registers the daemon afresh from this bundle.
    if unsafe { service.status() } == SMAppServiceStatus::Enabled {
        let _ = unsafe { service.unregisterAndReturnError() };
    }
    // A daemon that still needs approval in Login Items is registered all
    // the same, yet register fails with EPERM ("Operation not permitted");
    // an already registered one fails too. The status says where it stands.
    let registered = unsafe { service.registerAndReturnError() };
    match unsafe { service.status() } {
        SMAppServiceStatus::Enabled => Ok(Install::Ready),
        SMAppServiceStatus::RequiresApproval => {
            unsafe { SMAppService::openSystemSettingsLoginItems() };
            Ok(Install::NeedsApproval)
        }
        _ => Err(match registered {
            Err(error) => format!(
                "the campus VPN helper could not be registered: {}",
                error.localizedDescription()
            ),
            Ok(()) => "the campus VPN helper could not be registered".into(),
        }),
    }
}
