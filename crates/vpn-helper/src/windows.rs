//! The Windows helper: a LocalSystem service the app installs once, behind a
//! named pipe the interactive user can reach. The same binary is the route
//! hook (`hook`, started by `assets/isct-vpn-script.js`) and its own
//! installer (`install`, run elevated by the app).
//!
//!   isct-vpn-helper.exe install --server <host>   copy the payload, register, start
//!   isct-vpn-helper.exe service [options]         run under the service manager
//!   isct-vpn-helper.exe hook                      openconnect's route hook
//!   isct-vpn-helper.exe interrupt <pid>           Ctrl+C on that process's console
//!   isct-vpn-helper.exe [options]                 serve in the foreground (development)

use super::{handle, log, new_state, parse_args, windows_net, Config, Inner, State};
use std::ffi::OsString;
use std::fs::{self, File};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{mpsc, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime};
use vpn_protocol::{PIPE_PATH, SERVICE_NAME};
use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{ERROR_PIPE_CONNECTED, HANDLE};
use windows::Win32::Globalization::{MultiByteToWideChar, CP_ACP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows::Win32::System::Console::{
    AttachConsole, FreeConsole, GenerateConsoleCtrlEvent, SetConsoleCtrlHandler, CTRL_C_EVENT,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

const HELPER_EXE: &str = "isct-vpn-helper.exe";
const SCRIPT: &str = "isct-vpn-script.js";
/// The Wintun adapter openconnect creates for a tunnel, named per tunnel. A
/// killed openconnect leaves its adapter listed for a while, and given that
/// name again openconnect tries to open the dead adapter instead of creating
/// one, and gives up. Without a name it may settle for someone's TAP adapter.
const TUN_PREFIX: &str = "isct-campus";
/// SYSTEM and administrators own the pipe; interactive users may read and
/// write it, like the staff group on the macOS socket. Nobody else connects.
const PIPE_SDDL: PCWSTR = w!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)");
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Where the installer copies the payload: under Program Files, writable by
/// administrators only, never the per-user app directory it came from.
fn install_dir() -> PathBuf {
    let base = std::env::var_os("ProgramFiles").unwrap_or_else(|| r"C:\Program Files".into());
    PathBuf::from(base).join(r"IsctCompanion\VPN")
}

/// The helper's log and the host routes it pinned.
pub(crate) fn data_dir() -> PathBuf {
    let base = std::env::var_os("ProgramData").unwrap_or_else(|| r"C:\ProgramData".into());
    PathBuf::from(base).join(r"IsctCompanion\VPN")
}

pub(crate) fn defaults() -> Config {
    let dir = install_dir();
    Config {
        socket: PIPE_PATH.into(),
        openconnect: dir.join(r"openconnect\openconnect.exe"),
        script: dir.join(SCRIPT),
        server: super::DEFAULT_SERVER.into(),
        libdirs: Vec::new(),
        log: data_dir().join("helper.log"),
    }
}

pub(crate) fn main() {
    let mut args = std::env::args().skip(1);
    let code = match args.next().as_deref() {
        Some("install") => install(args),
        Some("hook") => windows_net::hook(&data_dir()),
        Some("service") => service::run(),
        Some("interrupt") => interrupt(args.next().and_then(|pid| pid.parse().ok())),
        // `serve` returns only when it cannot listen.
        _ => {
            serve(parse_args(std::env::args().skip(1)), new_state());
            1
        }
    };
    std::process::exit(code);
}

/// The service's command line: every path points at the installed copies.
fn service_arguments(dir: &Path, data: &Path, server: &str) -> Vec<OsString> {
    vec![
        "service".into(),
        "--openconnect".into(),
        dir.join(r"openconnect\openconnect.exe").into(),
        "--script".into(),
        dir.join(SCRIPT).into(),
        "--log".into(),
        data.join("helper.log").into(),
        "--server".into(),
        server.into(),
    ]
}

mod service {
    use super::*;
    use windows_service::service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    };
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::{define_windows_service, service_dispatcher};

    define_windows_service!(ffi_service_main, service_main);

    pub(super) fn run() -> i32 {
        match service_dispatcher::start(SERVICE_NAME, ffi_service_main) {
            Ok(()) => 0,
            Err(_) => 1,
        }
    }

    fn service_main(_arguments: Vec<OsString>) {
        // The installed command line, not StartService's arguments.
        let cfg = parse_args(std::env::args().skip(2));
        let (stopped, stop) = mpsc::channel();
        let handler = move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                let _ = stopped.send(());
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        };
        let Ok(status) = service_control_handler::register(SERVICE_NAME, handler) else {
            return;
        };
        let report = |current_state, controls_accepted, wait_hint| {
            let _ = status.set_service_status(ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state,
                controls_accepted,
                exit_code: ServiceExitCode::Win32(0),
                checkpoint: 0,
                wait_hint,
                process_id: None,
            });
        };
        let state = new_state();
        {
            let (cfg, state) = (cfg.clone(), state.clone());
            thread::spawn(move || serve(cfg, state));
        }
        report(
            ServiceState::Running,
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            Duration::ZERO,
        );
        let _ = stop.recv();
        report(
            ServiceState::StopPending,
            ServiceControlAccept::empty(),
            Duration::from_secs(10),
        );
        let _ = crate::stop(&cfg, &state);
        report(
            ServiceState::Stopped,
            ServiceControlAccept::empty(),
            Duration::ZERO,
        );
    }
}

/// Answers the pipe until the process ends. Starting also clears what a
/// tunnel that died with the machine left behind: the resolver rules are in
/// the registry and outlive a reboot.
fn serve(cfg: Config, state: State) {
    let _ = fs::create_dir_all(data_dir());
    windows_net::clean_up(&data_dir(), None);
    crate::ondemand::watch_idle(cfg.clone(), state.clone());
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    if let Err(error) = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PIPE_SDDL,
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    } {
        log(&cfg, &format!("pipe security: {error}"));
        return;
    }
    // Kept for the life of the process; every pipe instance uses it.
    let security = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: BOOL(0),
    };
    log(&cfg, "helper started");
    let path: Vec<u16> = cfg.socket.as_os_str().encode_wide().chain([0]).collect();
    let mut first = true;
    loop {
        // The first instance must be ours: a pipe someone else created
        // earlier under this name would otherwise collect the cookies.
        let open_mode = if first {
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE
        } else {
            PIPE_ACCESS_DUPLEX
        };
        let pipe = unsafe {
            CreateNamedPipeW(
                PCWSTR(path.as_ptr()),
                open_mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                4096,
                4096,
                0,
                Some(&security),
            )
        };
        if pipe.is_invalid() {
            log(
                &cfg,
                &format!("create pipe: {}", windows::core::Error::from_thread()),
            );
            if first {
                return;
            }
            thread::sleep(Duration::from_secs(1));
            continue;
        }
        first = false;
        let connected = match unsafe { ConnectNamedPipe(pipe, None) } {
            Ok(()) => true,
            Err(error) => error.code() == ERROR_PIPE_CONNECTED.to_hresult(),
        };
        // The pipe handle is now owned by `File`, which closes it.
        let stream = unsafe { File::from_raw_handle(pipe.0) };
        if !connected {
            continue;
        }
        let (cfg, state) = (cfg.clone(), state.clone());
        thread::spawn(move || {
            handle(&stream, &stream, &cfg, &state);
            // The client reads to the end of the line; let it, then hang up.
            let _ = stream.sync_all();
            let _ = unsafe { DisconnectNamedPipe(HANDLE(stream.as_raw_handle())) };
        });
    }
}

/// openconnect's own lines are ASCII; Windows' error messages come in the
/// system code page.
pub(crate) fn decode_output(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_owned();
    }
    let flags = MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0);
    let length = unsafe { MultiByteToWideChar(CP_ACP, flags, bytes, None) };
    let mut wide = vec![0u16; length.max(0) as usize];
    unsafe { MultiByteToWideChar(CP_ACP, flags, bytes, Some(&mut wide)) };
    String::from_utf16_lossy(&wide)
}

pub(crate) fn prepare(cmd: &mut Command, _cfg: &Config) {
    // A console without a window: Ctrl+C on it is how openconnect is asked
    // to log out and run the hook's disconnect (`terminate`).
    let started = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    cmd.arg("--interface")
        .arg(format!("{TUN_PREFIX}-{started}"))
        .creation_flags(CREATE_NO_WINDOW);
}

/// A job that ends openconnect with the helper, so a crashed service never
/// leaves a tunnel nobody can stop.
fn job() -> Option<HANDLE> {
    static JOB: OnceLock<usize> = OnceLock::new();
    let raw = *JOB.get_or_init(|| unsafe {
        let Ok(job) = CreateJobObjectW(None, PCWSTR::null()) else {
            return 0;
        };
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let set = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if set.is_err() {
            return 0;
        }
        job.0 as usize
    });
    (raw != 0).then_some(HANDLE(raw as *mut core::ffi::c_void))
}

pub(crate) fn adopt(child: &Child, cfg: &Config) {
    let Some(job) = job() else {
        log(cfg, "no job object; openconnect may outlive the helper");
        return;
    };
    if let Err(error) = unsafe { AssignProcessToJobObject(job, HANDLE(child.as_raw_handle())) } {
        log(cfg, &format!("job object: {error}"));
    }
}

/// openconnect's hook output does not reach the helper on Windows, so the
/// route table is read instead: up once the campus route is on the tunnel.
pub(crate) fn confirm_route(state: &mut Inner) {
    if state.routed || state.address.is_none() {
        return;
    }
    if let Some(index) = state.tun_index {
        state.routed = windows_net::campus_route_on(index);
    }
}

/// Ctrl+C reaches every process on a console, and sharing openconnect's took
/// the service down with it, so a short-lived copy of the helper sends it.
pub(crate) fn terminate(child: &Child, cfg: &Config) {
    let sent = std::env::current_exe().and_then(|exe| {
        Command::new(exe)
            .arg("interrupt")
            .arg(child.id().to_string())
            .creation_flags(CREATE_NO_WINDOW)
            .status()
    });
    match sent {
        Ok(status) if status.success() => {}
        Ok(status) => log(cfg, &format!("interrupt: {status}")),
        Err(error) => log(cfg, &format!("interrupt: {error}")),
    }
}

/// `interrupt <pid>`: moves to that process's console and sends Ctrl+C, which
/// this process then ignores. Exit code 0 when it was sent.
fn interrupt(pid: Option<u32>) -> i32 {
    unsafe extern "system" fn handled(_event: u32) -> BOOL {
        BOOL(1)
    }
    let Some(pid) = pid else {
        return 1;
    };
    unsafe {
        let _ = FreeConsole();
        if AttachConsole(pid).is_err() {
            return 1;
        }
        // Registered once attached: a handler from before does not survive
        // the move and the event would end this process too.
        let _ = SetConsoleCtrlHandler(Some(handled), true);
        if GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0).is_err() {
            return 1;
        }
    }
    // The event arrives on its own thread; leave it time to be swallowed.
    thread::sleep(Duration::from_millis(200));
    0
}

/// openconnect removes its own routes and resolvers when it ends cleanly;
/// this covers one that was killed.
pub(crate) fn after_stop(_cfg: &Config) {
    windows_net::clean_up(&data_dir(), None);
}

/// `install --server <host>`, run elevated from the app's resources: the
/// payload is copied under Program Files and the service (re)registered and
/// started. Exit code 0 on success.
fn install(args: impl Iterator<Item = String>) -> i32 {
    let args: Vec<String> = args.collect();
    let server = args
        .windows(2)
        .find(|pair| pair[0] == "--server")
        .map(|pair| pair[1].clone())
        .unwrap_or_else(|| super::DEFAULT_SERVER.into());
    let _ = fs::create_dir_all(data_dir());
    match install_service(&server) {
        Ok(()) => 0,
        Err(error) => {
            log(&defaults(), &format!("install: {error}"));
            1
        }
    }
}

fn install_service(server: &str) -> Result<(), String> {
    use windows_service::service::{
        ServiceAccess, ServiceAction, ServiceActionType, ServiceErrorControl,
        ServiceFailureActions, ServiceFailureResetPeriod, ServiceInfo, ServiceStartType,
        ServiceState, ServiceType,
    };
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    let text = |error: windows_service::Error| error.to_string();

    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let source = exe.parent().ok_or("no payload directory")?.to_path_buf();
    let dir = install_dir();
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .map_err(text)?;
    let access = ServiceAccess::QUERY_STATUS
        | ServiceAccess::START
        | ServiceAccess::STOP
        | ServiceAccess::CHANGE_CONFIG;
    let existing = manager.open_service(SERVICE_NAME, access).ok();
    // A running helper holds its executable and openconnect open.
    if let Some(service) = &existing {
        if service.query_status().map_err(text)?.current_state != ServiceState::Stopped {
            let _ = service.stop();
            let deadline = Instant::now() + Duration::from_secs(20);
            while service.query_status().map_err(text)?.current_state != ServiceState::Stopped {
                if Instant::now() > deadline {
                    return Err("the running helper did not stop".into());
                }
                thread::sleep(Duration::from_millis(250));
            }
        }
    }
    copy_payload(&exe, &source, &dir)?;

    let info = ServiceInfo {
        name: SERVICE_NAME.into(),
        display_name: "IsctCompanion Campus VPN".into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: dir.join(HELPER_EXE),
        launch_arguments: service_arguments(&dir, &data_dir(), server),
        dependencies: Vec::new(),
        account_name: None, // LocalSystem
        account_password: None,
    };
    let service = match existing {
        Some(service) => {
            service.change_config(&info).map_err(text)?;
            service
        }
        None => manager.create_service(&info, access).map_err(text)?,
    };
    let _ = service.set_description(
        "Connects IsctCompanion's campus VPN. Only Science Tokyo's networks go through it.",
    );
    // A crashed helper comes back, as launchd's KeepAlive does on macOS.
    let _ = service.update_failure_actions(ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(86_400)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![
            ServiceAction {
                action_type: ServiceActionType::Restart,
                delay: Duration::from_secs(5),
            };
            3
        ]),
    });
    service.start::<&str>(&[]).map_err(text)?;
    Ok(())
}

/// The helper, the route hook shim and openconnect with its libraries.
fn copy_payload(exe: &Path, source: &Path, dir: &Path) -> Result<(), String> {
    let step = |what: &str, error: std::io::Error| format!("{what}: {error}");
    fs::create_dir_all(dir).map_err(|e| step("create install directory", e))?;
    fs::copy(exe, dir.join(HELPER_EXE)).map_err(|e| step("copy helper", e))?;
    fs::copy(source.join(SCRIPT), dir.join(SCRIPT)).map_err(|e| step("copy route hook", e))?;
    let openconnect = dir.join("openconnect");
    if openconnect.exists() {
        fs::remove_dir_all(&openconnect).map_err(|e| step("remove old openconnect", e))?;
    }
    copy_tree(&source.join("openconnect"), &openconnect).map_err(|e| step("copy openconnect", e))
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/windows.rs"]
mod tests;

/// The tunnel's Wintun adapter, as openconnect named its index.
pub(crate) fn tunnel_index(state: &Inner) -> Option<u32> {
    state.tun_index
}

/// The adapter openconnect would never pick: its own name, not `TUN_PREFIX`.
const STANDBY_NAME: &str = "isct-standby";

/// A Wintun adapter of the helper's own for standby, from the wintun.dll the
/// payload ships beside openconnect. The helper installs every route.
pub(crate) fn standby_device(cfg: &Config) -> Result<tun::AsyncDevice, String> {
    use crate::ondemand::{STANDBY_ADDRESS, STANDBY_MTU};
    let wintun = cfg.openconnect.with_file_name("wintun.dll");
    let mut config = tun::Configuration::default();
    config
        .tun_name(STANDBY_NAME)
        .address(STANDBY_ADDRESS)
        .netmask(std::net::Ipv4Addr::BROADCAST)
        .mtu(STANDBY_MTU)
        .up();
    config.platform_config(|platform| platform.wintun_file(wintun));
    tun::create_as_async(&config).map_err(|error| error.to_string())
}

pub(crate) fn standby_interface(
    device: &tun::AsyncDevice,
) -> Result<crate::ondemand::StandbyIf, String> {
    use tun::AbstractDevice;
    let name = device.tun_name().map_err(|error| error.to_string())?;
    let index = device.tun_index().map_err(|error| error.to_string())?;
    Ok(crate::ondemand::StandbyIf {
        name,
        index: index as u32,
    })
}

pub(crate) fn add_standby_route(
    net: std::net::Ipv4Addr,
    len: u8,
    interface: &crate::ondemand::StandbyIf,
) -> Result<(), String> {
    windows_net::add_route(net, len, interface.index, std::net::Ipv4Addr::UNSPECIFIED)
}

pub(crate) fn delete_standby_route(
    net: std::net::Ipv4Addr,
    len: u8,
    interface: &crate::ondemand::StandbyIf,
) -> Result<(), String> {
    windows_net::delete_route(net, len, interface.index)
}

pub(crate) use windows_net::{bind_to_interface, has_connections, traffic};
