//! Stall watchdog for the outgoing writer: no byte reaches the socket, connection closes.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncWrite, BufWriter};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::time::Instant;

/// No socket progress for this long while writing: stalled. Below the keep-alive
/// timeout (2 × `keep_alive_time`, default 30s).
pub const STALL_TIMEOUT: Duration = Duration::from_secs(10);

pub type JavaWriteHalf = BufWriter<ProgressWriter<OwnedWriteHalf>>;

/// Last moment the socket accepted bytes.
#[derive(Clone)]
pub struct WriteProgress {
    base: Instant,
    /// Millis since `base`.
    last: Arc<AtomicU64>,
}

impl WriteProgress {
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: Instant::now(),
            last: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn touch(&self) {
        let millis = self.base.elapsed().as_millis() as u64;
        self.last.store(millis, Ordering::Relaxed);
    }

    fn last(&self) -> Instant {
        self.base + Duration::from_millis(self.last.load(Ordering::Relaxed))
    }

    /// Resolves once nothing moved for `STALL_TIMEOUT`.
    pub async fn stalled(&self) {
        loop {
            let deadline = self.last() + STALL_TIMEOUT;
            if Instant::now() >= deadline {
                return;
            }
            tokio::time::sleep_until(deadline).await;
        }
    }
}

impl Default for WriteProgress {
    fn default() -> Self {
        Self::new()
    }
}

/// Stamps `WriteProgress` on every accepted byte and completed flush.
pub struct ProgressWriter<W> {
    inner: W,
    progress: WriteProgress,
}

impl<W> ProgressWriter<W> {
    pub const fn new(inner: W, progress: WriteProgress) -> Self {
        Self { inner, progress }
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for ProgressWriter<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let res = Pin::new(&mut self.inner).poll_write(cx, buf);
        if matches!(res, Poll::Ready(Ok(n)) if n > 0) {
            self.progress.touch();
        }
        res
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let res = Pin::new(&mut self.inner).poll_flush(cx);
        if matches!(res, Poll::Ready(Ok(()))) {
            self.progress.touch();
        }
        res
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
