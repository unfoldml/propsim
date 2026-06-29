//! # propsim-iroh (SHAPE ONLY)
//!
//! The iroh `Transport` adapter (architecture.md §6.4, §17 Tier 3). It pins the
//! in-memory transport surface that iroh's pluggable custom-transport seam
//! ([n0-computer/iroh#3848]) will bind real `Endpoint`s onto, proving the
//! Tier-1 [`Transport`] trait is sufficient for iroh.
//!
//! **What is implemented now:** the `IrohInMemory` type and its `Transport`
//! impl shape, plus the `IrohInMemory::default()` constructor that slots into
//! `.transport(..)`.
//!
//! **What is deferred:** binding real iroh `Endpoint`s/`Router`s onto this
//! transport and driving `spawn_each` real-async handlers. That path needs the
//! `propsim-intercept` libc shim (to virtualize iroh's internal clock/entropy
//! reads) and the real `iroh` dependency, and lands in a later phase.
//!
//! [n0-computer/iroh#3848]: https://github.com/n0-computer/iroh/issues/3848

#![forbid(unsafe_code)]

use propsim_core::{NetworkModel, NodeId, Transport};

/// An in-memory transport realizing a [`NetworkModel`] beneath real iroh
/// endpoints. Datagrams enqueue into the deterministic scheduler's virtual-time
/// queue with the model's delay/loss/duplication applied.
pub struct IrohInMemory {
    model: NetworkModel,
}

impl IrohInMemory {
    /// A transport with an explicit network model.
    pub fn with(model: NetworkModel) -> Self {
        IrohInMemory { model }
    }

    /// The configured network model.
    pub fn model(&self) -> &NetworkModel {
        &self.model
    }
}

impl Default for IrohInMemory {
    /// Per-link FIFO, reliable — the conservative default for real-stack runs.
    fn default() -> Self {
        IrohInMemory {
            model: NetworkModel::ordered_reliable(),
        }
    }
}

/// An opaque iroh wire frame. Concretized to the real iroh datagram/stream type
/// when the real binding lands; for now it pins the associated `Msg` type.
#[derive(Clone, Debug)]
pub struct IrohFrame(pub Vec<u8>);

impl Transport for IrohInMemory {
    type Msg = IrohFrame;

    fn send(&mut self, _from: NodeId, _to: NodeId, _msg: Self::Msg) {
        // SHAPE: the real impl enqueues into the shared scheduler queue, applying
        // `self.model` delay/loss/duplication. Binding iroh Endpoints onto this
        // transport and driving spawn_each handlers depends on propsim-intercept.
        unimplemented!(
            "iroh Endpoint binding + spawn_each driving lands with propsim-intercept \
             (architecture.md §6.4, §19)"
        )
    }
}
