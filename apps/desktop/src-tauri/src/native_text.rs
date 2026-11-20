use crate::core_ports;
use serde_json::Value;
use tauri::AppHandle;

pub(crate) struct WindowText {
    pub title: &'static str,
    pub message: &'static str,
    pub quit: &'static str,
    pub keep: &'static str,
    #[cfg(target_os = "macos")]
    pub cancel: &'static str,
    #[cfg(target_os = "windows")]
    pub tray_show: &'static str,
    pub remember: &'static str,
}

/// The sign-in sheet a school page's HTTP authentication raises
/// (school_webkit.rs); WebView2 shows its own.
#[cfg(target_os = "macos")]
pub(crate) struct SignInText {
    /// `{{host}}` stands for the server's host.
    pub title: &'static str,
    pub retry: &'static str,
    pub user: &'static str,
    pub password: &'static str,
    pub sign_in: &'static str,
    pub cancel: &'static str,
}

pub(crate) struct NativeText {
    pub window: WindowText,
    #[cfg(target_os = "macos")]
    pub sign_in: SignInText,
    #[cfg(target_os = "windows")]
    campus_print: &'static [(&'static str, &'static str)],
}

include!(concat!(env!("OUT_DIR"), "/native_text.rs"));

/// The sign-in sheet's words; English when the app is gone.
#[cfg(target_os = "macos")]
pub(crate) fn sign_in(app: Option<&AppHandle>) -> &'static SignInText {
    &app.map_or(&EN, current).sign_in
}

pub(crate) fn current(app: &AppHandle) -> &'static NativeText {
    let locale = core_ports::core(app)
        .store
        .device_preference("/locale")
        .ok()
        .flatten();
    match locale.as_ref().and_then(Value::as_str) {
        Some("ja") => &JA,
        Some("zh") => &ZH,
        _ => &EN,
    }
}

#[cfg(target_os = "windows")]
impl NativeText {
    pub(crate) fn campus_print(&self, key: &'static str) -> &'static str {
        self.campus_print
            .iter()
            .chain(EN.campus_print)
            .find_map(|&(name, text)| (name == key).then_some(text))
            .unwrap_or(key)
    }
}
