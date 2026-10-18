use crate::require_local_main;
use isct_core::notification::NotificationContent;
#[cfg(not(target_os = "macos"))]
use tauri::plugin::PermissionState;
#[cfg(not(target_os = "macos"))]
use tauri_plugin_notification::NotificationExt;

pub(crate) struct DesktopNotifications(pub(crate) tauri::AppHandle);

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

#[cfg(not(target_os = "macos"))]
async fn show(app: &tauri::AppHandle, content: &NotificationContent) -> Result<(), String> {
    app.notification()
        .builder()
        .title(content.title())
        .body(content.body())
        .show()
        .map_err(|_| "notification could not be shown".to_string())
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

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub(crate) fn request_notification_permission(
    webview: tauri::Webview,
    app: tauri::AppHandle,
) -> Result<&'static str, String> {
    require_local_main(&webview)?;
    match app
        .notification()
        .request_permission()
        .map_err(|_| "notification permission could not be requested".to_string())?
    {
        PermissionState::Granted => Ok("granted"),
        PermissionState::Denied => Ok("denied"),
        PermissionState::Prompt | PermissionState::PromptWithRationale => Ok("default"),
    }
}
