//! The single-threaded, virtual-time discrete-event scheduler.
//!
//! Determinism rests on three rules (architecture.md §12):
//! 1. one thread, no OS-thread scheduling;
//! 2. the event heap orders by `(at, seq)` where `seq` is a global monotonic
//!    enqueue counter, so events at the same virtual instant fire in a fixed,
//!    seed-independent order — the run is byte-reproducible from the seed;
//! 3. all randomness flows from the one seeded [`SimRng`]; decision paths use
//!    ordered containers, never `HashMap` iteration order.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::time::Duration;

use propsim_core::history::{Function, OpEntry, OpKind, ProcessId, Value};
use propsim_core::node::{Node, OpToken};
use propsim_core::{History, NetworkModel, NodeId, Time};

use crate::rng::SimRng;

/// A run-level event a property's `.after(..)` can anchor a deadline to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunEvent {
    /// All partitions have healed, at the given virtual time.
    NetworkHealed(Duration),
    /// A node rejoined after a crash, at the given virtual time.
    NodeRejoined(Duration, NodeId),
}

/// A scheduled event, ordered by `(at, seq)` in the heap.
struct Scheduled<M, T> {
    at: Duration,
    seq: u64,
    kind: EventKind<M, T>,
}

/// What a scheduled event does when it fires.
enum EventKind<M, T> {
    /// Deliver `msg` from→to.
    Deliver { from: NodeId, to: NodeId, msg: M },
    /// Fire timer `t` on `node`. `gen` guards against firing a cancelled/replaced timer.
    Timer { node: NodeId, timer: T, gen: u64 },
    /// Issue a client op (recorded into the history by the caller).
    ClientOp { process: NodeId, index: usize },
    /// Apply a fault action.
    Fault { action: FaultAction },
}

/// A resolved fault action on the timeline (lowered from `propsim_core::Faults`).
#[derive(Clone, Debug)]
pub enum FaultAction {
    Partition(Vec<NodeId>, Vec<NodeId>),
    HealAll,
    Crash(NodeId),
    Restart(NodeId),
}

// Ordering: smaller `at` first, then smaller `seq`. Wrapped in `Reverse` in the
// heap to turn the max-heap into a min-heap.
impl<M, T> PartialEq for Scheduled<M, T> {
    fn eq(&self, other: &Self) -> bool {
        self.at == other.at && self.seq == other.seq
    }
}
impl<M, T> Eq for Scheduled<M, T> {}
impl<M, T> PartialOrd for Scheduled<M, T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<M, T> Ord for Scheduled<M, T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.at.cmp(&other.at).then(self.seq.cmp(&other.seq))
    }
}

/// The scheduler state that is *not* the node slice — so a `HarnessCtx` can
/// borrow it mutably while the active node is borrowed separately.
pub struct SchedCore<N: Node> {
    pub now: Duration,
    pub rng: SimRng,
    pub model: NetworkModel,
    /// Virtual-time run horizon: events scheduled at or after this are not fired,
    /// so a protocol that re-arms timers forever still terminates. Time is
    /// compressed, so a generous horizon costs nothing.
    pub horizon: Duration,
    /// Min-heap of pending events.
    queue: BinaryHeap<Reverse<Scheduled<N::Msg, N::Timer>>>,
    seq: u64,
    /// Per-node current timer generation; bumped on set/cancel so stale Timer
    /// events are ignored when popped (lazy cancellation).
    timer_gen: BTreeMap<(u64, TimerKey), u64>,
    /// Active partition groups; empty means fully connected.
    partitions: Vec<Vec<NodeId>>,
    /// Crashed nodes (state retained, delivery suppressed).
    crashed: Vec<NodeId>,
    /// Per-link last scheduled delivery time, for FIFO when `model.ordered`.
    last_link: BTreeMap<(u64, u64), Duration>,
    /// The recorded operation history.
    pub history: History,
    /// Run-level events for deadline anchoring.
    pub events: Vec<RunEvent>,
    /// Client ops awaiting completion, keyed by token. `BTreeMap` so force-close
    /// (on crash / horizon) drains in a deterministic order.
    pending_ops: BTreeMap<u64, PendingOp>,
    next_token: u64,
    node_count: usize,
}

/// An open client op: the recorded `Invoke` it pairs with, and the `:f` to reuse
/// for its completion entry.
struct PendingOp {
    process: ProcessId,
    f: Function,
}

/// A stable key for a node's timer, derived from the `Debug` form of the tag.
/// Timer tags are small enums; their `Debug` string is a cheap stable identity
/// without requiring `Hash`/`Ord` bounds on the associated `Timer` type.
type TimerKey = String;

impl<N: Node> SchedCore<N>
where
    N::Timer: std::fmt::Debug,
{
    pub fn new(seed: u64, model: NetworkModel, node_count: usize) -> Self {
        SchedCore {
            now: Duration::ZERO,
            rng: SimRng::new(seed),
            model,
            // Default horizon: 10 virtual seconds. The driver widens this to
            // cover the latest scripted fault plus any liveness deadline.
            horizon: Duration::from_secs(10),
            queue: BinaryHeap::new(),
            seq: 0,
            timer_gen: BTreeMap::new(),
            partitions: Vec::new(),
            crashed: Vec::new(),
            last_link: BTreeMap::new(),
            history: History::default(),
            events: Vec::new(),
            pending_ops: BTreeMap::new(),
            next_token: 0,
            node_count,
        }
    }

    /// Record an `Invoke` at the current virtual time, register a pending op, and
    /// return its token. The completion entry is recorded later by [`close_op`].
    pub(crate) fn open_op(&mut self, process: ProcessId, f: Function, value: Value) -> OpToken {
        let index = self.history.0.len() as u64;
        self.history.0.push(OpEntry {
            index,
            time: Time::virtual_nanos(now_nanos(self.now)),
            kind: OpKind::Invoke,
            process,
            f: f.clone(),
            value,
        });
        let tok = self.next_token;
        self.next_token += 1;
        self.pending_ops.insert(tok, PendingOp { process, f });
        OpToken(tok)
    }

    /// Close a pending op at the current virtual time, recording its terminal
    /// entry with the same `:f` as its `Invoke`. A no-op on an unknown/closed
    /// token (so double-completion is safe).
    pub(crate) fn close_op(&mut self, token: OpToken, kind: OpKind, value: Value) {
        if let Some(p) = self.pending_ops.remove(&token.0) {
            let index = self.history.0.len() as u64;
            self.history.0.push(OpEntry {
                index,
                time: Time::virtual_nanos(now_nanos(self.now)),
                kind,
                process: p.process,
                f: p.f,
                value,
            });
        }
    }

    /// The `:f` recorded for a still-open op (so a completion can reuse it).
    pub(crate) fn pending_f(&self, token: OpToken) -> Option<Function> {
        self.pending_ops.get(&token.0).map(|p| p.f.clone())
    }

    /// How many ops for `process` have been *closed* (recorded a terminal
    /// `Ok`/`Fail`/`Info` entry). Used to gate per-process issue: a process is
    /// free once its closed count catches up to its scheduled count.
    pub(crate) fn closed_for(&self, process: ProcessId) -> usize {
        self.history
            .0
            .iter()
            .filter(|e| {
                e.process == process && matches!(e.kind, OpKind::Ok | OpKind::Fail | OpKind::Info)
            })
            .count()
    }

    fn next_seq(&mut self) -> u64 {
        let s = self.seq;
        self.seq += 1;
        s
    }

    fn enqueue(&mut self, at: Duration, kind: EventKind<N::Msg, N::Timer>) {
        let seq = self.next_seq();
        self.queue.push(Reverse(Scheduled { at, seq, kind }));
    }

    /// Whether `a` and `b` can currently exchange messages.
    fn connected(&self, a: NodeId, b: NodeId) -> bool {
        if self.crashed.contains(&a) || self.crashed.contains(&b) {
            return false;
        }
        if self.partitions.is_empty() {
            return true;
        }
        // Connected iff both fall in the same partition group.
        self.partitions
            .iter()
            .any(|g| g.contains(&a) && g.contains(&b))
    }

    /// Sample a delivery delay for the active network model, honoring per-link
    /// FIFO when `ordered`.
    fn schedule_delivery(&mut self, from: NodeId, to: NodeId, msg: N::Msg)
    where
        N::Msg: Clone,
    {
        if !self.connected(from, to) {
            return; // dropped by partition/crash
        }
        if self.rng.chance(self.model.loss.value()) {
            return; // dropped by loss
        }
        let base = self.sample_delay();
        let mut at = self.now + base;
        if self.model.ordered {
            let key = (from.0, to.0);
            let floor = self.last_link.get(&key).copied().unwrap_or(Duration::ZERO);
            if at < floor {
                at = floor;
            }
            self.last_link.insert(key, at);
        }
        let dup = self.rng.chance(self.model.duplicate.value());
        self.enqueue(
            at,
            EventKind::Deliver {
                from,
                to,
                msg: msg.clone(),
            },
        );
        if dup {
            self.enqueue(at, EventKind::Deliver { from, to, msg });
        }
    }

    fn sample_delay(&mut self) -> Duration {
        let lo = self.model.delay_ms.start;
        let hi = self.model.delay_ms.end;
        Duration::from_millis(self.rng.range_u64(lo, hi.max(lo)))
    }

    // --- timer bookkeeping ---

    fn timer_key(node: NodeId, timer: &N::Timer) -> (u64, TimerKey) {
        (node.0, format!("{timer:?}"))
    }

    fn arm_timer(&mut self, node: NodeId, timer: N::Timer, after: Duration) {
        let key = Self::timer_key(node, &timer);
        let gen = self.timer_gen.entry(key).or_insert(0);
        *gen += 1;
        let gen = *gen;
        self.enqueue(self.now + after, EventKind::Timer { node, timer, gen });
    }

    fn cancel_timer(&mut self, node: NodeId, timer: &N::Timer) {
        let key = Self::timer_key(node, timer);
        // Bump the generation so any already-queued firing is treated as stale.
        *self.timer_gen.entry(key).or_insert(0) += 1;
    }

    fn timer_is_current(&self, node: NodeId, timer: &N::Timer, gen: u64) -> bool {
        let key = Self::timer_key(node, timer);
        self.timer_gen.get(&key).copied().unwrap_or(0) == gen
    }

    /// Other node ids (for broadcast).
    pub fn peers(&self, me: NodeId) -> Vec<NodeId> {
        (0..self.node_count as u64)
            .map(NodeId)
            .filter(|&n| n != me)
            .collect()
    }

    // --- fault application ---

    fn apply_fault(&mut self, action: FaultAction) -> Option<NodeId> {
        match action {
            FaultAction::Partition(a, b) => {
                self.partitions = vec![a, b];
                None
            }
            FaultAction::HealAll => {
                self.partitions.clear();
                self.events.push(RunEvent::NetworkHealed(self.now));
                None
            }
            FaultAction::Crash(n) => {
                if !self.crashed.contains(&n) {
                    self.crashed.push(n);
                }
                // A crashed node can no longer complete the ops it was servicing;
                // force-close them as indeterminate (`Info`) at the crash time.
                let victims: Vec<u64> = self
                    .pending_ops
                    .iter()
                    .filter(|(_, p)| p.process == ProcessId(n.0))
                    .map(|(k, _)| *k)
                    .collect();
                for k in victims {
                    self.close_op(OpToken(k), OpKind::Info, Value::Nil);
                }
                None
            }
            FaultAction::Restart(n) => {
                self.crashed.retain(|&c| c != n);
                self.events.push(RunEvent::NodeRejoined(self.now, n));
                Some(n) // caller re-runs on_start
            }
        }
    }
}

// The public driver lives in `lib.rs::run_deterministic`, which owns the node
// slice and pumps this queue. The bookkeeping methods used there:
impl<N: Node> SchedCore<N>
where
    N::Timer: std::fmt::Debug,
    N::Msg: Clone,
{
    /// Expose the controlled effect operations to the `HarnessCtx`.
    pub(crate) fn ctx_send(&mut self, from: NodeId, to: NodeId, msg: N::Msg) {
        self.schedule_delivery(from, to, msg);
    }
    pub(crate) fn ctx_set_timer(&mut self, node: NodeId, timer: N::Timer, after: Duration) {
        self.arm_timer(node, timer, after);
    }
    pub(crate) fn ctx_cancel_timer(&mut self, node: NodeId, timer: &N::Timer) {
        self.cancel_timer(node, timer);
    }

    /// Schedule a fault action on the timeline.
    pub fn schedule_fault(&mut self, at: Duration, action: FaultAction) {
        self.enqueue(at, EventKind::Fault { action });
    }

    /// Schedule a client op marker on the timeline.
    pub fn schedule_client_op(&mut self, at: Duration, process: NodeId, index: usize) {
        self.enqueue(at, EventKind::ClientOp { process, index });
    }

    /// Pop the next event, advancing the virtual clock. Returns `None` when the
    /// queue is drained. Stale (cancelled) timers are skipped here.
    pub(crate) fn pop(&mut self) -> Option<PoppedEvent<N::Msg, N::Timer>> {
        while let Some(Reverse(ev)) = self.queue.pop() {
            if ev.at >= self.horizon {
                // Past the run horizon; advance the clock to the horizon and stop.
                self.now = self.horizon;
                return None;
            }
            self.now = ev.at;
            match ev.kind {
                EventKind::Timer { node, timer, gen } => {
                    if self.timer_is_current(node, &timer, gen) && !self.crashed.contains(&node) {
                        return Some(PoppedEvent::Timer { node, timer });
                    }
                    // else: stale/cancelled or crashed — skip.
                }
                EventKind::Deliver { from, to, msg } => {
                    if !self.crashed.contains(&to) {
                        return Some(PoppedEvent::Deliver { from, to, msg });
                    }
                }
                EventKind::ClientOp { process, index } => {
                    return Some(PoppedEvent::ClientOp { process, index })
                }
                EventKind::Fault { action } => {
                    let rejoined = self.apply_fault(action);
                    return Some(PoppedEvent::Fault { rejoined });
                }
            }
        }
        None
    }

    /// Force-close every still-open op as indeterminate (`Info`) at the current
    /// time. Called once after the main loop ends (at the horizon) so every
    /// `Invoke` has a terminal entry and the history is well-formed. Drains in
    /// token order (the `BTreeMap`) for determinism.
    pub(crate) fn force_close_pending(&mut self) {
        let leftover: Vec<u64> = self.pending_ops.keys().copied().collect();
        for k in leftover {
            self.close_op(OpToken(k), OpKind::Info, Value::Nil);
        }
    }
}

/// Convert a virtual `Duration` to nanoseconds, saturating at `i64::MAX`.
fn now_nanos(now: Duration) -> i64 {
    now.as_nanos().min(i64::MAX as u128) as i64
}

/// A resolved event handed back from [`SchedCore::pop`] for the driver to dispatch.
pub(crate) enum PoppedEvent<M, T> {
    Deliver { from: NodeId, to: NodeId, msg: M },
    Timer { node: NodeId, timer: T },
    ClientOp { process: NodeId, index: usize },
    Fault { rejoined: Option<NodeId> },
}
