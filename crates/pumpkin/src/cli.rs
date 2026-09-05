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
    /// Open the monitoring window alongside the server.
    ///
    /// Only meaningful in a build with the `gui` feature; otherwise the flag is rejected with a
    /// message saying so, rather than being silently ignored.
    pub gui: bool,
}

const HELP: &str = "\
Pumpkin - a Minecraft server in Rust.

Usage: pumpkin [OPTIONS]

Options:
      --gui      Open the monitoring and console window alongside the server
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

    if parsed.gui && !cfg!(feature = "gui") {
        return Action::Exit {
            message: GUI_NOT_COMPILED.to_owned(),
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
