//! The local IPC listener a connected `pumpkin-gui` process talks to.
//!
//! The server listens (Unix domain socket / Windows named pipe); `pumpkin-gui` connects as a
//! client. This keeps the two processes properly decoupled: the GUI can crash or be closed
//! without taking the server down, and a fresh `pumpkin-gui --attach <endpoint>` can reattach
//! later. When `pumpkin-gui` spawns this server itself, it picks the endpoint and passes it via
//! [`pumpkin_gui_api::GUI_ENDPOINT_ENV`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use pumpkin_config::gui::GuiConfig;
use pumpkin_gui_api::{
    GuiMessage, LogRing, PROTOCOL_VERSION, ServerMessage, ServerMeta, ThemePreference,
    read_message, write_message,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{broadcast, mpsc};

use crate::command::CommandSender;
use crate::plugin::server::server_command::ServerCommandEvent;
use crate::server::Server;
use crate::STOP_INTERRUPT;

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

/// Everything a connection needs that does not come from the socket itself.
struct Listener {
    server: Arc<Server>,
    ring: Arc<LogRing>,
    tx: Broadcaster,
    theme: ThemePreference,
    meta: ServerMeta,
}

impl Listener {
    /// Wires one accepted stream up to its own reader and writer tasks.
    fn serve<S>(self: &Arc<Self>, stream: S)
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        let (read_half, write_half) = tokio::io::split(stream);
        let rx = self.tx.subscribe();
        self.server.clone().spawn_task(handle_connection(
            self.clone(),
            read_half,
            write_half,
            rx,
        ));
    }
}

/// Binds the listener and spawns its accept loop on the server's runtime. Returns the endpoint
/// string to hand to a spawned `pumpkin-gui` process (or to print for a manual `--attach`).
pub fn spawn_listener(
    server: Arc<Server>,
    ring: Arc<LogRing>,
    config: &GuiConfig,
) -> std::io::Result<(String, Broadcaster)> {
    let (tx, _rx) = broadcast::channel(64);
    let _ = BROADCAST.set(tx.clone());

    let listener = Arc::new(Listener {
        theme: ThemePreference::from_config(&config.theme),
        meta: super::sampler::server_meta(&server),
        server,
        ring,
        tx: tx.clone(),
    });

    // Set when `pumpkin-gui` spawned this process itself -> otherwise pick a fresh endpoint.
    let requested = std::env::var(pumpkin_gui_api::GUI_ENDPOINT_ENV).ok();
    let endpoint = bind_and_serve(listener, requested)?;
    // Only after a successful bind, a failed one must fall back to the console, not strand the
    // server with neither a listener nor a console reader.
    ATTACHED.store(true, Ordering::Release);
    Ok((endpoint, tx))
}

#[cfg(not(any(unix, windows)))]
compile_error!(
    "the GUI listener needs a Unix domain socket or a Windows named pipe; build without the `gui` \
     feature on other targets"
);

#[cfg(unix)]
fn bind_and_serve(listener: Arc<Listener>, requested: Option<String>) -> std::io::Result<String> {
    let path = std::path::PathBuf::from(requested.unwrap_or_else(pumpkin_gui_api::unique_endpoint));
    let _ = std::fs::remove_file(&path);
    let socket = tokio::net::UnixListener::bind(&path)?;
    let endpoint = path.to_string_lossy().into_owned();

    listener.server.clone().spawn_task(async move {
        loop {
            // These tasks live on the server's tracker, which `Server::shutdown` waits on, so an
            // uncancelled `accept()` would block the save forever.
            let accepted = tokio::select! {
                accepted = socket.accept() => accepted,
                () = STOP_INTERRUPT.cancelled() => break,
            };
            let Ok((stream, _addr)) = accepted else {
                break;
            };
            listener.serve(stream);
        }
        let _ = std::fs::remove_file(&path);
    });

    Ok(endpoint)
}

#[cfg(windows)]
fn bind_and_serve(listener: Arc<Listener>, requested: Option<String>) -> std::io::Result<String> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let name = requested.unwrap_or_else(pumpkin_gui_api::unique_endpoint);
    let mut pipe = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&name)?;
    let endpoint = name.clone();

    listener.server.clone().spawn_task(async move {
        loop {
            // Cancelled for the same reason as the Unix arm: `Server::shutdown` waits on this
            // task's tracker.
            let connected = tokio::select! {
                connected = pipe.connect() => connected,
                () = STOP_INTERRUPT.cancelled() => break,
            };
            if connected.is_err() {
                break;
            }
            // A pipe instance serves one client, so the next has to exist before this is handed off.
            let next = match ServerOptions::new().create(&name) {
                Ok(next) => next,
                Err(_) => break,
            };
            listener.serve(std::mem::replace(&mut pipe, next));
        }
    });

    Ok(endpoint)
}

async fn handle_connection<R, W>(
    ctx: Arc<Listener>,
    mut read_half: R,
    write_half: W,
    broadcast_rx: broadcast::Receiver<ServerMessage>,
) where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let server = &ctx.server;
    let (out_tx, out_rx) = mpsc::unbounded_channel::<ServerMessage>();

    // Queued before the forwarder can push anything
    let _ = out_tx.send(ServerMessage::Hello {
        protocol: PROTOCOL_VERSION,
        meta: ctx.meta.clone(),
        theme: ctx.theme,
    });

    // The broadcast only reaches GUIs that were already subscribed, and `send` drops the message
    // outright when there are none, so every line logged before this connection existed.
    let mut backlog = Vec::new();
    let from_seq = ctx.ring.drain_since(0, &mut backlog);
    if !backlog.is_empty() {
        let _ = out_tx.send(ServerMessage::LogLines(backlog));
    }

    server.clone().spawn_task(writer_loop(write_half, out_rx));
    server
        .clone()
        .spawn_task(forward_loop(broadcast_rx, out_tx.clone(), from_seq));

    loop {
        // A connected GUI is idle most of the time, so without the cancel arm this read would
        // outlive the shutdown and stall `Server::shutdown`.
        let message = tokio::select! {
            message = read_message::<_, GuiMessage>(&mut read_half) => message,
            () = STOP_INTERRUPT.cancelled() => break,
        };

        match message {
            Ok(GuiMessage::Submit(line)) => submit(server, line),
            Ok(GuiMessage::Complete { id, line, cursor }) => {
                let candidates = crate::logging::console_completions(server, &line, cursor);
                let _ = out_tx.send(ServerMessage::Completions { id, candidates });
            }
            Ok(GuiMessage::RequestStop) => submit(server, "stop".to_owned()),
            Err(_) => break,
        }
    }
}

async fn writer_loop<W: AsyncWrite + Unpin>(
    mut write_half: W,
    mut rx: mpsc::UnboundedReceiver<ServerMessage>,
) {
    loop {
        let msg = tokio::select! {
            msg = rx.recv() => msg,
            () = STOP_INTERRUPT.cancelled() => break,
        };
        let Some(msg) = msg else { break };

        if write_message(&mut write_half, &msg).await.is_err() {
            return;
        }
    }

    // On the way out, flush what is already queued so the GUI still receives `ShuttingDown`
    // instead of just seeing the socket close.
    while let Ok(msg) = rx.try_recv() {
        if write_message(&mut write_half, &msg).await.is_err() {
            return;
        }
    }
}

/// `from_seq` is where the replayed backlog ended. The receiver was subscribed before that
/// replay ran, so it still holds those same lines.
async fn forward_loop(
    mut rx: broadcast::Receiver<ServerMessage>,
    tx: mpsc::UnboundedSender<ServerMessage>,
    from_seq: u64,
) {
    loop {
        // The `BROADCAST` static holds a sender for the life of the process, so this receiver
        // never sees `Closed` on its own and has to be cancelled explicitly.
        let received = tokio::select! {
            received = rx.recv() => received,
            () = STOP_INTERRUPT.cancelled() => break,
        };

        match received {
            Ok(ServerMessage::LogLines(lines)) => {
                let lines: Vec<_> = lines
                    .into_iter()
                    .filter(|line| line.seq >= from_seq)
                    .collect();
                if !lines.is_empty() && tx.send(ServerMessage::LogLines(lines)).is_err() {
                    break;
                }
            }
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
