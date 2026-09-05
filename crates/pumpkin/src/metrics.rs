//! Lightweight process-wide counters.

use std::sync::atomic::{AtomicU64, Ordering};

#[repr(align(64))]
struct PaddedCounter(AtomicU64);

static BYTES_IN: PaddedCounter = PaddedCounter(AtomicU64::new(0));
static BYTES_OUT: PaddedCounter = PaddedCounter(AtomicU64::new(0));

/// Records bytes read from a client.
pub fn record_bytes_in(bytes: u64) {
    BYTES_IN.0.fetch_add(bytes, Ordering::Relaxed);
}

/// Records bytes written to a client.
pub fn record_bytes_out(bytes: u64) {
    BYTES_OUT.0.fetch_add(bytes, Ordering::Relaxed);
}

/// Total bytes read from clients since startup.
#[must_use]
pub fn bytes_in() -> u64 {
    BYTES_IN.0.load(Ordering::Relaxed)
}

/// Total bytes written to clients since startup.
#[must_use]
pub fn bytes_out() -> u64 {
    BYTES_OUT.0.load(Ordering::Relaxed)
}
