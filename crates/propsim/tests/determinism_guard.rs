//! The determinism-regression guard (architecture.md §14.2): the same seed must
//! reproduce the same history.

use std::collections::BTreeSet;

use propsim::prelude::*;
use propsim_core::history::Value;
use propsim_core::node::{Ctx, Node};
use propsim_core::NodeId;

/// A node that draws all randomness through `cx.rng()`, so two runs at the same
/// seed are byte-identical.
#[derive(Clone, Default)]
struct GoodNode {
    known: BTreeSet<u64>,
}

#[derive(Clone, Debug)]
struct Msg(u64);
#[derive(Clone, Debug)]
enum T {
    Tick,
}

impl Node for GoodNode {
    type Msg = Msg;
    type Timer = T;
    type Op = ();
    type Response = ();
    fn on_start(&mut self, cx: &mut dyn Ctx<Self>) {
        self.known.insert(cx.me().0);
        cx.set_timer(T::Tick, std::time::Duration::from_millis(10));
    }
    fn on_msg(&mut self, _from: NodeId, m: Msg, _cx: &mut dyn Ctx<Self>) {
        self.known.insert(m.0);
    }
    fn on_timer(&mut self, _t: T, cx: &mut dyn Ctx<Self>) {
        let pick = cx.rng().next_u64() % 100;
        cx.broadcast(Msg(pick));
        cx.set_timer(T::Tick, std::time::Duration::from_millis(10));
    }
}

/// A workload whose recorded op values depend on the seed, so the history is
/// seed-sensitive (and the guard would catch a seed-affecting regression).
fn workload() -> proptest::strategy::BoxedStrategy<propsim_core::FrozenOp> {
    use proptest::prelude::*;
    (0u64..1_000_000)
        .prop_map(|v| {
            propsim_core::FrozenOp::new(
                NodeId(0),
                Value::List(vec![Value::keyword("w"), Value::Int(v as i64)]),
            )
        })
        .boxed()
}

fn plan() -> TestPlan<GoodNode> {
    Simulation::plan::<GoodNode>()
        .nodes(4)
        .transport(InMemory::unordered_lossy())
        .state_machine()
        .workload(workload())
        .seeds(8)
        .finish()
}

#[test]
fn same_seed_is_deterministic() {
    // Runs the plan twice at the same seed; asserts byte-identical history.
    assert_deterministic(plan, Seed(0xC0FFEE));
    assert_deterministic(plan, Seed(0x1234_5678));
}

#[test]
fn different_seeds_produce_different_histories() {
    // Confirms the history is genuinely seed-sensitive — so the guard above is
    // not vacuously passing on a constant history.
    let h1 = record(Seed(1));
    let h2 = record(Seed(2));
    assert_ne!(
        h1.to_jepsen_edn(),
        h2.to_jepsen_edn(),
        "two different seeds should produce different op histories"
    );
}

fn record(seed: Seed) -> propsim_core::History {
    let (data, node_def, wl, client, _props) = plan().into_parts();
    propsim_sim::run_deterministic::<GoodNode>(
        &data,
        node_def.as_ref(),
        wl.as_ref(),
        client.as_deref(),
        seed,
    )
    .unwrap()
    .history
}
