//! How a host that embeds the server runs the server's threads.
//!
//! Each thread calls a hook with its role as it starts, and the pools take
//! their sizes from the host; unset, the server keeps its own. Pumpkin
//! knows nothing about priorities: they differ per OS.

use std::sync::OnceLock;

/// What a server thread does, for the host to rank it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadRole {
    /// The tick: the ticker and the simulation pool it runs in.
    Tick,
    /// Throughput work nothing waits on per frame: world generation,
    /// compression, chunk IO and their scheduling.
    Background,
}

/// Pool sizes; `None` keeps the server's own choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ThreadBudget {
    pub simulation: Option<usize>,
    pub compute: Option<usize>,
    pub chunk_gen: Option<usize>,
}

static HOOK: OnceLock<fn(ThreadRole)> = OnceLock::new();
static BUDGET: OnceLock<ThreadBudget> = OnceLock::new();

/// Sets the hook, once per process; `false` if one was set already.
pub fn set_start_hook(hook: fn(ThreadRole)) -> bool {
    HOOK.set(hook).is_ok()
}

/// Sets the pool sizes, once per process; `false` if set already.
pub fn set_budget(budget: ThreadBudget) -> bool {
    BUDGET.set(budget).is_ok()
}

/// The pool sizes the host set, or none.
#[must_use]
pub fn budget() -> ThreadBudget {
    BUDGET.get().copied().unwrap_or_default()
}

/// Called by each server thread as it starts.
pub fn started(role: ThreadRole) {
    if let Some(hook) = HOOK.get() {
        hook(role);
    }
}
