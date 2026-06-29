//! # propsim-core
//!
//! The stable plugin contract for `propsim` (architecture.md §9, §17). This is
//! the surface external executors and oracles compile against, plus the
//! backend-neutral authoring types (`Property`, `Faults`, `Seed`, `Scenario`).
//!
//! Tier 1 of the crate family: it deliberately ships only the contract. The one
//! piece of real behavior that lives here is **capability negotiation**
//! ([`backend::validate`]); executors and oracles are implemented in later-tier
//! crates against the traits defined here.
//!
//! `proptest` is part of the public API (the `Strategy` passed to
//! [`PlanBuilder::workload`]), so a `proptest` major bump implies a
//! `propsim-core` major bump.

#![forbid(unsafe_code)]

pub mod backend;
pub mod faults;
pub mod node;
pub mod scenario;
pub mod transport;

mod executor;
mod needs;
mod plan;
mod property_impl;
mod seed;
mod world;

/// The three property constructors — `property::always`, `property::sometimes`,
/// `property::eventually_within` (architecture.md §7.2).
pub mod property {
    pub use crate::property_impl::constructors::*;
}

// Re-export the history interchange so plugin authors name one crate.
pub use propsim_history as history;
pub use propsim_history::{Clock, Function, History, OpEntry, OpKind, ProcessId, Time, Value};

pub use backend::{validate, Backend, BackendBuilder, BackendError};
pub use executor::{Artifacts, Executor, RunOutput};
pub use faults::{FaultKind, Faults, Mode, ScriptedAction, ScriptedEvent};
pub use needs::{Needs, Produces};
pub use node::{ClientCodec, Completion, Ctx, Node, NodeId, OpOutcome, OpToken, Rng};
pub use oracle::{Anomaly, Oracle, Verdict, Witness};
pub use plan::{
    NamedVerdict, NodeDef, PlanBuilder, PlanData, PlanParts, Report, RunFailure, Simulation,
    TestPlan,
};
pub use property_impl::{millis, secs, Event, Property, PropertyKind};
pub use scenario::{FrozenOp, Scenario};
pub use seed::{Seed, SeedParseError};
pub use transport::{prob, InMemory, NetworkModel, Prob, Transport};
pub use world::{ErasedState, World, WorldTrace};

pub mod oracle;

/// The convenience prelude: glob-import to author tests.
pub mod prelude {
    pub use crate::backend::{Backend, BackendError};
    pub use crate::faults::{Faults, Mode};
    pub use crate::node::{ClientCodec, Completion, Ctx, Node, NodeId, OpOutcome, OpToken};
    pub use crate::property;
    pub use crate::property_impl::{millis, secs, Event, Property};
    pub use crate::scenario::Scenario;
    pub use crate::seed::Seed;
    pub use crate::transport::{prob, InMemory};
    pub use crate::world::World;
    pub use crate::{Simulation, TestPlan};
}
