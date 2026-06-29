//! Lowering a backend-neutral [`propsim_core::Faults`] description onto the
//! scheduler timeline (architecture.md §7.3).
//!
//! - **Scripted** faults map one-to-one onto timed [`FaultAction`]s.
//! - **Swarm** faults pick a random *subset* of the enabled kinds per seed
//!   (swarm testing), fold continuous kinds (latency/loss/dup) into the active
//!   [`NetworkModel`], and schedule timed partition/crash events. In
//!   `Mode::Liveness` the schedule injures, then heals, so a bounded-liveness
//!   property has a window in which to make progress.

use std::time::Duration;

use propsim_core::faults::{FaultKind, Faults, Mode, ScriptedAction};
use propsim_core::{NetworkModel, NodeId};

use crate::rng::SimRng;
use crate::scheduler::{FaultAction, SchedCore};
use propsim_core::node::Node;

/// Apply a `Faults` description to a freshly-constructed scheduler: mutate the
/// active network model (for swarm continuous kinds) and schedule discrete
/// fault events on the timeline.
pub fn lower<N: Node>(
    faults: Option<&Faults>,
    model: &mut NetworkModel,
    core: &mut SchedCore<N>,
    rng: &mut SimRng,
    node_count: usize,
) where
    N::Timer: std::fmt::Debug,
    N::Msg: Clone,
{
    let Some(faults) = faults else { return };

    if let Some(script) = faults.script() {
        for ev in script {
            let action = match &ev.action {
                ScriptedAction::Partition(a, b) => FaultAction::Partition(a.clone(), b.clone()),
                ScriptedAction::HealAll => FaultAction::HealAll,
                ScriptedAction::Crash(n) => FaultAction::Crash(*n),
                ScriptedAction::Restart(n) => FaultAction::Restart(*n),
            };
            core.schedule_fault(ev.at, action);
        }
        return;
    }

    if let Some(kinds) = faults.swarm_kinds() {
        lower_swarm(kinds, faults.get_mode(), model, core, rng, node_count);
    }
}

fn lower_swarm<N: Node>(
    kinds: &[FaultKind],
    mode: Mode,
    model: &mut NetworkModel,
    core: &mut SchedCore<N>,
    rng: &mut SimRng,
    node_count: usize,
) where
    N::Timer: std::fmt::Debug,
    N::Msg: Clone,
{
    // Swarm testing: omit a random subset of the enabled kinds for this seed.
    let enabled: Vec<&FaultKind> = kinds.iter().filter(|_| rng.chance(0.5)).collect();

    let mut scheduled_partition = false;
    for kind in &enabled {
        match kind {
            FaultKind::Latency(range) => {
                model.delay_ms = range.clone();
            }
            FaultKind::Drop(p) => model.loss = *p,
            FaultKind::Duplicate(p) => model.duplicate = *p,
            FaultKind::Reorder => model.ordered = false,
            FaultKind::Partitions => {
                if node_count >= 2 {
                    // Split the cluster at a random index.
                    let cut = rng.range_u64(1, node_count as u64) as usize;
                    let a: Vec<NodeId> = (0..cut as u64).map(NodeId).collect();
                    let b: Vec<NodeId> = (cut as u64..node_count as u64).map(NodeId).collect();
                    let at = Duration::from_millis(rng.range_u64(100, 1000));
                    core.schedule_fault(at, FaultAction::Partition(a, b));
                    scheduled_partition = true;
                }
            }
            FaultKind::CrashRestart => {
                let victim = NodeId(rng.range_u64(0, node_count.max(1) as u64));
                let crash_at = Duration::from_millis(rng.range_u64(100, 1000));
                core.schedule_fault(crash_at, FaultAction::Crash(victim));
                if matches!(mode, Mode::Liveness) {
                    let restart_at = crash_at + Duration::from_millis(rng.range_u64(500, 2000));
                    core.schedule_fault(restart_at, FaultAction::Restart(victim));
                }
            }
        }
    }

    // Liveness mode must heal so progress can be asserted after the window.
    if matches!(mode, Mode::Liveness) && scheduled_partition {
        let heal_at = Duration::from_millis(rng.range_u64(2000, 4000));
        core.schedule_fault(heal_at, FaultAction::HealAll);
    }
}
