//! Backend presets (architecture.md §5, §9.1). All return the same `Backend`
//! type, so only the `.run(backend)` line changes between the inner loop, the
//! rigorous check, and the cluster run.

use propsim_core::Backend;
use propsim_sim::DeterministicExecutor;

/// The fast inner loop: the deterministic in-process simulator with native
/// white-box property evaluation. Pure Rust, default features — no JVM, no
/// cluster, no `unsafe`.
pub fn deterministic() -> Backend {
    Backend::custom(DeterministicExecutor::new()).build()
}

/// The rigorous check: the same deterministic execution (so white-box properties
/// still run and shrink cheaply) plus the Elle sound transactional oracle over
/// the same in-process history — no cluster.
///
/// Requires the `elle` feature; until the Elle bridge lands it is a compile-time
/// stub so the preset name exists without pulling a JVM.
#[cfg(feature = "elle")]
pub fn rigorous() -> Backend {
    // TODO(Tier 3): attach propsim_elle::Elle::strict_serializable() here.
    Backend::custom(DeterministicExecutor::new()).build()
}

/// The full-fidelity cluster run via Jepsen. Requires the `jepsen` feature; a
/// stub until the Jepsen executor lands.
#[cfg(feature = "jepsen")]
pub fn jepsen() -> Backend {
    unimplemented!("the Jepsen executor lands in a later phase (architecture.md §19 Phase 2)")
}
