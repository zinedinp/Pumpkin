//! Locates and launches the `pumpkin-gui` executable, handing it the endpoint to connect to.

use pumpkin_gui_api::GUI_ENDPOINT_ENV;

#[cfg(windows)]
const BIN_NAME: &str = "pumpkin-gui.exe";
#[cfg(not(windows))]
const BIN_NAME: &str = "pumpkin-gui";

/// Looks first next to the current executable, then on `PATH`.
fn locate_gui_binary() -> Option<std::path::PathBuf> {
    if let Ok(current_exe) = std::env::current_exe()
        && let Some(dir) = current_exe.parent()
    {
        let candidate = dir.join(BIN_NAME);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(BIN_NAME))
            .find(|candidate| candidate.is_file())
    })
}

/// Spawns `pumpkin-gui`, passing `endpoint` via [`GUI_ENDPOINT_ENV`].
pub fn spawn(endpoint: &str) -> std::io::Result<std::process::Child> {
    let bin = locate_gui_binary().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("could not find `{BIN_NAME}` next to the current executable or on PATH"),
        )
    })?;

    std::process::Command::new(bin)
        .env(GUI_ENDPOINT_ENV, endpoint)
        .spawn()
}
