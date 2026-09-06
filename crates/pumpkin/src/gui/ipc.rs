//! The local IPC listener a connected `pumpkin-gui` process talks to.
//!
//! The server listens (Unix domain socket / Windows named pipe); `pumpkin-gui` connects as a
//! client. This keeps the two processes properly decoupled: the GUI can crash or be closed
//! without taking the server down, and a fresh `pumpkin-gui --connect <endpoint>` can reattach
//! later.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use pumpkin_config::gui::GuiConfig;
use pumpkin_gui_api::{GuiMessage, ServerMessage, ThemePreference, read_message, write_message};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{broadcast, mpsc};

use crate::command::CommandSender;
use crate::plugin::server::server_command::ServerCommandEvent;
use crate::server::Server;

/// Fans `Snapshot`/`LogLines`/`ShuttingDown` out to every connected GUI.
pub type Broadcaster = broadcast::Sender<ServerMessage>;

static ATTACHED: AtomicBool = AtomicBool::new(false);
static BROADCAST: OnceLock<Broadcaster> = OnceLock::new();

/// True once [`spawn_listener`] has bound the socket, so `PumpkinServer::start` can leave the TTY
/// alone. Set regardless of whether a GUI has actually connected yet.
#[must_use]
pub fn is_attached() -> bool {
    ATTACHED.load(Ordering::Acquire)
}

/// Tells every connected GUI the server is shutting down. No-op if the listener never started.
pub fn notify_shutdown() {
    if let Some(tx) = BROADCAST.get() {
        let _ = tx.send(ServerMessage::ShuttingDown);
    }
}

fn endpoint_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("pumpkin-gui-{}.sock", std::process::id()))
}

/// Binds the listener and spawns its accept loop on the server's runtime. Returns the endpoint
/// string to hand to a spawned `pumpkin-gui` process (or to print for a manual `--connect`).
pub fn spawn_listener(
    server: Arc<Server>,
    config: &GuiConfig,
) -> std::io::Result<(String, Broadcaster)> {
    ATTACHED.store(true, Ordering::Release);

    let (tx, _rx) = broadcast::channel(64);
    let _ = BROADCAST.set(tx.clone());
    let theme = ThemePreference::from_config(&config.theme);

    let endpoint = bind_and_serve(server, tx.clone(), theme)?;
    Ok((endpoint, tx))
}

#[cfg(unix)]
fn bind_and_serve(
    server: Arc<Server>,
    tx: Broadcaster,
    theme: ThemePreference,
) -> std::io::Result<String> {
    let path = endpoint_path();
    let _ = std::fs::remove_file(&path);
    let listener = tokio::net::UnixListener::bind(&path)?;
    let endpoint = path.to_string_lossy().into_owned();

    server.clone().spawn_task(async move {
        loop {
            let Ok((stream, _addr)) = listener.accept().await else {
                break;
            };
            let (read_half, write_half) = tokio::io::split(stream);
            let meta = super::sampler::server_meta(&server);
            let rx = tx.subscribe();
            server.clone().spawn_task(handle_connection(
                server.clone(),
                read_half,
                write_half,
                rx,
                meta,
                theme,
            ));
        }
        let _ = std::fs::remove_file(&path);
    });

    Ok(endpoint)
}

#[cfg(windows)]
fn bind_and_serve(
    server: Arc<Server>,
    tx: Broadcaster,
    theme: ThemePreference,
) -> std::io::Result<String> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let name = format!(r"\\.\pipe\pumpkin-gui-{}", std::process::id());
    let mut pipe = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&name)?;
    let endpoint = name.clone();

    server.clone().spawn_task(async move {
        loop {
            if pipe.connect().await.is_err() {
                break;
            }
            let next = match ServerOptions::new().create(&name) {
                Ok(next) => next,
                Err(_) => break,
            };
            let connected = std::mem::replace(&mut pipe, next);

            let (read_half, write_half) = tokio::io::split(connected);
            let meta = super::sampler::server_meta(&server);
            let rx = tx.subscribe();
            server.clone().spawn_task(handle_connection(
                server.clone(),
                read_half,
                write_half,
                rx,
                meta,
                theme,
            ));
        }
    });

    Ok(endpoint)
}

async fn handle_connection<R, W>(
    server: Arc<Server>,
    mut read_half: R,
    write_half: W,
    broadcast_rx: broadcast::Receiver<ServerMessage>,
    meta: pumpkin_gui_api::ServerMeta,
    theme: ThemePreference,
) where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (out_tx, out_rx) = mpsc::unbounded_channel::<ServerMessage>();

    server.clone().spawn_task(writer_loop(write_half, out_rx));
    server
        .clone()
        .spawn_task(forward_loop(broadcast_rx, out_tx.clone()));

    let _ = out_tx.send(ServerMessage::Hello { meta, theme });

    loop {
        match read_message::<_, GuiMessage>(&mut read_half).await {
            Ok(GuiMessage::Submit(line)) => submit(&server, line),
            Ok(GuiMessage::Complete { id, line, cursor }) => {
                let candidates = crate::logging::console_completions(&server, &line, cursor);
                let _ = out_tx.send(ServerMessage::Completions { id, candidates });
            }
            Ok(GuiMessage::RequestStop) => submit(&server, "stop".to_owned()),
            Err(_) => break,
        }
    }
}

async fn writer_loop<W: AsyncWrite + Unpin>(
    mut write_half: W,
    mut rx: mpsc::UnboundedReceiver<ServerMessage>,
) {
    while let Some(msg) = rx.recv().await {
        if write_message(&mut write_half, &msg).await.is_err() {
            break;
        }
    }
}

async fn forward_loop(
    mut rx: broadcast::Receiver<ServerMessage>,
    tx: mpsc::UnboundedSender<ServerMessage>,
) {
    loop {
        match rx.recv().await {
            Ok(msg) => {
                if tx.send(msg).is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

/// Runs a console command exactly as if it had been typed in the terminal, so plugins see the
/// same event and the command behaves identically either way.
fn submit(server: &Arc<Server>, line: String) {
    let dispatch_server = server.clone();
    server.spawn_task(async move {
        let mut event = ServerCommandEvent::new(line.clone());
        dispatch_server
            .plugin_manager
            .fire(&dispatch_server, &mut event)
            .await;

        if !event.cancelled {
            dispatch_server
                .command_dispatcher
                .load()
                .handle_command(&CommandSender::Console.into_source(&dispatch_server), &line);
        }
    });
}
