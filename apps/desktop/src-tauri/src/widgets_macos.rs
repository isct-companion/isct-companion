use app_core::{
    schedule::widgets::{WidgetDestination, WidgetKind},
    widgets::{WidgetError, WidgetPort, WidgetRoute},
};
use std::{
    ffi::{c_char, c_void, CStr},
    fs::{self, File},
    io::{self, Write},
    os::unix::{ffi::OsStrExt, fs::OpenOptionsExt},
    path::PathBuf,
    sync::{Mutex, MutexGuard, PoisonError},
};
use tauri::{AppHandle, Emitter, Manager, Url};

unsafe extern "C" {
    fn isct_widget_container(
        receive: extern "C" fn(*const c_char, *mut c_void),
        context: *mut c_void,
    ) -> bool;
    fn isct_widget_reload(courses: bool, assignments: bool);
}

pub(crate) struct MacWidgets(PathBuf);

impl MacWidgets {
    pub(crate) fn open() -> Result<Self, WidgetError> {
        extern "C" fn receive(path: *const c_char, context: *mut c_void) {
            // Swift calls synchronously with a borrowed non-null C string and
            // the pointer to this invocation's PathBuf.
            unsafe {
                let path = std::ffi::OsStr::from_bytes(CStr::from_ptr(path).to_bytes());
                (*context.cast::<PathBuf>()).push(path);
            }
        }
        let mut path = PathBuf::new();
        if !unsafe { isct_widget_container(receive, (&mut path as *mut PathBuf).cast()) } {
            return Err(WidgetError::Storage(
                "The app group container is unavailable".into(),
            ));
        }
        Ok(Self(path.join("Widgets")))
    }
}

fn missing_is_ok<T>(result: io::Result<T>) -> Result<Option<T>, WidgetError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(storage(error)),
    }
}

fn storage(error: io::Error) -> WidgetError {
    WidgetError::Storage(error.to_string())
}

impl WidgetPort for MacWidgets {
    fn current(&self) -> Result<Option<String>, WidgetError> {
        missing_is_ok(fs::read_to_string(self.0.join("current.json")))
    }

    fn replace(&self, json: &str) -> Result<(), WidgetError> {
        fs::create_dir_all(&self.0).map_err(storage)?;
        let temporary = self.0.join("staged.json");
        let mut file = File::options()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(storage)?;
        file.write_all(json.as_bytes()).map_err(storage)?;
        file.sync_all().map_err(storage)?;
        fs::rename(temporary, self.0.join("current.json")).map_err(storage)
    }

    fn clear(&self) -> Result<(), WidgetError> {
        missing_is_ok(fs::remove_file(self.0.join("current.json")))?;
        missing_is_ok(fs::remove_file(self.0.join("staged.json")))?;
        Ok(())
    }

    fn reload(&self, kinds: &[WidgetKind]) -> Result<(), WidgetError> {
        unsafe {
            isct_widget_reload(
                kinds.contains(&WidgetKind::Courses),
                kinds.contains(&WidgetKind::Assignments),
            );
        }
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct WidgetLinks(Mutex<LinkState>);

#[derive(Default)]
struct LinkState {
    ready: bool,
    pending: Option<WidgetDestination>,
}

impl WidgetLinks {
    fn state(&self) -> MutexGuard<'_, LinkState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(crate) fn opened(app: &AppHandle, url: &Url) {
    let Some(destination) = WidgetDestination::from_url(url) else {
        return;
    };
    crate::window_close::show_main(app);
    let links = app.state::<WidgetLinks>();
    {
        let mut state = links.state();
        if !state.ready {
            state.pending = Some(destination);
            return;
        }
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Ok(route) = resolve(&app, destination).await {
            let _ = app.emit_to("main", "widget-route", route);
        }
    });
}

async fn resolve(
    app: &AppHandle,
    destination: WidgetDestination,
) -> Result<WidgetRoute, crate::CommandError> {
    let core = crate::core_ports::core(app);
    crate::run_read_command(move || core.widget_route(destination).map_err(crate::coded)).await
}

#[tauri::command]
pub(crate) async fn widget_frontend_ready(
    webview: tauri::Webview,
    app: AppHandle,
) -> Result<Option<WidgetRoute>, crate::CommandError> {
    crate::require_local_main(&webview)?;
    let pending = {
        let links = app.state::<WidgetLinks>();
        let mut state = links.state();
        state.ready = true;
        state.pending.take()
    };
    match pending {
        Some(destination) => resolve(&app, destination).await.map(Some),
        None => Ok(None),
    }
}
