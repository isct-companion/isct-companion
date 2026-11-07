#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[cfg(target_os = "windows")]
fn main() {
    if run().is_err() {
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {}

#[cfg(target_os = "windows")]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    use isct_campus_menu_setup::{Outcome, Request};
    let request_file = std::env::args_os().nth(1).ok_or("missing request file")?;
    let request_file = std::path::PathBuf::from(request_file);
    let request: Request = serde_json::from_slice(&std::fs::read(&request_file)?)?;
    let outcome = match install(&request) {
        Ok(()) => Outcome::Added,
        Err(error) => Outcome::Failed(error.to_string()),
    };
    let pending = request_file.with_extension("pending");
    std::fs::write(&pending, serde_json::to_vec(&outcome)?)?;
    std::fs::rename(pending, request_file.with_extension("result"))?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn install(request: &isct_campus_menu_setup::Request) -> Result<(), Box<dyn std::error::Error>> {
    use isct_campus_menu_setup::{PRINT_FLAG, VERB_KEY};
    use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
    if !request.executable.is_absolute()
        || !request.executable.is_file()
        || request.executable.file_name().and_then(|s| s.to_str()) != Some("isct-companion.exe")
        || request.label.is_empty()
        || request.label.len() > 512
        || request.label.chars().any(char::is_control)
    {
        return Err("invalid campus menu installation request".into());
    }
    let exe = request
        .executable
        .to_str()
        .ok_or("non-Unicode executable path")?;
    let command = format!("\"{exe}\" {PRINT_FLAG} \"%1\"");
    let verb = windows_registry::CURRENT_USER.create(VERB_KEY)?;
    verb.set_string("", &request.label)?;
    verb.set_string("Icon", format!("\"{exe}\",0"))?;
    verb.create("command")?.set_string("", &command)?;
    // Read back from the Explorer-launched installer's registry environment,
    // never from the potentially isolated parent application's view.
    let installed = windows_registry::CURRENT_USER.open(VERB_KEY)?;
    if installed.get_string("")? != request.label
        || installed.open("command")?.get_string("")? != command
    {
        return Err("campus menu registration verification failed".into());
    }
    // SHCNE_ASSOCCHANGED requires null item pointers.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    Ok(())
}
