//! The Style-A [`Node`] state-machine seam (architecture.md §6.2, §6.3).
//!
//! A `Node` is a small synchronous state machine where *every* side effect goes
//! through a harness-owned context ([`Ctx`]). Because there is no other way to
//! observe time or randomness, Style-A runs are perfectly deterministic.
//!
//! Tier 1 defines the trait and the `Ctx` effect surface; the executor that
//! owns the clock, RNG, and message queue and drives these callbacks lives in
//! the simulator crate. `Ctx` is a trait so the concrete harness type can
//! implement it without Tier 1 depending on the engine.

use std::time::Duration;

use propsim_history::{Function, Value};

/// A node's identity within the cluster.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NodeId(pub u64);

/// An opaque, harness-issued identity for one in-flight client operation.
///
/// A node that cannot complete an op synchronously stashes the token (e.g. in a
/// per-key pending map) and hands it back to [`Ctx::complete_op`] when the op
/// resolves. The harness records the `Invoke` at request time and the completion
/// at the (later) virtual time of that `complete_op` call — which is what gives
/// the recorded history real, overlapping operation spans.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct OpToken(pub u64);

/// What [`Node::on_client_op`] decided about a freshly-arrived client op.
pub enum OpOutcome<R> {
    /// Completed synchronously; the harness records `Ok(R)` at the current
    /// (invoke) virtual time. Correct only for genuinely node-local, instantaneous
    /// ops (e.g. a read served from local state).
    Done(R),
    /// Not yet resolved. The harness keeps the `Invoke` open; the node MUST later
    /// call [`Ctx::complete_op`] with the same token (or the op is force-closed as
    /// `Info` when the node crashes or the run horizon is reached).
    Pending,
}

/// How a pending op finishes, passed to [`Ctx::complete_op`].
pub enum Completion<R> {
    /// Succeeded with response `R` → recorded as a Jepsen `Ok`.
    Ok(R),
    /// Definitely did not take effect → recorded as `Fail` (excluded from the
    /// linearization order).
    Fail,
    /// Indeterminate (may or may not have taken effect) → recorded as `Info`.
    Info,
}

/// A Style-A protocol node: a synchronous state machine driven by the harness.
///
/// All effects are routed through [`Ctx`], so the only observable nondeterminism
/// is what the harness owns.
pub trait Node: Sized {
    /// The wire message type exchanged between nodes.
    type Msg;
    /// The timer tag type this node schedules.
    type Timer;
    /// The node's typed client-operation type, decoded from a workload `Value` by
    /// a [`ClientCodec`]. Use `()` if the node takes no client ops.
    type Op;
    /// The typed response a completed client op yields, encoded back to a `Value`
    /// for the recorded `Ok` entry. Use `()` if the node takes no client ops.
    type Response;

    /// Called once when the node starts. Typically arms initial timers.
    fn on_start(&mut self, cx: &mut dyn Ctx<Self>);

    /// Called on each inbound message.
    fn on_msg(&mut self, from: NodeId, msg: Self::Msg, cx: &mut dyn Ctx<Self>);

    /// Called when a previously-set timer fires.
    fn on_timer(&mut self, timer: Self::Timer, cx: &mut dyn Ctx<Self>);

    /// A generated client op arrived at this node. `token` identifies this op for
    /// a later [`Ctx::complete_op`].
    ///
    /// The default leaves the op pending forever (force-closed as `Info` at the
    /// horizon), so nodes that take no client workload compile unchanged.
    fn on_client_op(
        &mut self,
        _op: Self::Op,
        _token: OpToken,
        _cx: &mut dyn Ctx<Self>,
    ) -> OpOutcome<Self::Response> {
        OpOutcome::Pending
    }
}

/// Bridges the type-erased workload [`Value`] to a node's typed `Op`/`Response`.
///
/// The harness cannot know how to turn a generic op payload into a node's op, so
/// the user supplies this. The canonical idiom is one struct that implements both
/// `ClientCodec<N>` (used by the engine at invoke time) and
/// `SequentialModel` (used by the linearizability oracle at check time), so the
/// `:f` function names and value encodings are written once.
pub trait ClientCodec<N: Node> {
    /// Map a workload op value to its Jepsen `:f` and the node's typed op. `None`
    /// means the op is malformed — the harness records an immediate `Fail` and
    /// never touches a node.
    fn decode(&self, value: &Value) -> Option<(Function, N::Op)>;

    /// Encode a typed response into the `:value` of the completing `Ok` entry.
    fn encode_response(&self, op_f: &Function, resp: &N::Response) -> Value;
}

/// The entire effect surface available to a [`Node`] callback.
///
/// `cx.send`, `cx.broadcast`, `cx.set_timer`, `cx.cancel_timer`, `cx.now()`
/// (virtual), and `cx.rng()` (seeded) are the *only* ways a node interacts with
/// the world. The harness implements this trait.
pub trait Ctx<N: Node> {
    /// Send a message to one peer.
    fn send(&mut self, to: NodeId, msg: N::Msg);

    /// Send a message to every other node.
    fn broadcast(&mut self, msg: N::Msg)
    where
        N::Msg: Clone;

    /// Arm a timer to fire after `after` of virtual time.
    fn set_timer(&mut self, timer: N::Timer, after: Duration);

    /// Cancel a previously-armed timer, if present.
    fn cancel_timer(&mut self, timer: N::Timer);

    /// The current virtual time.
    fn now(&self) -> Duration;

    /// A handle to the seeded RNG.
    fn rng(&mut self) -> &mut dyn Rng;

    /// This node's own id.
    fn me(&self) -> NodeId;

    /// Resolve a previously-[`Pending`](OpOutcome::Pending) client op. The harness
    /// records the matching terminal entry (`Ok`/`Fail`/`Info`) at the *current*
    /// virtual time — the true completion time. Calling with an unknown or
    /// already-closed token is a no-op.
    fn complete_op(&mut self, token: OpToken, completion: Completion<N::Response>);
}

/// The seeded random source exposed through [`Ctx::rng`].
///
/// A minimal surface so the stable contract does not depend on a particular RNG
/// crate; the simulator implements it over its seeded generator.
pub trait Rng {
    /// A uniformly random `u64`.
    fn next_u64(&mut self) -> u64;

    /// A duration uniformly in `range` milliseconds.
    fn duration_ms(&mut self, range: std::ops::Range<u64>) -> Duration {
        if range.is_empty() {
            return Duration::from_millis(range.start);
        }
        let span = range.end - range.start;
        let pick = range.start + (self.next_u64() % span);
        Duration::from_millis(pick)
    }
}
