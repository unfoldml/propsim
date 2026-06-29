//! # propsim-sim
//!
//! The deterministic in-process executor (architecture.md §9.3): a
//! single-threaded, virtual-time discrete-event simulator with a seeded RNG and
//! an in-memory transport. It drives Style-A [`Node`] state machines and records
//! the [`History`] and a typed `WorldTrace<N>` that white-box properties read.
//!
//! The engine's real API is the generic free function [`run_deterministic`] (and
//! the [`drive`]/[`drive_erased`] entry points the facade's `Run` trait calls);
//! [`DeterministicExecutor`] is the type-erased [`Executor`] wrapper that lets the
//! same engine live inside a [`Backend`] for capability negotiation.

#![forbid(unsafe_code)]

mod ctx;
mod faults;
mod properties;
mod rng;
mod scheduler;

use std::collections::VecDeque;
use std::time::Duration;

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::{RngAlgorithm, TestRng, TestRunner};

use propsim_core::history::{Function, OpKind, ProcessId};
use propsim_core::node::{ClientCodec, Node, OpOutcome};
use propsim_core::{
    Artifacts, Backend, Executor, FrozenOp, History, NamedVerdict, NetworkModel, NodeDef, NodeId,
    PlanData, Produces, Property, Report, RunFailure, RunOutput, Scenario, Seed, TestPlan,
    WorldTrace,
};

pub use properties::evaluate_properties;
pub use rng::SimRng;
pub use scheduler::RunEvent;

use ctx::HarnessCtx;
use scheduler::{PoppedEvent, SchedCore};

/// The bounds the deterministic engine needs on a node type: the Tier-1 [`Node`]
/// contract plus `Clone` (to snapshot per-node state into the `WorldTrace`) and
/// `Debug` on the timer tag (for a stable timer identity) and `Clone` on the
/// message (the in-memory transport may duplicate).
pub trait SimNode: Node + Clone
where
    Self::Timer: std::fmt::Debug + Clone,
    Self::Msg: Clone,
{
}
impl<N> SimNode for N
where
    N: Node + Clone,
    N::Timer: std::fmt::Debug + Clone,
    N::Msg: Clone,
{
}

/// One seed's fully-typed result. `world_trace` stays typed here so the property
/// evaluator can read live `&World<N>`; it is erased only when crossing into a
/// [`RunOutput`] for the history-oracle pipeline.
pub struct TypedRun<N: Node> {
    /// The operation history recorded during the run.
    pub history: History,
    /// The per-frame world snapshots, kept typed for property evaluation.
    pub world_trace: WorldTrace<N>,
    /// The scheduler events in execution order.
    pub events: Vec<RunEvent>,
    /// The seed this run was executed at.
    pub seed: Seed,
}

/// Run one deterministic execution at `seed` and return the typed result.
///
/// Only Style A (`NodeDef::StateMachine`) is implemented; Style B (`SpawnEach`)
/// requires the libc interception crate and is a later phase.
pub fn run_deterministic<N>(
    data: &PlanData,
    node_def: Option<&NodeDef<N>>,
    workload: Option<&proptest::strategy::BoxedStrategy<FrozenOp>>,
    codec: Option<&dyn ClientCodec<N>>,
    seed: Seed,
) -> Result<TypedRun<N>, RunFailure>
where
    N: Node + Clone + Default,
    N::Timer: std::fmt::Debug + Clone,
    N::Msg: Clone,
{
    match node_def {
        Some(NodeDef::StateMachine(_)) | None => {}
        Some(NodeDef::SpawnEach) => {
            return Err(RunFailure::NoExecutor); // Style B not yet implemented
        }
    }

    let node_count = data.nodes.max(1);
    let model = data
        .network
        .clone()
        .unwrap_or_else(NetworkModel::ordered_reliable);

    let mut core = SchedCore::<N>::new(seed.0, model.clone(), node_count);
    core.horizon = run_horizon(data);
    // Nodes are held in Option slots so the active node can be taken out while a
    // HarnessCtx borrows the rest of the scheduler.
    let mut nodes: Vec<Option<N>> = (0..node_count).map(|_| Some(N::default())).collect();

    // Lower faults onto the timeline (scripted) or fold them into the model +
    // schedule discrete events (swarm). Swarm draws from a side RNG seeded from
    // the same seed so the main RNG stream is unperturbed by config.
    let mut model_mut = model;
    let mut fault_rng = SimRng::new(seed.0 ^ 0xF0F0_F0F0);
    faults::lower(
        data.faults.as_ref(),
        &mut model_mut,
        &mut core,
        &mut fault_rng,
        node_count,
    );
    core.model = model_mut;

    // Resolve the client op stream and group it per process for gated issue: one
    // op per process is in flight at a time, so overlap arises ACROSS processes
    // (sequential within one) — the shape linearizability needs.
    let ops = resolve_ops(data, workload, seed);
    let mut per_process: Vec<VecDeque<usize>> = vec![VecDeque::new(); node_count];
    for (i, op) in ops.iter().enumerate() {
        let p = (op.process.0 as usize) % node_count;
        per_process[p].push_back(i);
    }
    // Count of ops scheduled per process so far (gating: one in flight per process).
    let mut scheduled: Vec<usize> = vec![0; node_count];

    // Drive on_start for every node.
    let mut trace = WorldTrace::<N>::new();
    for id in 0..node_count {
        dispatch_start(&mut core, &mut nodes, NodeId(id as u64), codec);
    }
    snapshot(&mut trace, &core, &nodes);

    // Issue the first op for each process, then let the gated pump issue the rest.
    pump_client_ops(&mut core, &mut per_process, &mut scheduled, node_count);

    // Main loop.
    while let Some(ev) = core.pop() {
        match ev {
            PoppedEvent::Deliver { from, to, msg } => {
                dispatch_msg(&mut core, &mut nodes, to, from, msg, codec);
            }
            PoppedEvent::Timer { node, timer } => {
                dispatch_timer(&mut core, &mut nodes, node, timer, codec);
            }
            PoppedEvent::ClientOp { process, index } => {
                dispatch_client_op(&mut core, &mut nodes, process, &ops[index], codec);
            }
            PoppedEvent::Fault { rejoined } => {
                if let Some(n) = rejoined {
                    dispatch_start(&mut core, &mut nodes, n, codec);
                }
            }
        }
        // After every event, issue the next op for any process whose previous op
        // has closed (gated issue).
        pump_client_ops(&mut core, &mut per_process, &mut scheduled, node_count);
        snapshot(&mut trace, &core, &nodes);
    }

    // Force-close any op still open at the horizon as indeterminate (Info) so the
    // history is well-formed.
    core.force_close_pending();

    trace.history = core.history.clone();
    Ok(TypedRun {
        history: core.history,
        world_trace: trace,
        events: core.events,
        seed,
    })
}

/// For each process with no in-flight op and remaining queued ops, schedule its
/// next op a small virtual delay from now. Deterministic in process order.
///
/// "In flight" means scheduled-but-not-yet-closed: an op is in flight from the
/// moment it is scheduled (`scheduled[p]` is bumped here) until its terminal
/// history entry is recorded (`closed_for` counts those). Gating only on the
/// scheduler's *pending* set would be wrong — an op that is scheduled but whose
/// `Invoke` has not fired is not yet "pending", yet the process is still busy.
fn pump_client_ops<N>(
    core: &mut SchedCore<N>,
    per_process: &mut [VecDeque<usize>],
    scheduled: &mut [usize],
    node_count: usize,
) where
    N: Node + Clone,
    N::Timer: std::fmt::Debug,
    N::Msg: Clone,
{
    for p in 0..node_count {
        let closed = core.closed_for(ProcessId(p as u64));
        // Busy while a scheduled op for this process has not closed yet.
        if scheduled[p] > closed {
            continue;
        }
        if let Some(&idx) = per_process[p].front() {
            per_process[p].pop_front();
            scheduled[p] += 1;
            let at = core.now + Duration::from_millis(1);
            core.schedule_client_op(at, NodeId(p as u64), idx);
        }
    }
}

/// Concretize the workload op stream for this seed (or take it from a replay
/// scenario, which pins the exact ops).
fn resolve_ops(
    data: &PlanData,
    workload: Option<&proptest::strategy::BoxedStrategy<FrozenOp>>,
    seed: Seed,
) -> Vec<FrozenOp> {
    if let Some(scenario) = &data.replay {
        return scenario.ops.clone();
    }
    let Some(strategy) = workload else {
        return Vec::new();
    };
    // Generate a concrete op stream by sampling the strategy `seeds`-ish times.
    let count = data.seeds.clamp(1, 64);
    let rng = TestRng::from_seed(RngAlgorithm::ChaCha, &seed.0.to_le_bytes().repeat(4));
    let mut runner = TestRunner::new_with_rng(Default::default(), rng);
    (0..count)
        .filter_map(|_| strategy.new_tree(&mut runner).ok().map(|t| t.current()))
        .collect()
}

/// The virtual-time horizon for a run: the latest scripted fault time plus a
/// generous convergence margin, floored at 10s. Because time is compressed, a
/// large horizon costs negligible wall time but guarantees termination of
/// protocols that re-arm timers forever.
fn run_horizon(data: &PlanData) -> Duration {
    let mut latest = Duration::from_secs(5);
    let faults = data
        .faults
        .as_ref()
        .or_else(|| data.replay.as_ref().map(|s| &s.faults));
    if let Some(faults) = faults {
        if let Some(script) = faults.script() {
            for ev in script {
                if ev.at > latest {
                    latest = ev.at;
                }
            }
        }
    }
    // Add a 10s convergence margin after the last scripted event.
    (latest + Duration::from_secs(10)).max(Duration::from_secs(10))
}

fn op_function(op: &FrozenOp) -> Function {
    // If the op payload leads with a keyword (e.g. `[:write k v]`), use it as the
    // Jepsen `:f`; otherwise default to a generic "op".
    use propsim_core::history::Value;
    match &op.op {
        Value::Keyword(k) => Function::new(k.clone()),
        Value::List(items) => match items.first() {
            Some(Value::Keyword(k)) => Function::new(k.clone()),
            _ => Function::new("op"),
        },
        _ => Function::new("op"),
    }
}

// --- dispatch helpers (borrow-split: take the node out, run, put it back) ---

fn dispatch_start<N>(
    core: &mut SchedCore<N>,
    nodes: &mut [Option<N>],
    id: NodeId,
    codec: Option<&dyn ClientCodec<N>>,
) where
    N: Node + Clone,
    N::Timer: std::fmt::Debug + Clone,
    N::Msg: Clone,
{
    if let Some(mut node) = nodes[id.0 as usize].take() {
        let mut cx = HarnessCtx {
            core,
            me: id,
            codec,
        };
        node.on_start(&mut cx);
        nodes[id.0 as usize] = Some(node);
    }
}

fn dispatch_msg<N>(
    core: &mut SchedCore<N>,
    nodes: &mut [Option<N>],
    to: NodeId,
    from: NodeId,
    msg: N::Msg,
    codec: Option<&dyn ClientCodec<N>>,
) where
    N: Node + Clone,
    N::Timer: std::fmt::Debug + Clone,
    N::Msg: Clone,
{
    if let Some(mut node) = nodes[to.0 as usize].take() {
        let mut cx = HarnessCtx {
            core,
            me: to,
            codec,
        };
        node.on_msg(from, msg, &mut cx);
        nodes[to.0 as usize] = Some(node);
    }
}

fn dispatch_timer<N>(
    core: &mut SchedCore<N>,
    nodes: &mut [Option<N>],
    id: NodeId,
    timer: N::Timer,
    codec: Option<&dyn ClientCodec<N>>,
) where
    N: Node + Clone,
    N::Timer: std::fmt::Debug + Clone,
    N::Msg: Clone,
{
    if let Some(mut node) = nodes[id.0 as usize].take() {
        let mut cx = HarnessCtx {
            core,
            me: id,
            codec,
        };
        node.on_timer(timer, &mut cx);
        nodes[id.0 as usize] = Some(node);
    }
}

/// Decode a generated op, record its `Invoke`, deliver it to the target node, and
/// record an immediate `Ok` (synchronous `Done`) or leave it open (`Pending`,
/// completed later via `cx.complete_op`).
fn dispatch_client_op<N>(
    core: &mut SchedCore<N>,
    nodes: &mut [Option<N>],
    process: NodeId,
    op: &FrozenOp,
    codec: Option<&dyn ClientCodec<N>>,
) where
    N: Node + Clone,
    N::Timer: std::fmt::Debug + Clone,
    N::Msg: Clone,
{
    let node_count = nodes.len();
    // Route to the op's explicit target if given, else `process % node_count`.
    let target = op
        .route
        .map(|n| n.0 as usize % node_count)
        .unwrap_or((process.0 as usize) % node_count);
    let pid = ProcessId(process.0);

    // Decode Value -> (:f, typed op). No codec / malformed op -> Invoke + Fail.
    let decoded = codec.and_then(|c| c.decode(&op.op));
    let Some((f, typed_op)) = decoded else {
        let tok = core.open_op(pid, op_function(op), op.op.clone());
        core.close_op(tok, OpKind::Fail, propsim_core::Value::Nil);
        return;
    };

    let tok = core.open_op(pid, f.clone(), op.op.clone());

    // The target node may be crashed (slot empty) -> indeterminate.
    let Some(mut node) = nodes[target].take() else {
        core.close_op(tok, OpKind::Info, propsim_core::Value::Nil);
        return;
    };
    let outcome = {
        let mut cx = HarnessCtx {
            core,
            me: NodeId(target as u64),
            codec,
        };
        node.on_client_op(typed_op, tok, &mut cx)
    };
    nodes[target] = Some(node);

    match outcome {
        OpOutcome::Done(resp) => {
            let value = codec
                .map(|c| c.encode_response(&f, &resp))
                .unwrap_or(propsim_core::Value::Nil);
            core.close_op(tok, OpKind::Ok, value);
        }
        OpOutcome::Pending => { /* node will complete_op later */ }
    }
}

fn snapshot<N>(trace: &mut WorldTrace<N>, core: &SchedCore<N>, nodes: &[Option<N>])
where
    N: Node + Clone,
    N::Timer: std::fmt::Debug,
{
    let snap: Vec<N> = nodes.iter().filter_map(|n| n.clone()).collect();
    trace.frames.push((core.now, snap));
}

// --- facade entry points ---

/// Drive the typed white-box path: run the deterministic engine, evaluate
/// properties, run any history-based oracles on the erased output, and merge
/// both verdict streams into one [`Report`]. Called by the facade's `Run` trait.
pub fn drive<N>(plan: TestPlan<N>, backend: &Backend) -> Result<Report, RunFailure>
where
    N: Node + Clone + Default,
    N::Timer: std::fmt::Debug + Clone,
    N::Msg: Clone,
{
    let (data, node_def, workload, client, properties) = plan.into_parts();
    let codec = client.as_deref();

    let seeds = data.seeds.max(1);
    let mut verdicts: Vec<NamedVerdict> = Vec::new();
    let mut seeds_run = 0;

    for s in 0..seeds {
        let seed = effective_seed(&data, s);
        let run = run_deterministic::<N>(&data, node_def.as_ref(), workload.as_ref(), codec, seed)?;
        seeds_run += 1;

        // White-box properties (typed path).
        let prop_verdicts = evaluate_properties(&properties, &run.world_trace, &run.events);

        // History-based oracles (erased path) over this run's history.
        let run_output = RunOutput {
            history: run.history.clone(),
            world_trace: None,
            artifacts: Artifacts::new(),
            seed,
        };
        let oracle_verdicts: Vec<NamedVerdict> = backend
            .oracles()
            .iter()
            .map(|o| NamedVerdict::new(o.name(), o.check(&run_output)))
            .collect();

        // First failure aborts with a shrunk scenario.
        for nv in prop_verdicts.iter().chain(oracle_verdicts.iter()) {
            if !nv.verdict.valid {
                let scenario = shrink_scenario(
                    &data,
                    node_def.as_ref(),
                    workload.as_ref(),
                    codec,
                    &properties,
                    backend,
                    seed,
                    &nv.name,
                );
                return Err(RunFailure::Violation {
                    name: nv.name.clone(),
                    seed,
                    scenario,
                });
            }
        }
        verdicts.extend(prop_verdicts);
        verdicts.extend(oracle_verdicts);
    }

    Ok(Report {
        verdicts,
        seeds_run,
    })
}

/// Drive the erased path: a black-box executor that produces only a history.
/// Used when the chosen backend is not the deterministic sim (e.g. a future
/// Jepsen backend); white-box properties were already rejected by negotiation.
pub fn drive_erased(
    data: &PlanData,
    properties_len: usize,
    backend: &Backend,
) -> Result<Report, RunFailure> {
    let _ = properties_len; // negotiation guaranteed none are white-box here
    let seed = effective_seed(data, 0);
    let out = backend.executor().run(data, seed);
    let verdicts = backend
        .oracles()
        .iter()
        .map(|o| NamedVerdict::new(o.name(), o.check(&out)))
        .collect();
    Ok(Report {
        verdicts,
        seeds_run: 1,
    })
}

/// The seed for run index `s`, honoring the `PROPSIM_SEED` env override for `s==0`.
fn effective_seed(data: &PlanData, s: usize) -> Seed {
    if s == 0 {
        if let Ok(v) = std::env::var("PROPSIM_SEED") {
            if let Ok(seed) = v.parse::<Seed>() {
                return seed;
            }
        }
    }
    // Derive per-index seeds deterministically from the run index.
    use propsim_core::node::Rng as _;
    let _ = data;
    let mut r = SimRng::new(0x5EED_0000 ^ s as u64);
    Seed(r.next_u64())
}

/// Re-run with proptest-style shrinking to a minimal failing op stream. This
/// implementation shrinks by truncating the op stream (the cheap, deterministic
/// shrink); the resulting `Scenario` reproduces the failure on any backend.
#[allow(clippy::too_many_arguments)]
fn shrink_scenario<N>(
    data: &PlanData,
    node_def: Option<&NodeDef<N>>,
    workload: Option<&proptest::strategy::BoxedStrategy<FrozenOp>>,
    codec: Option<&dyn ClientCodec<N>>,
    properties: &[Property<N>],
    backend: &Backend,
    seed: Seed,
    failed_name: &str,
) -> Scenario
where
    N: Node + Clone + Default,
    N::Timer: std::fmt::Debug + Clone,
    N::Msg: Clone,
{
    let full = run_deterministic::<N>(data, node_def, workload, codec, seed)
        .map(|r| collect_ops(&r))
        .unwrap_or_default();

    // Try progressively shorter prefixes; keep the shortest that still fails.
    let mut best = full.clone();
    let mut len = full.len();
    while len > 0 {
        let candidate = full[..len - 1].to_vec();
        if still_fails(
            data,
            node_def,
            codec,
            properties,
            backend,
            seed,
            failed_name,
            &candidate,
        ) {
            best = candidate;
            len -= 1;
        } else {
            break;
        }
    }
    Scenario::new(
        best,
        data.faults
            .clone()
            .unwrap_or_else(propsim_core::Faults::swarm),
    )
    .from_seed(seed)
}

fn collect_ops<N: Node>(run: &TypedRun<N>) -> Vec<FrozenOp> {
    // Recover the frozen ops from the recorded Invokes.
    run.history
        .entries()
        .iter()
        .filter(|e| matches!(e.kind, propsim_core::OpKind::Invoke))
        .map(|e| FrozenOp::new(NodeId(e.process.0), e.value.clone()))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn still_fails<N>(
    data: &PlanData,
    node_def: Option<&NodeDef<N>>,
    codec: Option<&dyn ClientCodec<N>>,
    properties: &[Property<N>],
    backend: &Backend,
    seed: Seed,
    failed_name: &str,
    ops: &[FrozenOp],
) -> bool
where
    N: Node + Clone + Default,
    N::Timer: std::fmt::Debug + Clone,
    N::Msg: Clone,
{
    let pinned = data_with_ops(data, ops);
    let Ok(run) = run_deterministic::<N>(&pinned, node_def, None, codec, seed) else {
        return false;
    };
    let prop_verdicts = evaluate_properties(properties, &run.world_trace, &run.events);
    if prop_verdicts
        .iter()
        .any(|nv| nv.name == failed_name && !nv.verdict.valid)
    {
        return true;
    }
    let out = RunOutput {
        history: run.history.clone(),
        world_trace: None,
        artifacts: Artifacts::new(),
        seed,
    };
    backend
        .oracles()
        .iter()
        .any(|o| o.name() == failed_name && !o.check(&out).valid)
}

/// A copy of `data` with the op stream pinned via a replay scenario.
fn data_with_ops(data: &PlanData, ops: &[FrozenOp]) -> PlanData {
    PlanData {
        nodes: data.nodes,
        network: data.network.clone(),
        faults: data.faults.clone(),
        seeds: data.seeds,
        replay: Some(Scenario::new(
            ops.to_vec(),
            data.faults
                .clone()
                .unwrap_or_else(propsim_core::Faults::swarm),
        )),
    }
}

// --- the type-erased Executor wrapper ---

/// The deterministic executor as a type-erased [`Executor`], so it can live in a
/// [`Backend`] and answer capability negotiation. Its erased `run` produces a
/// history-only output; the white-box typed path is reached via [`drive`].
#[derive(Clone, Default)]
pub struct DeterministicExecutor;

impl DeterministicExecutor {
    /// A fresh deterministic executor.
    pub fn new() -> Self {
        DeterministicExecutor
    }
}

impl Executor for DeterministicExecutor {
    fn capabilities(&self) -> Produces {
        Produces::DETERMINISTIC
    }

    fn run(&self, _plan: &PlanData, seed: Seed) -> RunOutput {
        // History-only erased run: with no node type in scope we cannot drive a
        // Style-A state machine, so this yields an empty history. The real
        // white-box run happens in `drive` with `N` in scope.
        RunOutput {
            history: History::default(),
            world_trace: None,
            artifacts: Artifacts::new(),
            seed,
        }
    }
}
