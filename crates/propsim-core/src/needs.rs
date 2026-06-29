//! Capability descriptors — the two halves of capability negotiation
//! (architecture.md §9.1).

/// What an [`Executor`](crate::Executor) can produce for oracles to consume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Produces {
    /// White-box per-node internal state (a [`WorldTrace`](crate::WorldTrace)).
    pub world_snapshots: bool,
    /// The client operation history (true in practice for every executor).
    pub history: bool,
    /// Deadlines expressed in compressible virtual time.
    pub virtual_time: bool,
    /// Byte-for-byte replay from an 8-byte [`Seed`](crate::Seed).
    pub seed_replayable: bool,
}

impl Produces {
    /// The deterministic in-process executor's capabilities: everything.
    pub const DETERMINISTIC: Produces = Produces {
        world_snapshots: true,
        history: true,
        virtual_time: true,
        seed_replayable: true,
    };

    /// A real-cluster (Jepsen) executor's capabilities: history only, black-box,
    /// wall-clock, not seed-replayable.
    pub const REAL_CLUSTER: Produces = Produces {
        world_snapshots: false,
        history: true,
        virtual_time: false,
        seed_replayable: false,
    };
}

/// What an [`Oracle`](crate::Oracle) needs to render a verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Needs {
    /// Reads every node's internal state — a white-box oracle. Cannot run on a
    /// black-box executor.
    World,
    /// Reads only the recorded client operation history — portable across every
    /// executor.
    History,
}
