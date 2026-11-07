//! Explorer owns the launch so registration uses the interactive desktop's
//! user context, rather than inheriting an isolated/elevated launcher.
use isct_campus_menu_setup::{Outcome, Request, HELPER};
use std::{path::Path, time::Duration};
use tauri::Manager;
use windows::{
    core::{Interface, BSTR},
    Win32::{
        System::{
            Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, IDispatch, IServiceProvider,
                CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED,
            },
            Variant::VARIANT,
        },
        UI::Shell::{
            IShellBrowser, IShellDispatch2, IShellFolderViewDual, IShellWindows,
            SID_STopLevelBrowser, ShellWindows, SVGIO_BACKGROUND, SWC_DESKTOP, SWFO_NEEDDISPATCH,
        },
    },
};

pub(crate) async fn install(app: &tauri::AppHandle, label: String) -> Result<(), String> {
    let helper = app
        .path()
        .resource_dir()
        .map_err(|e| e.to_string())?
        .join("resources/campus-print")
        .join(HELPER);
    if !helper.is_file() {
        return Err("The campus menu installer is missing; reinstall the application.".into());
    }
    let request = Request {
        executable: std::env::current_exe().map_err(|e| e.to_string())?,
        label,
    };
    let directory = tempfile::Builder::new()
        .prefix("isct-campus-menu-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let file = directory.path().join("request.json");
    std::fs::write(
        &file,
        serde_json::to_vec(&request).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let launch_file = file.clone();
    // COM belongs to this blocking worker; release every interface before
    // uninitializing it. Never block the UI thread on Explorer's broker.
    tauri::async_runtime::spawn_blocking(move || launch(&helper, &launch_file))
        .await
        .map_err(|e| e.to_string())??;
    let result = file.with_extension("result");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        match std::fs::read(&result) {
            Ok(bytes) => {
                return match serde_json::from_slice::<Outcome>(&bytes).map_err(|e| e.to_string())? {
                    Outcome::Added => Ok(()),
                    Outcome::Failed(error) => Err(error),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        if tokio::time::Instant::now() >= deadline {
            // Explorer owns the short-lived installer after launch. It has
            // no background work: it writes/verifies once and exits. Removing
            // this private request directory discards any late acknowledgement;
            // retrying is safe because registration is idempotent.
            return Err(
                "The campus menu installer did not confirm registration within 30 seconds.".into(),
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn launch(helper: &Path, request: &Path) -> Result<(), String> {
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .map_err(|e| e.to_string())?;
    let result = launch_in_apartment(helper, request).map_err(|e| e.to_string());
    unsafe { CoUninitialize() };
    result
}

fn launch_in_apartment(helper: &Path, request: &Path) -> windows::core::Result<()> {
    // Microsoft's Execute-in-Explorer pattern deliberately obtains the
    // desktop's Shell.Application object instead of creating one locally.
    unsafe {
        let windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER)?;
        let empty = VARIANT::default();
        let mut hwnd = 0;
        let desktop =
            windows.FindWindowSW(&empty, &empty, SWC_DESKTOP, &mut hwnd, SWFO_NEEDDISPATCH)?;
        let provider: IServiceProvider = desktop.cast()?;
        let browser: IShellBrowser = provider.QueryService(&SID_STopLevelBrowser)?;
        // GetItemObject exposes IDispatch; request that documented interface
        // first, then query the dual interface on the returned automation object.
        let dispatch: IDispatch = browser
            .QueryActiveShellView()?
            .GetItemObject(SVGIO_BACKGROUND)?;
        let view: IShellFolderViewDual = dispatch.cast()?;
        let shell: IShellDispatch2 = view.Application()?.cast()?;
        shell.ShellExecute(
            &BSTR::from(helper.as_os_str().to_string_lossy().as_ref()),
            &VARIANT::from(format!("\"{}\"", request.display()).as_str()),
            &empty,
            &VARIANT::from("open"),
            &VARIANT::from(0i32),
        )
    }
}
