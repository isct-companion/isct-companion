//! Native Google Drive authentication for iOS.
use serde::{Deserialize, Serialize};
use tauri::{
    plugin::{Builder, PluginHandle, TauriPlugin},
    AppHandle, Manager, Runtime,
};

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_google_drive_auth);

struct GoogleDriveAuth<R: Runtime>(PluginHandle<R>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SignInRequest<'a> {
    client_id: &'a str,
}

#[derive(Serialize)]
struct HandleUrlRequest<'a> {
    url: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeGoogleToken {
    pub client_id: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: u64,
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("google-drive-auth")
        .setup(|app, api| {
            let handle = api.register_ios_plugin(init_plugin_google_drive_auth)?;
            app.manage(GoogleDriveAuth(handle));
            Ok(())
        })
        .build()
}

pub fn sign_in<R: Runtime>(
    app: &AppHandle<R>,
    client_id: &str,
) -> Result<NativeGoogleToken, String> {
    app.state::<GoogleDriveAuth<R>>()
        .0
        .run_mobile_plugin("signIn", SignInRequest { client_id })
        .map_err(|error| error.to_string())
}

pub fn sign_out<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    app.state::<GoogleDriveAuth<R>>()
        .0
        .run_mobile_plugin("signOut", ())
        .map_err(|error| error.to_string())
}

pub fn handle_url<R: Runtime>(app: &AppHandle<R>, url: &str) -> Result<bool, String> {
    app.state::<GoogleDriveAuth<R>>()
        .0
        .run_mobile_plugin("handleUrl", HandleUrlRequest { url })
        .map_err(|error| error.to_string())
}
