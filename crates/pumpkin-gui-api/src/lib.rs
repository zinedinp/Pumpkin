//! The data contract between the Pumpkin server and any GUI frontend.

mod ansi;
mod endpoint;
mod model;
mod protocol;
mod system;

pub use endpoint::unique_endpoint;
pub use model::{
    LogLevel, LogLine, LogRing, PlayerRow, ServerMeta, Snapshot, ThemePreference, WorldRow,
};
pub use protocol::{
    GUI_ENDPOINT_ENV, GuiMessage, MAX_MESSAGE_LEN, RequestId, ServerMessage, WireError,
    read_message, write_message,
};
pub use system::{DiskSpace, SystemSampler, SystemStats, directory_size};
