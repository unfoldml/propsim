//! The [`Transport`] seam and the backend-neutral network semantics
//! (architecture.md §7.4).
//!
//! Tier 1 defines the description types and the trait shape. The in-memory
//! transport that interprets a [`NetworkModel`] lives in the simulator crate.

use std::ops::Range;

use crate::node::NodeId;

/// A probability in `[0.0, 1.0]`, clamped on construction.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Prob(f64);

/// Construct a [`Prob`], clamping into `[0.0, 1.0]`.
pub fn prob(p: f64) -> Prob {
    Prob(p.clamp(0.0, 1.0))
}

impl Prob {
    /// Probability 0.0 — the event never happens.
    pub const NEVER: Prob = Prob(0.0);
    /// Probability 1.0 — the event always happens.
    pub const ALWAYS: Prob = Prob(1.0);

    /// The underlying probability value.
    pub fn value(self) -> f64 {
        self.0
    }
}

/// A quantitative network model: per-link delay, loss, duplication, ordering.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NetworkModel {
    /// Per-message delay range, in milliseconds.
    pub delay_ms: Range<u64>,
    /// Probability a message is dropped.
    pub loss: Prob,
    /// Probability a delivered message is duplicated.
    pub duplicate: Prob,
    /// Whether per-link delivery is FIFO.
    pub ordered: bool,
}

impl NetworkModel {
    /// Per-link FIFO with no loss, duplication, or delay.
    pub fn ordered_reliable() -> Self {
        NetworkModel {
            delay_ms: 0..0,
            loss: Prob::NEVER,
            duplicate: Prob::NEVER,
            ordered: true,
        }
    }
}

/// Constructors for the in-memory transport's network semantics, extending
/// `stateright`'s `Ordered` / `UnorderedNonDuplicating` / `UnorderedDuplicating`
/// taxonomy with quantitative delay.
///
/// These return a [`NetworkModel`]; the concrete in-memory transport that
/// realizes it is in the simulator crate.
pub struct InMemory;

impl InMemory {
    /// Per-link FIFO, no loss.
    pub fn ordered() -> NetworkModel {
        NetworkModel::ordered_reliable()
    }

    /// Reorder + loss + duplication — an adversarial default.
    pub fn unordered_lossy() -> NetworkModel {
        NetworkModel {
            delay_ms: 0..50,
            loss: prob(0.02),
            duplicate: prob(0.01),
            ordered: false,
        }
    }

    /// An explicit model.
    pub fn with(model: NetworkModel) -> NetworkModel {
        model
    }
}

/// The transport seam an executor installs beneath the nodes.
///
/// Tier 1 fixes only the shape; the in-memory and iroh transports implement it
/// in later-tier crates. `Msg` is the node's wire message type.
pub trait Transport {
    /// The message type carried between nodes.
    type Msg;

    /// Enqueue `msg` for delivery from `from` to `to`, subject to the active
    /// [`NetworkModel`] and fault schedule.
    fn send(&mut self, from: NodeId, to: NodeId, msg: Self::Msg);
}
