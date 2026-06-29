//! The backend-neutral fault description (architecture.md §7.3).
//!
//! A [`Faults`] value *describes* a fault regime; each executor interprets it
//! (the simulator against its in-memory scheduler, the Jepsen executor against a
//! nemesis). Two ideas from the systems lineage are built in: **swarm testing**
//! (each seed omits a random subset of fault kinds) and **safety vs. liveness
//! modes** (liveness requires the scheduler to heal, then assert progress).

use std::ops::Range;
use std::time::Duration;

use crate::node::NodeId;
use crate::transport::Prob;

/// Whether the schedule injures uniformly (safety) or injures-then-heals
/// (liveness, so progress can be asserted after a healthy window).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Mode {
    /// Inject faults uniformly across the run.
    #[default]
    Safety,
    /// Injure, then heal, then check for progress.
    Liveness,
}

/// The fault kinds a swarm schedule may enable.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FaultKind {
    /// Arbitrary network cuts, including asymmetric.
    Partitions,
    /// Crash with durable state, later restart and rejoin.
    CrashRestart,
    /// Per-link latency / jitter, in milliseconds.
    Latency(Range<u64>),
    /// Packet loss with the given probability.
    Drop(Prob),
    /// Re-delivery (duplication) with the given probability.
    Duplicate(Prob),
    /// Out-of-order delivery.
    Reorder,
}

/// A backend-neutral fault description: either a swarm of enabled kinds, or a
/// scripted timeline of specific events.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Faults {
    spec: FaultSpec,
    mode: Mode,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
enum FaultSpec {
    /// A set of fault kinds the executor may swarm over (omit a random subset
    /// per seed).
    Swarm(Vec<FaultKind>),
    /// A specific, non-random timeline.
    Scripted(Vec<ScriptedEvent>),
}

/// One entry in a scripted fault timeline.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ScriptedEvent {
    /// When (virtual time from the run start) the action fires.
    pub at: Duration,
    /// What happens.
    pub action: ScriptedAction,
}

/// A scripted fault action.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ScriptedAction {
    /// Partition the cluster into the two given groups.
    Partition(Vec<NodeId>, Vec<NodeId>),
    /// Heal every partition.
    HealAll,
    /// Crash a node (durable state retained).
    Crash(NodeId),
    /// Restart a previously-crashed node.
    Restart(NodeId),
}

impl Faults {
    /// Begin a swarm schedule. Add kinds with the builder verbs, then
    /// [`Faults::mode`].
    pub fn swarm() -> Faults {
        Faults {
            spec: FaultSpec::Swarm(Vec::new()),
            mode: Mode::Safety,
        }
    }

    /// Begin a scripted (specific, non-random) schedule.
    pub fn scripted() -> Faults {
        Faults {
            spec: FaultSpec::Scripted(Vec::new()),
            mode: Mode::Safety,
        }
    }

    /// Set the safety/liveness mode.
    pub fn mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
    }

    /// The configured mode.
    pub fn get_mode(&self) -> Mode {
        self.mode
    }

    /// Whether this is a swarm (vs. scripted) description.
    pub fn is_swarm(&self) -> bool {
        matches!(self.spec, FaultSpec::Swarm(_))
    }

    /// The enabled swarm kinds, if this is a swarm description.
    pub fn swarm_kinds(&self) -> Option<&[FaultKind]> {
        match &self.spec {
            FaultSpec::Swarm(kinds) => Some(kinds),
            FaultSpec::Scripted(_) => None,
        }
    }

    /// The scripted timeline, if this is a scripted description.
    pub fn script(&self) -> Option<&[ScriptedEvent]> {
        match &self.spec {
            FaultSpec::Scripted(events) => Some(events),
            FaultSpec::Swarm(_) => None,
        }
    }

    // --- swarm builder verbs ---

    /// Enable arbitrary partitions.
    pub fn partitions(self) -> Self {
        self.push_kind(FaultKind::Partitions)
    }

    /// Enable crash/restart.
    pub fn crash_restart(self) -> Self {
        self.push_kind(FaultKind::CrashRestart)
    }

    /// Enable per-link latency/jitter over `range` milliseconds.
    pub fn latency_ms(self, range: Range<u64>) -> Self {
        self.push_kind(FaultKind::Latency(range))
    }

    /// Enable packet loss.
    pub fn drop(self, p: Prob) -> Self {
        self.push_kind(FaultKind::Drop(p))
    }

    /// Enable duplication.
    pub fn duplicate(self, p: Prob) -> Self {
        self.push_kind(FaultKind::Duplicate(p))
    }

    /// Enable reordering.
    pub fn reorder(self) -> Self {
        self.push_kind(FaultKind::Reorder)
    }

    fn push_kind(mut self, kind: FaultKind) -> Self {
        if let FaultSpec::Swarm(kinds) = &mut self.spec {
            kinds.push(kind);
        }
        // On a scripted spec, a swarm verb is a no-op; the typed constructors
        // (`swarm()` / `scripted()`) keep the two regimes distinct by intent.
        self
    }

    // --- scripted builder ---

    /// Open a scripted step at virtual time `at`. Chain `.partition`, `.heal_all`,
    /// `.crash`, or `.restart`.
    pub fn at(self, at: Duration) -> ScriptStep {
        ScriptStep { faults: self, at }
    }
}

/// A half-built scripted step bound to a timestamp; choose the action to commit.
pub struct ScriptStep {
    faults: Faults,
    at: Duration,
}

impl ScriptStep {
    /// Partition the cluster into two groups at this step's time.
    pub fn partition(self, a: &[u64], b: &[u64]) -> Faults {
        let action = ScriptedAction::Partition(
            a.iter().map(|&n| NodeId(n)).collect(),
            b.iter().map(|&n| NodeId(n)).collect(),
        );
        self.commit(action)
    }

    /// Heal all partitions at this step's time.
    pub fn heal_all(self) -> Faults {
        self.commit(ScriptedAction::HealAll)
    }

    /// Crash a node at this step's time.
    pub fn crash(self, node: u64) -> Faults {
        self.commit(ScriptedAction::Crash(NodeId(node)))
    }

    /// Restart a node at this step's time.
    pub fn restart(self, node: u64) -> Faults {
        self.commit(ScriptedAction::Restart(NodeId(node)))
    }

    fn commit(mut self, action: ScriptedAction) -> Faults {
        if let FaultSpec::Scripted(events) = &mut self.faults.spec {
            events.push(ScriptedEvent {
                at: self.at,
                action,
            });
        }
        self.faults
    }
}
