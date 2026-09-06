//! Command-line arguments.

/// What the process was asked to do.
pub enum Action {
    /// Start the server.
    Run(Args),
    /// Print `message` and exit with `code`.
    Exit { message: String, code: i32 },
}

#[derive(Default)]
pub struct Args {
    /// Opens the local IPC listener and hands the TTY console off, for a `pumpkin-gui` process to
    /// spawn this or attach to it.
    ///
    /// Only meaningful in a build with the `gui` feature; otherwise the flag is rejected with a
    /// message saying so, rather than being silently ignored.
    pub gui: bool,
    /// Forces plain TTY-console behaviour, overriding both `--gui` and the non-tty auto-detect
    /// heuristic.
    pub nogui: bool,
}

const HELP: &str = "\
Pumpkin - a Minecraft server in Rust.

Usage: pumpkin [OPTIONS]

Options:
      --gui      Open the local IPC listener for a pumpkin-gui process, instead of the console
      --nogui    Force plain console behaviour, overriding --gui and the non-tty auto-detect
  -h, --help     Print this help
  -V, --version  Print version information

Configuration is read from pumpkin.toml in the working directory.
The RUST_LOG environment variable overrides the configured log level.";

const GUI_NOT_COMPILED: &str = "\
error: this build does not include the GUI.

Rebuild with the feature enabled:

    cargo build --release --features gui

It is off by default because it links against Qt6, which would otherwise be
required to start the server at all.";

const GUI_AND_NOGUI: &str = "\
error: --gui and --nogui are opposites and cannot be used together.";

/// Parses process arguments.
pub fn parse<I, S>(args: I) -> Action
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut parsed = Args::default();

    for arg in args {
        match arg.as_ref() {
            "--gui" => parsed.gui = true,
            "--nogui" => parsed.nogui = true,
            "-h" | "--help" => {
                return Action::Exit {
                    message: HELP.to_owned(),
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

    if (parsed.gui || parsed.nogui) && !cfg!(feature = "gui") {
        return Action::Exit {
            message: GUI_NOT_COMPILED.to_owned(),
            code: 2,
        };
    }

    if parsed.gui && parsed.nogui {
        return Action::Exit {
            message: GUI_AND_NOGUI.to_owned(),
            code: 2,
        };
    }

    Action::Run(parsed)
}

fn version_line() -> String {
    format!(
        "pumpkin {} (commit {}, {})",
        env!("CARGO_PKG_VERSION"),
        env!("GIT_HASH"),
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    )
}
