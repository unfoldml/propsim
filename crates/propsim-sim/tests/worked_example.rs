//! The §11 worked example: gossip reconverges after a partition heals, within a
//! virtual-time deadline. Exercises the full Style-A inner loop end-to-end:
//! scheduler, in-mem transport, scripted partition/heal faults, WorldTrace
//! recording, and white-box property evaluation.

use std::collections::BTreeSet;

use propsim_core::node::{Ctx, Node};
use propsim_core::prelude::*;
use propsim_core::{NodeId, PropertyKind};
use propsim_sim::{evaluate_properties, run_deterministic};

/// A gossip node: holds a set of known values, periodically broadcasts them, and
/// merges any it receives. Each node seeds one value derived from its own id.
#[derive(Clone, Default)]
struct GossipNode {
    me: Option<NodeId>,
    known: BTreeSet<u64>,
}

#[derive(Clone, Debug)]
struct Gossip(Vec<u64>);

#[derive(Clone, Debug)]
enum Timer {
    Broadcast,
}

impl Node for GossipNode {
    type Msg = Gossip;
    type Timer = Timer;
    type Op = ();
    type Response = ();

    fn on_start(&mut self, cx: &mut dyn Ctx<Self>) {
        let me = cx.me();
        self.me = Some(me);
        // Seed a unique value per node so convergence is observable.
        self.known.insert(100 + me.0);
        cx.set_timer(Timer::Broadcast, std::time::Duration::from_millis(50));
    }

    fn on_msg(&mut self, _from: NodeId, msg: Gossip, _cx: &mut dyn Ctx<Self>) {
        for v in msg.0 {
            self.known.insert(v);
        }
    }

    fn on_timer(&mut self, t: Timer, cx: &mut dyn Ctx<Self>) {
        match t {
            Timer::Broadcast => {
                let payload: Vec<u64> = self.known.iter().copied().collect();
                cx.broadcast(Gossip(payload));
                // Re-arm with a little jitter so broadcasts keep flowing.
                let jitter = cx.rng().duration_ms(40..60);
                cx.set_timer(Timer::Broadcast, jitter);
            }
        }
    }
}

fn published_values(n: usize) -> BTreeSet<u64> {
    (0..n as u64).map(|i| 100 + i).collect()
}

/// Build a 7-node gossip plan with a scripted partition at 1s, heal at 3s, and a
/// liveness deadline of `deadline_ms` after the heal.
fn gossip_plan(deadline_ms: u64) -> TestPlan<GossipNode> {
    let n = 7;
    let all = published_values(n);
    let all_for_liveness = all.clone();

    Simulation::plan::<GossipNode>()
        .nodes(n)
        .transport(InMemory::ordered())
        .state_machine()
        .faults(
            // Partition from the very start so the two groups genuinely diverge
            // (each side only learns its own members' values), making the heal
            // strictly necessary for full convergence.
            Faults::scripted()
                .at(millis(1))
                .partition(&[0, 1, 2], &[3, 4, 5, 6])
                .at(secs(3))
                .heal_all()
                .mode(Mode::Liveness),
        )
        .check([
            // Safety (white-box): no node ever reports a value never published.
            property::always("no fabricated values", move |w: &World<GossipNode>| {
                w.nodes()
                    .flat_map(|node| node.known.iter().copied())
                    .all(|v| all.contains(&v))
            }),
            // Liveness (white-box): within `deadline_ms` of the heal, every node
            // holds every published value.
            property::eventually_within(
                "full convergence after heal",
                millis(deadline_ms),
                move |w: &World<GossipNode>| w.nodes().all(|node| node.known == all_for_liveness),
            )
            .after(Event::NetworkHealed),
        ])
        .seeds(1)
        .finish()
}

#[test]
fn gossip_reconverges_after_partition() {
    let plan = gossip_plan(2000);
    let (data, node_def, workload, client, properties) = plan.into_parts();
    let run = run_deterministic::<GossipNode>(
        &data,
        node_def.as_ref(),
        workload.as_ref(),
        client.as_deref(),
        Seed(0xC0FFEE),
    )
    .expect("run");

    // The network healed event was recorded.
    assert!(
        run.events
            .iter()
            .any(|e| matches!(e, propsim_sim::RunEvent::NetworkHealed(_))),
        "expected a NetworkHealed event"
    );

    let verdicts = evaluate_properties(&properties, &run.world_trace, &run.events);
    for v in &verdicts {
        assert!(
            v.verdict.valid,
            "property `{}` should hold but failed: {:?}",
            v.name, v.verdict
        );
    }
}

#[test]
fn impossible_deadline_fails_liveness() {
    // A 1ms deadline after heal is far too short to converge — liveness must fail,
    // while the safety property still holds.
    let plan = gossip_plan(1);
    let (data, node_def, workload, client, properties) = plan.into_parts();
    let run = run_deterministic::<GossipNode>(
        &data,
        node_def.as_ref(),
        workload.as_ref(),
        client.as_deref(),
        Seed(0xC0FFEE),
    )
    .expect("run");

    let verdicts = evaluate_properties(&properties, &run.world_trace, &run.events);
    let safety = verdicts
        .iter()
        .find(|v| v.name == "no fabricated values")
        .unwrap();
    let liveness = verdicts
        .iter()
        .find(|v| v.name == "full convergence after heal")
        .unwrap();

    assert!(safety.verdict.valid, "safety must still hold");
    assert!(
        !liveness.verdict.valid,
        "liveness must fail under a 1ms deadline"
    );
    // The deadline check is one of the EventuallyWithin kind.
    assert!(matches!(
        properties
            .iter()
            .find(|p| p.name() == "full convergence after heal")
            .unwrap()
            .kind(),
        PropertyKind::EventuallyWithin { .. }
    ));
}
