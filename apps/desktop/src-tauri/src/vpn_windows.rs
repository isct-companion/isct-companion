//! The Windows campus VPN helper: a LocalSystem service on a named pipe,
//! installed once through the UAC prompt. The helper binary is its own
//! installer (`isct-vpn-helper.exe install`, crates/vpn-helper/src/windows.rs).
use crate::vpn::{converse, io_error};
use app_core::vpn::{HelperReport, VpnError};
use std::{
    fs::OpenOptions,
    io::ErrorKind,
    os::windows::ffi::OsStrExt,
    path::Path,
    sync::mpsc,
    thread,
    time::Duration,
};
use vpn_protocol::{Request, PIPE_PATH, SERVICE_NAME};
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, ERROR_PIPE_BUSY};
use windows::Win32::System::Services::{
    CloseServiceHandle, OpenSCManagerW, OpenServiceW, SC_MANAGER_CONNECT, SERVICE_QUERY_STATUS,
};
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

/// The helper binary in the app's `resources/vpn`.
pub(crate) const HELPER: &str = "isct-vpn-helper.exe";

pub(crate) struct HelperSocket;

impl HelperSocket {
    pub(crate) fn installed() -> Self {
        Self
    }

    /// A pipe has no read timeout, so the exchange runs on its own thread and
    /// a helper that does not answer in time is reported as such; the thread
    /// ends when the helper answers or goes away.
    pub(crate) fn exchange(
        &self,
        request: &Request,
        timeout: Duration,
    ) -> Result<HelperReport, VpnError> {
        let pipe = open_pipe()?;
        let request = request.clone();
        let (sender, answer) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(converse(&pipe, &request));
        });
        answer.recv_timeout(timeout).unwrap_or(Err(VpnError::Timeout))
    }
}

fn open_pipe() -> Result<std::fs::File, VpnError> {
    for _ in 0..20 {
        match OpenOptions::new().read(true).write(true).open(PIPE_PATH) {
            Ok(pipe) => return Ok(pipe),
            // Every instance is serving another request for a moment.
            Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY.0 as i32) => {
                thread::sleep(Duration::from_millis(100));
            }
            // No pipe: either the installer never ran or the service is not
            // running, which the service manager tells apart.
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(if service_registered() {
                    VpnError::HelperUnreachable("the helper is not running".into())
                } else {
                    VpnError::HelperNotInstalled
                });
            }
            Err(error) => return Err(io_error(error)),
        }
    }
    Err(VpnError::Timeout)
}

fn service_registered() -> bool {
    unsafe {
        let Ok(manager) = OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT)
        else {
            return false;
        };
        let service = OpenServiceW(manager, &HSTRING::from(SERVICE_NAME), SERVICE_QUERY_STATUS);
        if let Ok(service) = service {
            let _ = CloseServiceHandle(service);
        }
        let _ = CloseServiceHandle(manager);
        service.is_ok()
    }
}

/// Runs the bundled helper's installer through the UAC prompt and waits for
/// it. Returns false when the person declined the prompt.
pub(crate) fn run_install(resources: &Path) -> Result<bool, String> {
    let file: Vec<u16> = resources
        .join(HELPER)
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect();
    let parameters = HSTRING::from(format!(
        "install --server {}",
        app_core::vpn::f5::CAMPUS_APM_HOST
    ));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    if let Err(error) = unsafe { ShellExecuteExW(&mut info) } {
        if error.code() == ERROR_CANCELLED.to_hresult() {
            return Ok(false);
        }
        return Err(format!("the campus VPN helper could not be installed: {error}"));
    }
    let process = info.hProcess;
    if process.is_invalid() {
        return Err("the campus VPN helper installer did not start".into());
    }
    let mut code = 1u32;
    unsafe {
        WaitForSingleObject(process, INFINITE);
        let _ = GetExitCodeProcess(process, &mut code);
        let _ = CloseHandle(process);
    }
    if code == 0 {
        Ok(true)
    } else {
        Err("the campus VPN helper could not be installed".into())
    }
}
