use crate::require_local_main;
use isct_core::notification::NotificationContent;
#[cfg(target_os = "windows")]
use tauri::Manager;

pub(crate) struct DesktopNotifications(pub(crate) tauri::AppHandle);

pub(crate) fn unavailable_reason() -> Option<&'static str> {
    #[cfg(target_os = "macos")]
    {
        (!crate::notifications_macos::available()).then_some(crate::notifications_macos::UNBUNDLED)
    }
    #[cfg(target_os = "windows")]
    {
        None
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Some("Native desktop notifications are available on macOS and Windows.")
    }
}

#[tauri::command]
pub(crate) async fn notification_permission(
    webview: tauri::Webview,
) -> Result<&'static str, String> {
    require_local_main(&webview)?;
    #[cfg(target_os = "macos")]
    {
        crate::notifications_macos::permission().await
    }
    #[cfg(target_os = "windows")]
    {
        Ok("granted")
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Err("Native desktop notifications are available on macOS and Windows.".into())
    }
}

#[tauri::command]
pub(crate) fn open_notification_settings(
    webview: tauri::Webview,
    app: tauri::AppHandle,
) -> Result<(), String> {
    require_local_main(&webview)?;
    use tauri_plugin_opener::OpenerExt;
    #[cfg(target_os = "macos")]
    let url = format!(
        "x-apple.systempreferences:com.apple.Notifications-Settings.extension?id={}",
        app.config().identifier
    );
    #[cfg(target_os = "windows")]
    let url = "ms-settings:notifications";
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    return app
        .opener()
        .open_url(url, None::<&str>)
        .map_err(|error| error.to_string());
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Err("Notification settings are unavailable on this platform.".into())
}

impl isct_core::schedule::NotificationSink for DesktopNotifications {
    fn deliver<'a>(
        &'a self,
        item: isct_core::schedule::NotificationItem,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<(), isct_core::schedule::NotificationSinkError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            let content = NotificationContent::new(item.title, item.body)
                .map_err(|error| error.to_string())
                .map_err(isct_core::schedule::NotificationSinkError::Delivery)?;
            show(&self.0, &content)
                .await
                .map_err(isct_core::schedule::NotificationSinkError::Delivery)
        })
    }
}

#[cfg(target_os = "macos")]
async fn show(_app: &tauri::AppHandle, content: &NotificationContent) -> Result<(), String> {
    crate::notifications_macos::show(content.title(), content.body()).await
}

#[cfg(target_os = "windows")]
async fn show(app: &tauri::AppHandle, content: &NotificationContent) -> Result<(), String> {
    let app = app.clone();
    let title = content.title().to_owned();
    let body = content.body().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let activate = app.clone();
        tauri_winrt_notification::Toast::new(&app.config().identifier)
            .title(&title)
            .text1(&body)
            .sound(Some(tauri_winrt_notification::Sound::Default))
            .on_activated(move |_| {
                if let Some(window) = activate.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
                Ok(())
            })
            .show()
            .map_err(|error| format!("notification could not be shown: {error}"))
    })
    .await
    .map_err(|error| format!("notification task failed: {error}"))?
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
async fn show(_app: &tauri::AppHandle, _content: &NotificationContent) -> Result<(), String> {
    Err("Native desktop notifications are available on macOS and Windows.".into())
}

#[tauri::command]
pub(crate) async fn notify(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    title: String,
    body: String,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    let content = NotificationContent::new(title, body).map_err(|error| error.to_string())?;
    show(&app, &content).await?;
    Ok(true)
}

#[tauri::command]
pub(crate) async fn bootstrap_reminders(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    fired: serde_json::Value,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    crate::core_ports::core(&app)
        .import_legacy_reminders(fired, chrono::Utc::now().fixed_offset())
        .await
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub(crate) async fn request_notification_permission(
    webview: tauri::Webview,
) -> Result<&'static str, String> {
    require_local_main(&webview)?;
    crate::notifications_macos::request_permission().await
}

#[cfg(target_os = "windows")]
#[tauri::command]
pub(crate) fn request_notification_permission(
    webview: tauri::Webview,
) -> Result<&'static str, String> {
    require_local_main(&webview)?;
    // Windows desktop has no per-call consent prompt. Native delivery below
    // returns the actual WinRT error instead of reporting a detached attempt as
    // success; global notification and Focus Assist policy remain user-owned.
    Ok("granted")
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
#[tauri::command]
pub(crate) fn request_notification_permission(
    webview: tauri::Webview,
) -> Result<&'static str, String> {
    require_local_main(&webview)?;
    Ok("denied")
}
