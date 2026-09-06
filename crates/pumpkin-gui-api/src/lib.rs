//! The data contract between the Pumpkin server and any GUI frontend.

mod ansi;
mod endpoint;
mod model;
mod protocol;
/// Host-side sampling; only the server needs it, and it is what pulls in `sysinfo`.
#[cfg(feature = "host")]
mod system;
mod version;

pub use ansi::DEFAULT_LINK_COLOR;
pub use endpoint::unique_endpoint;
pub use model::{
    DiskSpace, LogLevel, LogLine, LogRing, PlayerRow, ServerMeta, Snapshot, SystemStats,
    ThemePreference, WorldRow,
};
pub use protocol::{
    GUI_ENDPOINT_ENV, GuiMessage, MAX_MESSAGE_LEN, PROTOCOL_VERSION, RequestId, ServerMessage,
    WireError, read_message, write_message,
};
#[cfg(feature = "host")]
pub use system::{SystemSampler, directory_size};
pub use version::{
    GUI_VERSION_MARKER, VERSION_PREFIX, format_version_line, is_gui_capable_version_line,
    is_pumpkin_version_line,
};
