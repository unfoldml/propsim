//! The facade end-to-end: `plan.run(deterministic())` returns a passing report,
//! and a white-box property under a black-box backend is rejected at `run()`.

use std::collections::BTreeSet;

use propsim::prelude::*;
use propsim_core::node::{Ctx, Node};
use propsim_core::{Artifacts, Executor, NodeId, Produces, RunFailure, RunOutput};

#[derive(Clone, Default)]
struct GossipNode {
    known: BTreeSet<u64>,
}
#[derive(Clone, Debug)]
struct Gossip(Vec<u64>);
#[derive(Clone, Debug)]
enum T {
    Bcast,
}

impl Node for GossipNode {
    type Msg = Gossip;
    type Timer = T;
    type Op = ();
    type Response = ();
    fn on_start(&mut self, cx: &mut dyn Ctx<Self>) {
        self.known.insert(cx.me().0);
        cx.set_timer(T::Bcast, std::time::Duration::from_millis(20));
    }
    fn on_msg(&mut self, _f: NodeId, m: Gossip, _cx: &mut dyn Ctx<Self>) {
        self.known.extend(m.0);
    }
    fn on_timer(&mut self, _t: T, cx: &mut dyn Ctx<Self>) {
        cx.broadcast(Gossip(self.known.iter().copied().collect()));
        cx.set_timer(T::Bcast, std::time::Duration::from_millis(20));
    }
}

fn plan() -> TestPlan<GossipNode> {
    Simulation::plan::<GossipNode>()
        .nodes(3)
        .transport(InMemory::ordered())
        .state_machine()
        .check([property::always("ids only", |w: &World<GossipNode>| {
            w.nodes()
                .flat_map(|n| n.known.iter().copied())
                .all(|v| v < 3)
        })])
        .seeds(1)
        .finish()
}

#[test]
fn deterministic_run_passes() {
    let report = plan().run(propsim::deterministic());
    assert!(report.seeds_run >= 1);
    assert!(
        report.verdicts.iter().all(|nv| nv.verdict.valid),
        "all verdicts should pass: {:?}",
        report.verdicts
    );
}

/// A black-box executor (no world snapshots) — like a future Jepsen backend.
#[derive(Clone)]
struct BlackBox;
impl Executor for BlackBox {
    fn capabilities(&self) -> Produces {
        Produces::REAL_CLUSTER
    }
    fn run(&self, _p: &propsim_core::PlanData, seed: Seed) -> RunOutput {
        RunOutput {
            history: propsim_core::History::default(),
            world_trace: None,
            artifacts: Artifacts::new(),
            seed,
        }
    }
}

#[test]
fn white_box_property_rejected_on_black_box_backend() {
    let backend = Backend::custom(BlackBox).build();
    let result = plan().try_run(backend);
    match result {
        Err(RunFailure::Negotiation(e)) => {
            assert!(
                e.to_string().contains("ids only"),
                "should name the offender: {e}"
            );
        }
        other => panic!("expected a negotiation rejection, got {other:?}"),
    }
}
