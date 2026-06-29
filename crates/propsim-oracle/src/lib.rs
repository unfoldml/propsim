//! # propsim-oracle
//!
//! Native, no-external-process oracles for `propsim` (architecture.md §8.1). All
//! oracles here are history-based ([`Needs::History`](propsim_core::Needs)) and
//! therefore portable across executors.
//!
//! The flagship is a Porcupine-style linearizability checker. White-box
//! invariant / reachability / bounded-liveness checks are **not** here — those
//! are `property::always`/`sometimes`/`eventually_within` in `propsim-core`,
//! evaluated by the deterministic executor in `propsim-sim`.

#![forbid(unsafe_code)]

mod linearizability;
mod model;

pub use linearizability::{linearizable, sequentially_consistent, LinearizabilityOracle};
pub use model::{Response, SequentialModel};
