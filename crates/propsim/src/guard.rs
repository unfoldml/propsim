//! The determinism-regression guard (architecture.md §14.2).
//!
//! Runs the same seed twice and asserts the recorded history is byte-identical.
//! If a dependency starts reading wall-clock time or OS entropy through an
//! un-intercepted path, this fails immediately and points at the divergence —
//! converting "deterministic except in CI" into a failing test.

use propsim_core::node::Node;
use propsim_core::{History, Seed, TestPlan};

/// Assert that `plan` is deterministic at `seed`: two runs produce the identical
/// recorded history. Panics with a divergence pointer otherwise.
///
/// Takes a `plan`-producing closure because a `TestPlan` is consumed by a run.
pub fn assert_deterministic<N, F>(mut make_plan: F, seed: Seed)
where
    N: Node + Clone + Default,
    N::Timer: std::fmt::Debug + Clone,
    N::Msg: Clone,
    F: FnMut() -> TestPlan<N>,
{
    let a = record_history(make_plan(), seed);
    let b = record_history(make_plan(), seed);
    if let Some(pos) = first_divergence(&a, &b) {
        panic!(
            "non-deterministic execution at seed {seed}: histories diverge at entry {pos}\n  \
             run A: {:?}\n  run B: {:?}",
            a.entries().get(pos),
            b.entries().get(pos),
        );
    }
    assert_eq!(
        a.entries().len(),
        b.entries().len(),
        "non-deterministic execution at seed {seed}: history lengths differ ({} vs {})",
        a.entries().len(),
        b.entries().len(),
    );
}

fn record_history<N>(plan: TestPlan<N>, seed: Seed) -> History
where
    N: Node + Clone + Default,
    N::Timer: std::fmt::Debug + Clone,
    N::Msg: Clone,
{
    let (data, node_def, workload, client, _props) = plan.into_parts();
    propsim_sim::run_deterministic::<N>(
        &data,
        node_def.as_ref(),
        workload.as_ref(),
        client.as_deref(),
        seed,
    )
    .map(|r| r.history)
    .unwrap_or_default()
}

fn first_divergence(a: &History, b: &History) -> Option<usize> {
    a.entries()
        .iter()
        .zip(b.entries())
        .position(|(x, y)| x != y)
}
