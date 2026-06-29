//! # propsim
//!
//! Property-based testing for distributed protocols in Rust (architecture.md).
//! Author one backend-neutral `TestPlan` and run it on the deterministic
//! in-process simulator. This is the batteries-included **facade**: it
//! re-exports the stable contract (`propsim-core`) and bundles the default
//! deterministic executor (`propsim-sim`) and native oracles (`propsim-oracle`).
//!
//! With **default features** it is pure Rust — no JVM, no Go, no cluster, no
//! `unsafe` (architecture.md §17.3 litmus test).
//!
//! ```no_run
//! use propsim::prelude::*;
//!
//! # #[derive(Clone, Default)] struct Replica;
//! # impl Node for Replica { type Msg = (); type Timer = (); type Op = (); type Response = ();
//! #   fn on_start(&mut self, _: &mut dyn Ctx<Self>) {}
//! #   fn on_msg(&mut self, _: NodeId, _: (), _: &mut dyn Ctx<Self>) {}
//! #   fn on_timer(&mut self, _: (), _: &mut dyn Ctx<Self>) {} }
//! fn plan() -> TestPlan<Replica> {
//!     Simulation::plan::<Replica>()
//!         .nodes(5)
//!         .transport(InMemory::unordered_lossy())
//!         .state_machine()
//!         .finish()
//! }
//!
//! # fn main() {
//! plan().run(propsim::deterministic());   // ms, laptop, plain CI
//! # }
//! ```

#![forbid(unsafe_code)]

mod backend_presets;
mod guard;
mod run;

// Re-export the whole stable contract so users name one crate.
pub use propsim_core::*;

// The deterministic executor and native oracles, surfaced through the facade.
pub use propsim_oracle::{linearizable, sequentially_consistent, Response, SequentialModel};
pub use propsim_sim::DeterministicExecutor;

pub use backend_presets::deterministic;
#[cfg(feature = "jepsen")]
pub use backend_presets::jepsen;
#[cfg(feature = "elle")]
pub use backend_presets::rigorous;
pub use guard::assert_deterministic;
pub use run::Run;

/// The convenience prelude: glob-import to author tests.
pub mod prelude {
    pub use propsim_core::prelude::*;
    pub use propsim_core::property;

    pub use crate::backend_presets::deterministic;
    pub use crate::guard::assert_deterministic;
    pub use crate::run::Run;
    pub use propsim_oracle::{linearizable, sequentially_consistent, Response, SequentialModel};
}
