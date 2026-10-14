use crate::require_local_main;
use tauri::plugin::PermissionState;
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
            show(&self.0, &item.title, &item.body)
                .map_err(isct_core::schedule::NotificationSinkError::Delivery)
        })
    }
}

fn show(app: &tauri::AppHandle, title: &str, body: &str) -> Result<(), String> {
    if title.len() > 160 || body.len() > 2_000 {
        return Err("notification content is too large".into());
    }
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|_| "notification could not be shown".to_string())
}

#[tauri::command]
pub(crate) fn notify(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    title: String,
    body: String,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    show(&app, &title, &body)?;
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
