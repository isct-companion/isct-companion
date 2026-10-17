#![recursion_limit = "256"]

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

use cloud::DesktopCloudService;
use school_auth::SchoolAuthService;
use school_web::SchoolWebController;
use secrets::OsSecretVault;

#[cfg(desktop)]
mod agent_server;
#[cfg(target_os = "macos")]
mod campus_wifi;
mod cloud;
mod cloud_sync;
mod commands;
#[cfg(desktop)]
mod desktop_updates;
pub use isct_core::contracts;
pub use isct_core::data;
#[cfg(any(target_os = "macos", target_os = "ios"))]
mod icloud;
use isct_core::kyomu_guide;
use isct_core::lifecycle;
#[cfg(desktop)]
mod lms_downloader;
mod lms_files;
mod lms_protocol;
use isct_core::pull_coordinator;
#[cfg(matrix_ocr)]
use isct_ocr as matrix_ocr;
mod core_ports;
pub(crate) use isct_core::school_auth;
use isct_core::school_refresh;
mod school_browser_native;
mod school_downloads;
#[cfg(not(mobile))]
mod school_web;
#[cfg(mobile)]
#[path = "../../../../deprecated/tauri-mobile/rust/school_web_mobile.rs"]
mod school_web;
mod secrets;
pub(crate) use isct_core::syllabus;
#[cfg(desktop)]
mod workspaces;

/// The frontend's build-time identity: shell kind and operating system, both
/// fixed when this binary is compiled, so the UI never sniffs user agents.
fn native_bootstrap() -> String {
    #[cfg(desktop)]
    const TARGET: &str = "native-desktop";
    #[cfg(mobile)]
    const TARGET: &str = "native-mobile";
    format!(
        "Object.defineProperty(window,'__ISCT_COMPANION_NATIVE_TARGET__',{{value:'{TARGET}',writable:false,configurable:false}});\
         Object.defineProperty(window,'__ISCT_COMPANION_NATIVE_OS__',{{value:'{}',writable:false,configurable:false}});",
        std::env::consts::OS
    )
}

fn require_local_main(webview: &tauri::Webview) -> Result<(), String> {
    if webview.label() != "main" {
        return Err("native commands are restricted to the local main webview".into());
    }
    let url = webview
        .url()
        .map_err(|_| "native caller URL is unavailable")?;
    let packaged = (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
        || (url.scheme() == "http" && url.host_str() == Some("tauri.localhost"));
    #[cfg(debug_assertions)]
    let trusted = packaged
        || (url.scheme() == "http"
            && url.host_str() == Some("localhost")
            && url.port() == Some(4173));
    #[cfg(not(debug_assertions))]
    let trusted = packaged;
    if !trusted {
        return Err("native command caller is not the bundled application origin".into());
    }
    Ok(())
}

/// SQLite-backed commands run on a blocking thread so a slow query never stalls
/// WebView frames on the main thread. They stay serialized with each other, as
/// they were when Tauri ran them on the main thread.
async fn run_data_command<T, F>(work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    static DATA_COMMANDS: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let queued_at = std::time::Instant::now();
    tauri::async_runtime::spawn_blocking(move || {
        let _serialized = DATA_COMMANDS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let started = std::time::Instant::now();
        let result = work();
        if std::env::var_os("ISCT_TRACE_READS").is_some() {
            eprintln!(
                "[native-data] op={} queue_ms={} work_ms={} ok={}",
                std::any::type_name::<F>(),
                started.duration_since(queued_at).as_millis(),
                started.elapsed().as_millis(),
                result.is_ok()
            );
        }
        result
    })
    .await
    .map_err(|_| "The native data command stopped unexpectedly.".to_string())?
}

/// Keeps a plugin's Rust API but drops its JavaScript side. The notification
/// plugin's init script replaces `window.Notification` in every webview and
/// calls `is_permission_granted`, which no capability grants; the app
/// delivers notifications through its own `notify` command instead.
struct RustOnlyPlugin<R: tauri::Runtime>(tauri::plugin::TauriPlugin<R>);

impl<R: tauri::Runtime> tauri::plugin::Plugin<R> for RustOnlyPlugin<R> {
    fn name(&self) -> &'static str {
        self.0.name()
    }

    fn initialize(
        &mut self,
        app: &tauri::AppHandle<R>,
        config: serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.0.initialize(app, config)
    }

    fn on_event(&mut self, app: &tauri::AppHandle<R>, event: &tauri::RunEvent) {
        self.0.on_event(app, event)
    }
}

#[cfg(target_os = "macos")]
fn owned_macos_menu(app: &tauri::AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{Menu, MenuItem};
    let menu = Menu::default(app)?;
    let entries = menu.items()?;
    let app_menu = entries
        .first()
        .and_then(|item| item.as_submenu())
        .ok_or_else(|| std::io::Error::other("The application menu is unavailable."))?;
    let items = app_menu.items()?;
    let position = items
        .len()
        .checked_sub(1)
        .ok_or_else(|| std::io::Error::other("The application Quit item is unavailable."))?;
    let label = items[position]
        .as_predefined_menuitem()
        .ok_or_else(|| std::io::Error::other("The application Quit item is not predefined."))?
        .text()?;
    // Cocoa's predefined Quit calls terminate: directly, bypassing ExitRequested.
    // Keep the native menu, but route Quit / Cmd-Q through our owned shutdown.
    app_menu.remove_at(position)?;
    app_menu.append(&MenuItem::with_id(
        app,
        "owned-quit",
        label,
        true,
        Some("CmdOrCtrl+Q"),
    )?)?;
    Ok(menu)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .register_asynchronous_uri_scheme_protocol("lmsfile", |context, request, responder| {
            eprintln!(
                "[lms-file] protocol {} from {}",
                request.method(),
                context.webview_label()
            );
            if context.webview_label() != "main" {
                responder.respond(
                    tauri::http::Response::builder()
                        .status(tauri::http::StatusCode::BAD_REQUEST)
                        .header(tauri::http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                        .header("X-Isct-Reason", "invalid")
                        .body(br#"{"reason":"invalid"}"#.to_vec())
                        .unwrap_or_else(|_| tauri::http::Response::new(Vec::new())),
                );
                return;
            }
            let app = context.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                responder.respond(lms_protocol::handle(app, request).await);
            });
        })
        .plugin(RustOnlyPlugin(tauri_plugin_notification::init()))
        // The plugin's default link script runs in every webview and turns a
        // `target="_blank"` click into an `open_url` IPC call, which no
        // capability grants: links in school pages silently did nothing.
        // School views route new-window links natively (see school_web.rs);
        // the shell's links use the `open_external` command.
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_secure_storage::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(core_ports::DesktopRuntime::default())
        .manage(commands::matrix_camera::CameraScanner::default());
    #[cfg(target_os = "macos")]
    let builder = builder
        .manage(campus_wifi::CampusWifi::default())
        .menu(owned_macos_menu)
        .on_menu_event(|app, event| {
            if event.id() == "owned-quit" {
                app.exit(0);
            }
        });
    #[cfg(desktop)]
    let builder = builder
        .manage(lms_downloader::SessionView::default())
        .manage(desktop_updates::DesktopUpdates::default())
        .plugin(tauri_plugin_updater::Builder::new().build());
    #[cfg(target_os = "android")]
    let builder = builder.plugin(tauri_plugin_school_web::init());
    #[cfg(mobile)]
    let builder = builder.plugin(tauri_plugin_google_drive_auth::init());
    #[cfg(matrix_ocr)]
    let builder = builder.manage(matrix_ocr::MatrixOcrService::default());
    builder
        .invoke_handler(tauri::generate_handler![
            #[cfg(desktop)]
            commands::platform::check_app_update,
            #[cfg(desktop)]
            commands::platform::app_update_state,
            #[cfg(desktop)]
            commands::platform::install_app_update,
            commands::platform::runtime_capabilities,
            lms_files::course_storage_info,
            lms_files::choose_course_storage,
            lms_files::save_course_note,
            lms_files::read_course_note,
            lms_files::move_course_note,
            lms_files::list_course_notes,
            lms_files::open_course_material,
            lms_files::open_course_file,
            lms_files::clean_course_materials,
            #[cfg(desktop)]
            workspaces::assignment_workspace,
            #[cfg(desktop)]
            workspaces::create_assignment_workspace,
            #[cfg(desktop)]
            workspaces::open_workspace,
            #[cfg(desktop)]
            workspaces::set_workspace_submission,
            #[cfg(desktop)]
            workspaces::prepare_workspace_submission,
            #[cfg(desktop)]
            workspaces::delete_workspace,
            #[cfg(desktop)]
            workspaces::clean_workspaces,
            commands::platform::recognize_matrix_image,
            commands::matrix_camera::begin_matrix_camera,
            commands::matrix_camera::scan_matrix_camera_frame,
            commands::matrix_camera::stop_matrix_camera,
            commands::school::open_external,
            commands::school::resolve_school_url,
            commands::school::configure_school_web_policy,
            commands::school::open_school_site,
            commands::school::set_school_web_bounds,
            commands::school::hide_school_web,
            commands::school::reload_school_web,
            commands::school::navigate_school_web,
            commands::school::school_web_back,
            commands::school::school_web_forward,
            commands::school::reveal_school_download,
            commands::school::set_school_web_zoom,
            commands::school::pull_school,
            commands::school::pull_state,
            commands::school::cancel_pull,
            commands::school::refresh_reference_subscriptions,
            commands::school::set_kyomu_guide,
            commands::school_auth::school_credential_status,
            commands::school_auth::save_school_credentials,
            commands::school_auth::delete_school_credentials,
            commands::school_auth::copy_school_matrix,
            commands::school_auth::login_school,
            commands::school_auth::test_school_connection,
            commands::school_auth::school_session_snapshot,
            commands::school_auth::school_session_lifecycle,
            commands::notifications::request_notification_permission,
            commands::notifications::bootstrap_reminders,
            commands::read_models::lms_merge_preview,
            commands::notifications::notify,
            commands::school::school_refresh_event,
            commands::cloud::cloud_lifecycle,
            commands::cloud::cloud_sync_state,
            commands::cloud::set_cloud_sync_provider,
            commands::cloud::sync_cloud_now,
            commands::cloud::set_cloud_sync_automatic,
            commands::cloud::restore_cloud_version,
            commands::cloud::cloud_recovery_document,
            commands::cloud::connect_cloud,
            commands::cloud::cloud_authorization_status,
            commands::cloud::disconnect_cloud,
            commands::cloud::cloud_credential_sync_status,
            commands::cloud::cloud_credential_backup_status,
            commands::cloud::set_cloud_credential_sync_password,
            commands::cloud::sync_cloud_credentials,
            commands::cloud::clear_cloud_credential_sync_password,
            commands::workspace::load_workspace,
            commands::workspace::workspace_snapshot,
            commands::read_models::schedule_query,
            commands::read_models::finder_query,
            commands::workspace::mutate_workspace,
            commands::workspace::run_scheduled_task,
            commands::workspace::reset_workspace,
            commands::backup::export_data,
            commands::backup::save_file,
            commands::backup::import_data,
            commands::catalog::catalog_upsert_offerings,
            commands::catalog::catalog_offering,
            commands::catalog::catalog_offerings,
            commands::catalog::syllabus_course,
            commands::catalog::catalog_sync_department,
            commands::catalog::catalog_query,
            commands::catalog::catalog_sync_offerings,
            commands::agent::agent_connection,
            #[cfg(target_os = "macos")]
            campus_wifi::campus_wifi_connect,
            #[cfg(target_os = "macos")]
            campus_wifi::campus_wifi_status
        ])
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            let (data_store, _) = data::DataStore::initialize(app_data_dir.join("school.db"))
                .map_err(std::io::Error::other)?;
            app.manage(data_store);
            let vault = OsSecretVault::new(app.handle().clone());
            app.manage(
                DesktopCloudService::new(vault.clone())
                    .expect("failed to initialize the cloud transport"),
            );
            app.manage(std::sync::Arc::new(SchoolAuthService::with_status_cache(
                vault,
                app.path()
                    .app_data_dir()?
                    .join("school-credential-status.json"),
            )));
            app.manage(std::sync::Arc::new(isct_core::Core::with_events(
                app.state::<data::DataStore>().inner().clone(),
                app.state::<std::sync::Arc<SchoolAuthService>>()
                    .inner()
                    .clone(),
                if cfg!(mobile) {
                    pull_coordinator::PullNetwork::Cellular
                } else {
                    pull_coordinator::PullNetwork::Wifi
                },
                std::sync::Arc::new(core_ports::DesktopEvents(app.handle().clone())),
            )));
            let school_controller = tauri::async_runtime::block_on(async {
                let native = school_browser_native::NativeBrowser::new(app.handle().clone())?;
                let engine =
                    core_ports::core(app.handle()).attach_school_browser(native.clone())?;
                Ok::<_, isct_core::school_browser::SchoolBrowserError>(SchoolWebController::new(
                    native, engine,
                ))
            })?;
            app.manage(school_controller);
            let authorized = app
                .state::<DesktopCloudService<OsSecretVault>>()
                .authorized_providers()
                .unwrap_or_default();
            app.manage(
                cloud_sync::CloudSync::new(app_data_dir.join("cloud-sync.json"), &authorized)
                    .map_err(std::io::Error::other)?,
            );
            core_ports::runtime(app.handle())
                .attach_cloud(app.state::<cloud_sync::CloudSync>().0.clone())
                .map_err(std::io::Error::other)?;
            // The MCP listener exists only while Allow local agents is on;
            // the wake loop starts, stops and retries it.
            #[cfg(desktop)]
            app.manage(
                agent_server::AgentAccess::new(app.handle().clone())
                    .map_err(std::io::Error::other)?,
            );
            #[cfg(desktop)]
            {
                #[cfg(dev)]
                let main_url = WebviewUrl::External(
                    tauri::Url::parse("http://localhost:4173")
                        .expect("the local Vite URL is valid"),
                );
                #[cfg(not(dev))]
                let main_url = WebviewUrl::App("index.html".into());
                WebviewWindowBuilder::new(app, "main", main_url)
                    .title("IsctCompanion")
                    .inner_size(1280.0, 820.0)
                    .min_inner_size(860.0, 620.0)
                    .initialization_script(native_bootstrap())
                    .on_permission_request(|webview, kind| {
                        use tauri::webview::{PermissionKind, PermissionResponse};
                        match kind {
                            PermissionKind::Camera => {
                                if require_local_main(&webview).is_err() {
                                    return PermissionResponse::Deny;
                                }
                                // WebKitGTK has no default camera prompt. The
                                // local scanner requests video on a user click.
                                #[cfg(target_os = "linux")]
                                {
                                    PermissionResponse::Allow
                                }
                                #[cfg(not(target_os = "linux"))]
                                {
                                    PermissionResponse::Default
                                }
                            }
                            PermissionKind::Microphone => PermissionResponse::Deny,
                            _ => PermissionResponse::Default,
                        }
                    })
                    .build()?;
            }

            #[cfg(mobile)]
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .initialization_script(native_bootstrap())
                .build()?;
            // After the main window exists: the initial focus fact and event
            // delivery both refer to it.
            core_ports::DesktopRuntime::start(app.handle());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build IsctCompanion")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, code, .. } = &event {
                let runtime = app.state::<core_ports::DesktopRuntime>();
                if !runtime.stopped() {
                    api.prevent_exit();
                    if runtime.begin_exit() {
                        let app = app.clone();
                        let code = code.unwrap_or(0);
                        tauri::async_runtime::spawn(async move {
                            let drained = tokio::time::timeout(core_ports::EXIT_TIMEOUT, async {
                                cloud_sync::CloudSync::changed(&app).await;
                                #[cfg(desktop)]
                                if let Some(access) = app.try_state::<agent_server::AgentAccess>() {
                                    access.shutdown().await;
                                }
                                app.state::<core_ports::DesktopRuntime>()
                                    .shutdown(&core_ports::core(&app))
                                    .await;
                            })
                            .await;
                            if drained.is_err() {
                                eprintln!(
                                    "[shutdown] owned work did not finish within {:?}; exiting",
                                    core_ports::EXIT_TIMEOUT
                                );
                            }
                            let runtime = app.state::<core_ports::DesktopRuntime>();
                            runtime.mark_stopped();
                            #[cfg(desktop)]
                            desktop_updates::install_on_exit(&app);
                            app.exit(code);
                        });
                    }
                }
            }
            // OS suspension differs from losing keyboard focus. Only real
            // suspension pauses admission in every lane; focus remains a fact.
            #[cfg(mobile)]
            if let tauri::RunEvent::WindowEvent { event, .. } = &event {
                match event {
                    tauri::WindowEvent::Suspended => {
                        tauri::async_runtime::block_on(core_ports::runtime(app).suspend())
                    }
                    tauri::WindowEvent::Resumed => {
                        tauri::async_runtime::block_on(core_ports::runtime(app).resume())
                    }
                    _ => (),
                }
            }
            if matches!(&event, tauri::RunEvent::Resumed) {
                tauri::async_runtime::block_on(core_ports::runtime(app).resume());
            }

            let foreground = match &event {
                tauri::RunEvent::Resumed => Some(true),
                tauri::RunEvent::WindowEvent {
                    label,
                    event: tauri::WindowEvent::Focused(focused),
                    ..
                } if label == "main" => Some(*focused),
                #[cfg(mobile)]
                tauri::RunEvent::WindowEvent {
                    event: tauri::WindowEvent::Suspended,
                    ..
                } => Some(false),
                #[cfg(mobile)]
                tauri::RunEvent::WindowEvent {
                    event: tauri::WindowEvent::Resumed,
                    ..
                } => Some(true),
                _ => None,
            };
            if let Some(foreground) = foreground {
                if let Err(error) = core_ports::runtime(app)
                    .refresh_event(school_refresh::RefreshEvent::lifecycle(foreground))
                {
                    eprintln!("[school-refresh] {error}");
                }
                core_ports::publish_conditions(app);
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    cloud_sync::CloudSync::changed(&app).await;
                    let _ = app.state::<cloud_sync::CloudSync>().resume(&app).await;
                });
            }
            #[cfg(target_os = "ios")]
            if let tauri::RunEvent::Opened { urls } = event {
                for url in urls {
                    if let Err(error) =
                        tauri_plugin_google_drive_auth::handle_url(app, url.as_str())
                    {
                        eprintln!("Google OAuth callback could not be handled: {error}");
                    }
                }
            }
            #[cfg(not(target_os = "ios"))]
            let _ = (app, event);
        });
}
