//! The read-only [`World`] a property predicate observes, and the [`WorldTrace`]
//! an executor records (architecture.md §7.2).
//!
//! A predicate receives a snapshot of every node's state plus the recorded
//! operation history and the current virtual time: `w.nodes()`, `w.history()`,
//! `w.now()`. Tier 1 defines the read surface; the deterministic executor
//! populates it.

use std::time::Duration;

use propsim_history::History;

/// A snapshot of the whole system at one instant: every node's internal state,
/// the operation history so far, and the current virtual time.
///
/// `N` is the node state type. White-box (`Needs::World`) properties read this;
/// it is only available on an executor that produces world snapshots.
pub struct World<'a, N> {
    nodes: &'a [N],
    history: &'a History,
    now: Duration,
}

impl<'a, N> World<'a, N> {
    /// Construct a world view. Executors call this to present a snapshot to
    /// property predicates.
    pub fn new(nodes: &'a [N], history: &'a History, now: Duration) -> Self {
        World {
            nodes,
            history,
            now,
        }
    }

    /// Every node's current internal state.
    pub fn nodes(&self) -> impl Iterator<Item = &N> {
        self.nodes.iter()
    }

    /// The recorded client operation history.
    pub fn history(&self) -> &History {
        self.history
    }

    /// The current virtual time.
    pub fn now(&self) -> Duration {
        self.now
    }
}

/// The recorded sequence of world snapshots from one run — the white-box trace
/// an executor attaches to its [`RunOutput`](crate::RunOutput) when it produces
/// world snapshots.
///
/// Generic over a node state type `N`. Each frame pairs a virtual timestamp with
/// the full per-node state at that instant.
pub struct WorldTrace<N = ErasedState> {
    /// `(virtual time, per-node state)` frames in order.
    pub frames: Vec<(Duration, Vec<N>)>,
    /// The history accumulated over the run, shared by every frame's `World`.
    pub history: History,
}

impl<N> WorldTrace<N> {
    /// An empty trace — no frames and an empty history.
    pub fn new() -> Self {
        WorldTrace {
            frames: Vec::new(),
            history: History::default(),
        }
    }
}

impl<N> Default for WorldTrace<N> {
    fn default() -> Self {
        WorldTrace::new()
    }
}

/// A placeholder state type for a type-erased trace.
///
/// The deterministic executor records a concretely-typed `WorldTrace<N>`; where
/// the node type is not statically known (e.g. inside [`RunOutput`](crate::RunOutput), which is
/// backend-neutral), this stands in. The concrete down-cast happens in the
/// executor crate that owns `N`.
#[derive(Clone, Debug, Default)]
pub struct ErasedState;

// `RunOutput::world_trace` is `Option<WorldTrace>` (the erased form) so the
// normalized output shape does not carry a node type parameter through the
// backend-neutral plumbing. Manual Clone/Debug so the alias is ergonomic.
impl Clone for WorldTrace<ErasedState> {
    fn clone(&self) -> Self {
        WorldTrace {
            frames: self.frames.clone(),
            history: self.history.clone(),
        }
    }
}

impl std::fmt::Debug for WorldTrace<ErasedState> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorldTrace")
            .field("frames", &self.frames.len())
            .field("history_len", &self.history.entries().len())
            .finish()
    }
}
