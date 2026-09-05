use serde::{Deserialize, Serialize};

/// Settings for the optional Qt6 monitoring window.
///
/// Present regardless of whether the `gui` feature was compiled in, so a configuration file stays
/// valid across builds.
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
pub struct GuiConfig {
    /// Colour scheme to start in: `system`, `dark` or `light`.
    pub theme: String,
    /// How often the window samples server and system state, in milliseconds.
    pub refresh_ms: u64,
    /// How many log lines the window keeps in its scrollback.
    pub log_buffer_lines: usize,
    /// How often world folder sizes and free disk space are rescanned, in seconds.
    pub disk_scan_secs: u64,
}

impl Default for GuiConfig {
    fn default() -> Self {
        Self {
            theme: "system".to_owned(),
            refresh_ms: 500,
            log_buffer_lines: 5000,
            disk_scan_secs: 30,
        }
    }
}
