//! Bridges the running server to the optional Qt6 window.

mod log_layer;
mod sampler;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pumpkin_config::gui::GuiConfig;
use pumpkin_gui::{GuiCommands, GuiSide, LogLevel, ThemePreference};

use crate::command::{self, CommandSender};
use crate::plugin::server::server_command::ServerCommandEvent;
use crate::server::Server;

pub use log_layer::layer as log_layer;
// Re-exported so `main.rs` never has to name `pumpkin_gui` directly, keeping the optional
// dependency behind one module.
pub use pumpkin_gui::{GuiError, run};

static ATTACHED: AtomicBool = AtomicBool::new(false);

/// True once [`attach`] has run, so `PumpkinServer::start` can leave the TTY alone.
#[must_use]
pub fn is_attached() -> bool {
    ATTACHED.load(Ordering::Acquire)
}

/// Creates the handle the window and the server share.
#[must_use]
pub fn side(config: &GuiConfig) -> GuiSide {
    GuiSide::new(
        ThemePreference::from_config(&config.theme),
        config.log_buffer_lines,
    )
}

/// Connects a started server to the window: starts the samplers and enables the console.
pub fn attach(server: &Arc<Server>, side: &GuiSide, config: &GuiConfig) {
    ATTACHED.store(true, Ordering::Release);

    let commands: Arc<dyn GuiCommands> = Arc::new(ServerCommands {
        server: server.clone(),
    });
    let _ = side.commands.set(commands);

    // Console command replies go through `println!`, not `tracing`, so the log layer cannot see
    // them
    let logs = side.logs.clone();
    command::set_console_sink(Box::new(move |line| {
        logs.push(LogLevel::Info, "console".to_owned(), strip_ansi(line));
    }));

    sampler::spawn(server, side, config);
}

/// Removes ANSI colour codes.
///
/// Mirrors what the file logger does; the window colours lines itself from the level.
fn strip_ansi(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for escape in chars.by_ref() {
                if escape.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            result.push(c);
        }
    }

    result
}

struct ServerCommands {
    server: Arc<Server>,
}

impl GuiCommands for ServerCommands {
    fn submit(&self, line: String) {
        // Exactly the path `setup_stdin_console` takes, so plugins see the same event and the
        // command behaves as if it had been typed in the terminal.
        let server = self.server.clone();
        self.server.spawn_task(async move {
            let mut event = ServerCommandEvent::new(line.clone());
            server.plugin_manager.fire(&server, &mut event).await;

            if !event.cancelled {
                server
                    .command_dispatcher
                    .load()
                    .handle_command(&CommandSender::Console.into_source(&server), &line);
            }
        });
    }

    fn completions(&self, line: &str, cursor: usize) -> Vec<String> {
        crate::logging::console_completions(&self.server, line, cursor)
    }

    fn request_stop(&self) {
        self.submit("stop".to_owned());
    }
}
