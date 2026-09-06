//! Command-line arguments.

use std::io::IsTerminal;

/// What the process was asked to do.
pub enum Action {
    /// Start the server.
    Run(Args),
    /// Print `message` and exit with `code`.
    Exit { message: String, code: i32 },
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct Args {
    /// Open the local IPC listener for a `pumpkin-gui` process and hand the TTY console off,
    /// instead of the plain console. Always `false` without the `gui` feature.
    #[cfg_attr(not(feature = "gui"), allow(dead_code))]
    pub ipc: bool,
}

/// The ambient inputs the IPC/console decision depends on, injected rather than read inside
/// [`parse`] so the decision table can be unit-tested without touching the process
/// environment (`set_var` is `unsafe`.
#[derive(Debug, Clone, Copy)]
pub struct Environment {
    /// `PUMPKIN_GUI_ENDPOINT` is set: a `pumpkin-gui` process spawned us.
    pub gui_endpoint_set: bool,
    /// Stdin is a terminal, so a console is worth offering.
    #[cfg_attr(not(feature = "gui"), allow(dead_code))]
    pub stdin_is_terminal: bool,
}

impl Environment {
    /// Reads the real process environment.
    #[must_use]
    pub fn detect() -> Self {
        Self {
            gui_endpoint_set: std::env::var_os(pumpkin_gui_api::GUI_ENDPOINT_ENV).is_some(),
            stdin_is_terminal: std::io::stdin().is_terminal(),
        }
    }
}

const HELP_HEAD: &str = "\
Pumpkin - a Minecraft server in Rust.

Usage: pumpkin [OPTIONS]

Options:
";
#[cfg(feature = "gui")]
const HELP_NOGUI: &str =
    "      --nogui    Force plain console behaviour, overriding the non-tty auto-detect\n";
const HELP_TAIL: &str = "  -h, --help     Print this help
  -V, --version  Print version information

Configuration is read from pumpkin.toml in the working directory.
The RUST_LOG environment variable overrides the configured log level.";
#[cfg(feature = "gui")]
const HELP_GUI_TAIL: &str = "\n\nSetting PUMPKIN_GUI_ENDPOINT to a socket path (or, on Windows, \
a pipe name) opens the GUI IPC listener there instead of the console, for \
`pumpkin-gui --attach`.";

const GUI_NOT_COMPILED: &str = "\
error: this build does not include the GUI.

Rebuild with the feature enabled:

    cargo build --release --features gui

The feature is off by default because it adds the local IPC listener, the log
ring and the system samplers the pumpkin-gui window reads from.";

/// Parses process arguments.
pub fn parse<I, S>(args: I, env: Environment) -> Action
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut nogui = false;

    for arg in args {
        match arg.as_ref() {
            // Kept in the match arm on every build so a non-gui binary gives the diagnosable
            // "no GUI in this build" message, rather than a generic "unrecognized argument".
            "--nogui" => {
                if cfg!(feature = "gui") {
                    nogui = true;
                } else {
                    return Action::Exit {
                        message: GUI_NOT_COMPILED.to_owned(),
                        code: 2,
                    };
                }
            }
            "-h" | "--help" => {
                return Action::Exit {
                    message: help(),
                    code: 0,
                };
            }
            "-V" | "--version" => {
                return Action::Exit {
                    message: version_line(),
                    code: 0,
                };
            }
            other => {
                return Action::Exit {
                    message: format!(
                        "error: unrecognized argument '{other}'\n\nTry 'pumpkin --help'."
                    ),
                    code: 2,
                };
            }
        }
    }

    // A `pumpkin-gui` spawned but this build cannot answer it.
    #[cfg(not(feature = "gui"))]
    if env.gui_endpoint_set {
        return Action::Exit {
            message: gui_env_not_compiled(),
            code: 2,
        };
    }

    #[cfg(not(feature = "gui"))]
    let ipc = {
        let _ = env;
        false
    };

    #[cfg(feature = "gui")]
    let ipc = !nogui && (env.gui_endpoint_set || !env.stdin_is_terminal);

    let _ = nogui;
    Action::Run(Args { ipc })
}

#[cfg(not(feature = "gui"))]
fn gui_env_not_compiled() -> String {
    format!(
        "{GUI_NOT_COMPILED}\n\nThis was triggered by {} being set in the environment;\n\
         unset it to run this build as a plain server.",
        pumpkin_gui_api::GUI_ENDPOINT_ENV,
    )
}

fn help() -> String {
    #[cfg(feature = "gui")]
    {
        format!("{HELP_HEAD}{HELP_NOGUI}{HELP_TAIL}{HELP_GUI_TAIL}")
    }
    #[cfg(not(feature = "gui"))]
    {
        format!("{HELP_HEAD}{HELP_TAIL}")
    }
}

fn version_line() -> String {
    let gui_marker = if cfg!(feature = "gui") {
        format!(", {}", pumpkin_gui_api::GUI_VERSION_MARKER)
    } else {
        String::new()
    };
    format!(
        "pumpkin {} (commit {}, {}{gui_marker})",
        env!("CARGO_PKG_VERSION"),
        env!("GIT_HASH"),
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    )
}
