//! The desktop tools, each a Tauri plugin of the same name. `build.rs` names
//! the systems each is built for and its commands; a tool missing from this
//! build has no module here (`#[cfg(tool = "...")]`), and the page lists the
//! ones present (`__ISCT_COMPANION_TOOLS__`). Plugins are shell glue: Core
//! decides and keeps the state, a plugin carries it to its page and does the
//! system IO.
use tauri::{Builder, Wry};

#[cfg(tool = "campus-printer")]
pub(crate) mod campus_printer;
#[cfg(tool = "campus-vpn")]
mod campus_vpn;
#[cfg(tool = "campus-wifi")]
mod campus_wifi;

/// Adds every tool this build has.
pub(crate) fn register(builder: Builder<Wry>) -> Builder<Wry> {
    #[cfg(tool = "campus-printer")]
    let builder = builder.plugin(campus_printer::init());
    #[cfg(tool = "campus-vpn")]
    let builder = builder.plugin(campus_vpn::init());
    #[cfg(tool = "campus-wifi")]
    let builder = builder.plugin(campus_wifi::init());
    builder
}

/// The tools this build has (`build.rs`), as the JSON array the native
/// bootstrap injects.
pub(crate) fn tools_json() -> String {
    let tools = env!("ISCT_DESKTOP_TOOLS")
        .split(',')
        .filter(|tool| !tool.is_empty())
        .collect::<Vec<_>>();
    serde_json::to_string(&tools).unwrap_or_else(|_| "[]".into())
}

/// Lets the main window's bundled page call a tool plugin's `default`
/// commands. Each plugin grants this itself when it starts, so no capability
/// file names a tool or the systems it is built for.
#[cfg(any(tool = "campus-printer", tool = "campus-vpn", tool = "campus-wifi"))]
fn grant_main(app: &tauri::AppHandle, tool: &str) -> tauri::Result<()> {
    use tauri::Manager;
    app.add_capability(
        tauri::ipc::CapabilityBuilder::new(tool)
            .window("main")
            .permission(format!("{tool}:default")),
    )
}
