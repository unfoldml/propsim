//! The [`Scenario`] — the portable, cross-backend reproduction artifact
//! (architecture.md §14.1).
//!
//! The *seed* reproduces the deterministic executor only. The portable artifact
//! that crosses to the rigorous/Jepsen backends is the **`Scenario`**: the
//! frozen, already-shrunk op stream plus the fault schedule. We shrink cheap on
//! the simulator and reproduce expensive elsewhere by replaying it.

use propsim_history::Value;

use crate::faults::Faults;
use crate::node::NodeId;
use crate::seed::Seed;

/// One generated client operation, frozen to a concrete value.
///
/// The simulator's `proptest` `Strategy` generates a concrete op stream per seed;
/// this records that stream so other backends replay *exactly* it (architecture.md
/// §7.5). The op payload is held as a [`Value`] so the frozen stream is
/// node-type-agnostic and serializable without a generic parameter on `Scenario`.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FrozenOp {
    /// The process (client) that issues this op. Per-process ops are issued
    /// sequentially (one in flight at a time), so overlap arises across processes.
    pub process: NodeId,
    /// The node this op is routed to. `None` defaults to `process % node_count`.
    /// Set it to direct a request at a specific node (e.g. a read at a backup).
    #[cfg_attr(feature = "serde", serde(default))]
    pub route: Option<NodeId>,
    /// The operation payload, frozen to a concrete value.
    pub op: Value,
}

impl FrozenOp {
    /// An op issued by `process`, routed by the default `process % node_count` rule.
    pub fn new(process: NodeId, op: Value) -> Self {
        FrozenOp {
            process,
            route: None,
            op,
        }
    }

    /// Route this op at a specific node.
    pub fn routed_to(mut self, node: NodeId) -> Self {
        self.route = Some(node);
        self
    }
}

/// A frozen, replayable execution: the concrete op stream and the fault schedule.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Scenario {
    /// The seed this scenario was derived from (informational; the scenario, not
    /// the seed, is what crosses backends).
    pub origin_seed: Option<Seed>,
    /// The frozen client op stream, in issue order.
    pub ops: Vec<FrozenOp>,
    /// The fault schedule to apply during replay.
    pub faults: Faults,
}

impl Scenario {
    /// A scenario with the given op stream and faults, not tied to a seed.
    pub fn new(ops: Vec<FrozenOp>, faults: Faults) -> Self {
        Scenario {
            origin_seed: None,
            ops,
            faults,
        }
    }

    /// Record the seed this scenario came from.
    pub fn from_seed(mut self, seed: Seed) -> Self {
        self.origin_seed = Some(seed);
        self
    }
}

// `save`/`load` require serialization and so are gated behind the `serde`
// feature, keeping the default Tier-1 dependency set to just proptest +
// propsim-history. JSON is used as the on-disk form for human inspectability.
#[cfg(feature = "serde")]
mod persist {
    use super::Scenario;
    use std::io;
    use std::path::Path;

    impl Scenario {
        /// Serialize this scenario to `path` as JSON.
        pub fn save(&self, path: &Path) -> io::Result<()> {
            let json = serde_json::to_string_pretty(self)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            std::fs::write(path, json)
        }

        /// Load a scenario from a JSON file written by [`Scenario::save`].
        pub fn load(path: &Path) -> io::Result<Scenario> {
            let bytes = std::fs::read(path)?;
            serde_json::from_slice(&bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
        }
    }
}
