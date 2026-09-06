//! The data contract between the server and the GUI.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

use arc_swap::ArcSwap;

/// Which theme the window starts in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemePreference {
    #[default]
    System,
    Dark,
    Light,
}

impl ThemePreference {
    #[must_use]
    pub fn from_config(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "dark" => Self::Dark,
            "light" => Self::Light,
            _ => Self::System,
        }
    }
}

/// Severity of a captured log line, mirroring `tracing::Level` without depending on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    /// Lowercase name, used as the QML filter key.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// One line in the console view.
#[derive(Clone, Debug)]
pub struct LogLine {
    /// Monotonically increasing; the GUI remembers the last one it rendered.
    pub seq: u64,
    pub level: LogLevel,
    pub target: String,
    /// Human-readable text with ANSI/OSC 8 escapes stripped, for search, copy and save.
    pub message: String,
    /// The same line as an HTML fragment: real colours/attributes from the original ANSI escapes,
    /// hyperlinks (explicit OSC 8 click events and bare URLs) as `<a href>`. Computed once here
    /// rather than per draw, since each line is only ever rendered a handful of times.
    pub html: String,
}

struct LogRingInner {
    lines: VecDeque<LogLine>,
    next_seq: u64,
}

/// A bounded ring buffer of log lines that the GUI drains by sequence number.
pub struct LogRing {
    inner: Mutex<LogRingInner>,
    capacity: usize,
}

impl LogRing {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            inner: Mutex::new(LogRingInner {
                lines: VecDeque::with_capacity(capacity),
                next_seq: 0,
            }),
            capacity,
        }
    }

    /// `message` may carry raw ANSI SGR colour codes and OSC 8 hyperlinks exactly as printed to
    /// the terminal; they are parsed once here into a plain string and an HTML fragment rather
    /// than on every draw.
    pub fn push(&self, level: LogLevel, target: String, message: &str) {
        let rendered = crate::ansi::render(message);

        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let seq = inner.next_seq;
        inner.next_seq += 1;
        inner.lines.push_back(LogLine {
            seq,
            level,
            target,
            message: rendered.plain,
            html: rendered.html,
        });

        while inner.lines.len() > self.capacity {
            inner.lines.pop_front();
        }
    }

    /// Appends every line newer than `cursor` to `out` and returns the new cursor.
    ///
    /// A cursor of `0` yields the whole retained backlog.
    pub fn drain_since(&self, cursor: u64, out: &mut Vec<LogLine>) -> u64 {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        out.extend(
            inner
                .lines
                .iter()
                .filter(|line| line.seq >= cursor)
                .cloned(),
        );

        inner.next_seq
    }
}

/// One row of the player table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerRow {
    pub name: String,
    pub uuid: String,
    /// `"java"`, `"bedrock"`, or empty when the last join did not record an edition.
    pub edition: String,
    pub ping_ms: i32,
    pub dimension: String,
    pub gamemode: String,
    pub online_secs: u64,
    pub online: bool,
    pub operator: bool,
    pub banned: bool,
    pub whitelisted: bool,
}

/// One row of the world table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldRow {
    pub name: String,
    pub dimension: String,
    pub loaded_chunks: usize,
    pub entities: usize,
    pub players: usize,
    /// `None` until the slow disk sampler has run once.
    pub size_bytes: Option<u64>,
    pub time_of_day: i64,
    pub weather: String,
}

/// static Values.
#[derive(Clone, Debug, Default)]
pub struct ServerMeta {
    pub pumpkin_version: String,
    pub commit: String,
    pub java_version: String,
    pub bedrock_version: String,
    pub java_address: String,
    pub bedrock_address: String,
    pub cpu_cores: usize,
    /// Milliseconds a tick is allowed to take at the configured tick rate.
    ///
    /// Not fixed at 50: `basic_config.tps` is configurable, and the graph's overrun threshold has
    /// to follow it.
    pub tick_budget_ms: f64,
}

/// Everything the dashboard renders, sampled as one consistent set.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// False until the server has finished starting up.
    pub server_ready: bool,

    pub cpu_total: f32,
    pub cpu_per_core: Vec<f32>,
    /// CPU package temperature in degrees Celsius; `None` where no sensor is exposed.
    pub cpu_temp_c: Option<f32>,

    /// Resident set size of this process.
    pub mem_process_rss: u64,
    pub mem_system_used: u64,
    pub mem_system_total: u64,

    pub tps: f64,
    pub mspt: f64,
    /// The server's rolling window of the last 100 tick durations.
    pub tick_times_nanos: Vec<i64>,

    pub players: Vec<PlayerRow>,
    pub worlds: Vec<WorldRow>,

    /// Total size of all world folders; `None` until the slow sampler has run once.
    pub worlds_size_bytes: Option<u64>,
    pub disk_free: u64,
    pub disk_total: u64,

    pub net_in_bps: u64,
    pub net_out_bps: u64,

    pub uptime_secs: u64,
    pub meta: Arc<ServerMeta>,
}

/// Actions the GUI can trigger on the server.
///
/// Implemented on the `pumpkin` side; the GUI only holds a `dyn` reference.
pub trait GuiCommands: Send + Sync {
    /// Runs a console command, exactly as if it had been typed in the terminal.
    fn submit(&self, line: String);

    /// Tab-completion candidates for `line` at `cursor`.
    fn completions(&self, line: &str, cursor: usize) -> Vec<String>;

    /// Begins a graceful shutdown.
    fn request_stop(&self);
}

/// The handle the GUI is started with.
#[derive(Clone)]
pub struct GuiSide {
    pub snapshot: Arc<ArcSwap<Snapshot>>,
    pub logs: Arc<LogRing>,
    /// Set once the server exists; the GUI shows a disabled console until then.
    pub commands: Arc<OnceLock<Arc<dyn GuiCommands>>>,
    pub theme: ThemePreference,
}

impl GuiSide {
    #[must_use]
    pub fn new(theme: ThemePreference, log_buffer_lines: usize) -> Self {
        Self {
            snapshot: Arc::new(ArcSwap::from_pointee(Snapshot::default())),
            logs: Arc::new(LogRing::new(log_buffer_lines)),
            commands: Arc::new(OnceLock::new()),
            theme,
        }
    }

    /// The command sink, or `None` while the server is still starting.
    #[must_use]
    pub fn commands(&self) -> Option<&Arc<dyn GuiCommands>> {
        self.commands.get()
    }
}
