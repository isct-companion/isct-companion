//! The macOS campus VPN helper: a launchd daemon on a root:staff Unix
//! socket, installed once with the system's administrator prompt.
use super::helper::{converse, io_error};
use app_core::vpn::{HelperReport, VpnError};
use std::{
    io::ErrorKind,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::Duration,
};
use vpn_protocol::{Request, SOCKET_PATH};

/// The helper binary in the app's `resources/vpn`.
pub(crate) const HELPER: &str = "isct-vpn-helper";

const LABEL: &str = "app.isctcompanion.vpnhelper";
const HELPER_DEST: &str = "/Library/PrivilegedHelperTools/app.isctcompanion.vpnhelper";
const SCRIPT_DEST: &str = "/Library/PrivilegedHelperTools/isct-vpn-script";
const SUPPORT_DIR: &str = "/Library/Application Support/IsctCompanion/vpn";
const PLIST_DEST: &str = "/Library/LaunchDaemons/app.isctcompanion.vpnhelper.plist";

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

/// No socket file means the installer never ran. A socket file with nobody
/// listening means the daemon is not running, which is not the same thing.
fn connect_error(error: std::io::Error) -> VpnError {
    match error.kind() {
        ErrorKind::NotFound => VpnError::HelperNotInstalled,
        ErrorKind::ConnectionRefused => {
            VpnError::HelperUnreachable("the helper is not running".into())
        }
        _ => io_error(error),
    }
}

/// The installer script, run once with the administrator prompt. It copies
/// the bundled helper, openconnect and route script into system locations,
/// and registers the daemon with launchd. Every path is fixed here or comes
/// from the app's own resource directory.
pub(crate) fn install_script(resources: &Path) -> String {
    let src = resources.to_string_lossy();
    let plist = launchd_plist();
    let steps = [
        format!("mkdir -p {}", sh_quote(SUPPORT_DIR)),
        format!(
            "rm -rf {0}/openconnect && cp -R {1}/openconnect {0}/openconnect",
            sh_quote(SUPPORT_DIR),
            sh_quote(&src)
        ),
        format!(
            "cp -f {} {}",
            sh_quote(&format!("{src}/isct-vpn-helper")),
            sh_quote(HELPER_DEST)
        ),
        format!(
            "cp -f {} {}",
            sh_quote(&format!("{src}/vpnc-script")),
            sh_quote(SCRIPT_DEST)
        ),
        format!(
            "chown root:wheel {} {}",
            sh_quote(HELPER_DEST),
            sh_quote(SCRIPT_DEST)
        ),
        format!(
            "chmod 755 {} {}",
            sh_quote(HELPER_DEST),
            sh_quote(SCRIPT_DEST)
        ),
        format!("chown -R root:wheel {}", sh_quote(SUPPORT_DIR)),
        format!("chmod -R go-w {}", sh_quote(SUPPORT_DIR)),
        format!(
            "printf '%s' {} > {}",
            sh_quote(&plist),
            sh_quote(PLIST_DEST)
        ),
        format!(
            "chown root:wheel {} && chmod 644 {}",
            sh_quote(PLIST_DEST),
            sh_quote(PLIST_DEST)
        ),
        format!("launchctl bootout system/{LABEL} >/dev/null 2>&1 || true"),
        format!("launchctl bootstrap system {}", sh_quote(PLIST_DEST)),
    ];
    steps.join(" && ")
}

/// The daemon's launch arguments, pointing at the copied openconnect and its
/// libraries rather than anything under Homebrew.
fn launchd_plist() -> String {
    let openconnect = format!("{SUPPORT_DIR}/openconnect/bin/openconnect");
    let lib = format!("{SUPPORT_DIR}/openconnect/lib");
    let args = [
        HELPER_DEST.to_owned(),
        "--openconnect".into(),
        openconnect,
        "--script".into(),
        SCRIPT_DEST.into(),
        "--libdir".into(),
        lib,
        "--server".into(),
        app_core::vpn::f5::CAMPUS_APM_HOST.into(),
    ];
    let items: String = args
        .iter()
        .map(|arg| format!("    <string>{}</string>\n", xml_escape(arg)))
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n\
         \x20 <key>Label</key><string>{LABEL}</string>\n\
         \x20 <key>ProgramArguments</key>\n  <array>\n{items}  </array>\n\
         \x20 <key>RunAtLoad</key><true/>\n\
         \x20 <key>KeepAlive</key><true/>\n\
         \x20 <key>StandardOutPath</key><string>/var/log/app.isctcompanion.vpnhelper.out.log</string>\n\
         \x20 <key>StandardErrorPath</key><string>/var/log/app.isctcompanion.vpnhelper.err.log</string>\n\
         </dict>\n</plist>\n"
    )
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Single-quotes a value for `/bin/sh`.
fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// Runs the install script with the system's administrator prompt. Returns
/// false when the person cancelled the prompt.
pub(crate) fn run_install(resources: &Path) -> Result<bool, String> {
    let script = install_script(resources);
    // AppleScript string literal: escape backslash and double quote.
    let quoted = script.replace('\\', r"\\").replace('"', r#"\""#);
    let apple = format!("do shell script \"{quoted}\" with administrator privileges");
    let output = std::process::Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(apple)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        return Ok(true);
    }
    // -128 is the person cancelling the administrator prompt.
    if String::from_utf8_lossy(&output.stderr).contains("(-128)") {
        return Ok(false);
    }
    Err("the campus VPN helper could not be installed".into())
}

#[cfg(test)]
#[path = "../../tests/vpn_macos.rs"]
mod tests;
