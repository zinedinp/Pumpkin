//! The wire protocol between the server (listener) and a connected `pumpkin-gui` process
//! (client), plus the length-prefixed framing both sides use to send it.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::model::{LogLine, ServerMeta, Snapshot, ThemePreference};

/// Correlates a `GuiMessage::Complete` request with its `ServerMessage::Completions` response.
pub type RequestId = u32;

/// Name of the environment variable the server sets when it auto-spawns `pumpkin-gui`, carrying
/// the endpoint (a bare Unix socket path or Windows named-pipe name) to connect to.
pub const GUI_ENDPOINT_ENV: &str = "PUMPKIN_GUI_ENDPOINT";

/// Sent from the server to a connected GUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    /// Sent once, immediately after a connection is accepted.
    Hello {
        meta: ServerMeta,
        theme: ThemePreference,
    },
    /// Replaces the previous snapshot; sent roughly every `refresh_ms`.
    Snapshot(Snapshot),
    /// Log lines appended since the last message of this kind (or since connect).
    LogLines(Vec<LogLine>),
    /// Answers a `GuiMessage::Complete` carrying the same id.
    Completions {
        id: RequestId,
        candidates: Vec<String>,
    },
    /// The server is entering graceful shutdown, the GUI should close its window.
    ShuttingDown,
}

/// Sent from a connected GUI to the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GuiMessage {
    /// Runs a console command, exactly as if it had been typed in the terminal.
    Submit(String),
    /// Requests tab-completion candidates for `line` at `cursor`; answered by
    /// `ServerMessage::Completions` carrying the same id.
    Complete {
        id: RequestId,
        line: String,
        cursor: usize,
    },
    /// Begins a graceful shutdown.
    RequestStop,
}

/// A message larger than this is treated as a corrupt stream rather than allocated.
pub const MAX_MESSAGE_LEN: u32 = 8 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("message too large ({0} bytes)")]
    TooLarge(u32),
    #[error("(de)serialization error: {0}")]
    Postcard(#[from] postcard::Error),
}

/// Writes one length-prefixed, postcard-encoded message.
pub async fn write_message<W, T>(writer: &mut W, msg: &T) -> Result<(), WireError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let bytes = postcard::to_allocvec(msg)?;
    let len = u32::try_from(bytes.len()).map_err(|_| WireError::TooLarge(u32::MAX))?;
    writer.write_all(&len.to_le_bytes()).await?;
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

/// Reads one length-prefixed, postcard-encoded message.
pub async fn read_message<R, T>(reader: &mut R) -> Result<T, WireError>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_MESSAGE_LEN {
        return Err(WireError::TooLarge(len));
    }

    let mut buf = vec![0u8; len as usize];
    reader.read_exact(&mut buf).await?;
    Ok(postcard::from_bytes(&buf)?)
}
