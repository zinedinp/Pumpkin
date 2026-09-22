//! Outbound payload bytes not yet written: per connection and server-wide.

use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Per connection.
pub const MAX_PENDING_BYTES: usize = 64 * 1024 * 1024; // 64 MB
/// All connections together.
pub const MAX_GLOBAL_PENDING_BYTES: usize = 1024 * 1024 * 1024; // 1 GB
/// Over the global budget, only connections holding more than this are kicked.
/// Healthy connections drain within a tick and stay far below.
pub const GLOBAL_KICK_FLOOR: usize = 4 * 1024 * 1024; // 4 MB

/// Always >= the sum of all live `PendingBytes`: added first, released last.
static GLOBAL_PENDING_BYTES: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, PartialEq, Eq)]
pub enum Overflow {
    Connection(usize),
    Global(usize),
}

impl fmt::Display for Overflow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connection(bytes) => write!(
                f,
                "outbound packet buffer overflow ({bytes} bytes > {MAX_PENDING_BYTES} bytes)"
            ),
            Self::Global(bytes) => write!(
                f,
                "server outbound budget exceeded ({bytes} bytes > {MAX_GLOBAL_PENDING_BYTES} bytes) \
                 while holding more than {GLOBAL_KICK_FLOOR} bytes"
            ),
        }
    }
}

const fn check(local: usize, global: usize) -> Result<(), Overflow> {
    if local > MAX_PENDING_BYTES {
        Err(Overflow::Connection(local))
    } else if global > MAX_GLOBAL_PENDING_BYTES && local > GLOBAL_KICK_FLOOR {
        Err(Overflow::Global(global))
    } else {
        Ok(())
    }
}

/// One connection's share. Returns what is left to the global budget on drop.
#[derive(Default)]
pub struct PendingBytes {
    local: AtomicUsize,
}

impl PendingBytes {
    /// Rolled back on `Err`: caller drops the packet and closes the connection.
    pub fn reserve(&self, bytes: usize) -> Result<(), Overflow> {
        let global = GLOBAL_PENDING_BYTES
            .fetch_add(bytes, Ordering::AcqRel)
            .saturating_add(bytes);
        let local = self
            .local
            .fetch_add(bytes, Ordering::AcqRel)
            .saturating_add(bytes);
        check(local, global).inspect_err(|_| self.release(bytes))
    }

    /// Unchecked. Disconnect packets only.
    pub fn add(&self, bytes: usize) {
        GLOBAL_PENDING_BYTES.fetch_add(bytes, Ordering::AcqRel);
        self.local.fetch_add(bytes, Ordering::AcqRel);
    }

    /// Saturating: never releases more than this connection holds.
    pub fn release(&self, bytes: usize) {
        let prev = self
            .local
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |val| {
                Some(val.saturating_sub(bytes))
            })
            .unwrap_or_else(|val| val);
        GLOBAL_PENDING_BYTES.fetch_sub(prev.min(bytes), Ordering::AcqRel);
    }

    #[must_use]
    pub fn load(&self) -> usize {
        self.local.load(Ordering::Relaxed)
    }
}

impl Drop for PendingBytes {
    fn drop(&mut self) {
        GLOBAL_PENDING_BYTES.fetch_sub(*self.local.get_mut(), Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_saturates() {
        let pending = PendingBytes::default();
        pending.reserve(100).unwrap();
        pending.release(40);
        assert_eq!(pending.load(), 60);
        pending.release(100);
        assert_eq!(pending.load(), 0);
    }

    #[test]
    fn connection_overflow_rolls_back() {
        let pending = PendingBytes::default();
        pending.reserve(MAX_PENDING_BYTES).unwrap();
        assert!(matches!(pending.reserve(1), Err(Overflow::Connection(_))));
        assert_eq!(pending.load(), MAX_PENDING_BYTES);
    }

    #[test]
    fn global_budget_only_kicks_above_the_floor() {
        let over = MAX_GLOBAL_PENDING_BYTES + 1;
        assert_eq!(check(GLOBAL_KICK_FLOOR, over), Ok(()));
        assert_eq!(
            check(GLOBAL_KICK_FLOOR + 1, over),
            Err(Overflow::Global(over))
        );
        assert_eq!(
            check(GLOBAL_KICK_FLOOR + 1, MAX_GLOBAL_PENDING_BYTES),
            Ok(())
        );
    }
}
