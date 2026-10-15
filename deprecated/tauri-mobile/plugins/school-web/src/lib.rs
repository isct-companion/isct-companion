//! Internal Android adapter. Remote school pages never receive its plugin IPC.
#![cfg(target_os = "android")]
use serde::Serialize;
use tauri::{
    plugin::{Builder, PluginHandle, TauriPlugin},
    AppHandle, Manager, Runtime,
};
struct SchoolViews<R: Runtime>(PluginHandle<R>);
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("school-web")
        .setup(|app, api| {
            let handle =
                api.register_android_plugin("app.isctcompanion.schoolweb", "SchoolWebPlugin")?;
            app.manage(SchoolViews(handle));
            Ok(())
        })
        .build()
}
pub fn command<R: Runtime>(
    app: &AppHandle<R>,
    payload: impl Serialize,
) -> Result<serde_json::Value, String> {
    app.state::<SchoolViews<R>>()
        .0
        .run_mobile_plugin("operate", payload)
        .map_err(|error| error.to_string())
}
