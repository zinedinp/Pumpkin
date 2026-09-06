//! Bridges the running server to the optional GUI process over a local IPC socket.

mod ipc;
mod log_layer;
mod sampler;
mod spawn_process;

use std::sync::Arc;

use pumpkin_config::gui::GuiConfig;
use pumpkin_gui_api::{LogLevel, LogRing};

pub use ipc::{is_attached, notify_shutdown};
pub use log_layer::layer as log_layer;

use crate::command;
use crate::server::Server;

/// Builds the log ring the sampler and the IPC layer share; created before the server exists so
/// early boot log lines are not lost.
#[must_use]
pub fn new_log_ring(config: &GuiConfig) -> Arc<LogRing> {
    Arc::new(LogRing::new(config.log_buffer_lines))
}

/// Starts the local IPC listener and the samplers. Returns the endpoint a `pumpkin-gui` process
/// should connect to.
pub fn attach(
    server: &Arc<Server>,
    ring: &Arc<LogRing>,
    config: &GuiConfig,
) -> std::io::Result<String> {
    let (endpoint, broadcaster) = ipc::spawn_listener(server.clone(), config)?;

    // Console command replies go through `println!`, not `tracing`, so the log layer cannot see
    // them. `text.to_pretty_console()` (e.g. `/tps`, join/leave messages) carries real ANSI
    // colours and OSC 8 hyperlinks, same as it does in the terminal.
    let logs = ring.clone();
    command::set_console_sink(Box::new(move |line| {
        logs.push(LogLevel::Info, "console".to_owned(), line);
    }));

    sampler::spawn(server, ring, &broadcaster, config);
    Ok(endpoint)
}

/// Launches `pumpkin-gui` connected to `endpoint`.
///
/// Failure is not fatal to the server: the listener and samplers above keep running headless, and
/// a manually-started `pumpkin-gui --connect <endpoint>` can attach later.
pub fn spawn_gui_process(endpoint: &str) -> std::io::Result<std::process::Child> {
    spawn_process::spawn(endpoint)
}
