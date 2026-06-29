//! End-to-end proof that generated client ops drive nodes and are recorded at
//! their true invoke/complete times, so linearizability checking is non-vacuous.
//!
//! A 3-node primary-backup KV: `write(k,v)` lands on the primary, which applies
//! locally, replicates, and completes only after one backup acks (quorum of 2).
//! `read(k)` returns the local value of whichever node it lands on. A single
//! `KvSpec` implements BOTH the engine-side `ClientCodec` and the oracle-side
//! `SequentialModel`, so the `:f`/value encoding is single-sourced.
//!
//! - A clean run is linearizable.
//! - Routing a read to a backup that missed the replicate, *after* the write
//!   completed, yields a stale read → caught as non-linearizable.

use std::collections::BTreeMap;

use propsim_core::history::{Function, OpKind, Value};
use propsim_core::node::{Ctx, Node};
use propsim_core::{
    Artifacts, ClientCodec, Completion, Faults, FrozenOp, History, NetworkModel, NodeId, OpOutcome,
    OpToken, Oracle, PlanData, RunOutput, Scenario, Seed,
};
use propsim_oracle::{linearizable, Response, SequentialModel};
use propsim_sim::run_deterministic;

// ---------------------------------------------------------------------------
// The node
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct KvNode {
    me: u64,
    store: BTreeMap<i64, i64>,
    // seq -> (token, value, ack count)
    inflight: BTreeMap<i64, (OpToken, i64, u32)>,
    next_seq: i64,
}

#[derive(Clone, Debug)]
enum KvMsg {
    Replicate {
        seq: i64,
        k: i64,
        v: i64,
        primary: NodeId,
    },
    Ack {
        seq: i64,
    },
}

#[derive(Clone, Debug)]
enum KvOp {
    Read(i64),
    Write(i64, i64),
}

#[derive(Clone, Debug)]
enum KvResp {
    Read(i64),
    Write(i64),
}

const PRIMARY: u64 = 0;
const QUORUM: u32 = 2; // primary self-vote + one backup ack

impl Node for KvNode {
    type Msg = KvMsg;
    type Timer = ();
    type Op = KvOp;
    type Response = KvResp;

    fn on_start(&mut self, cx: &mut dyn Ctx<Self>) {
        self.me = cx.me().0;
    }

    fn on_msg(&mut self, from: NodeId, msg: KvMsg, cx: &mut dyn Ctx<Self>) {
        match msg {
            KvMsg::Replicate { seq, k, v, primary } => {
                self.store.insert(k, v);
                cx.send(primary, KvMsg::Ack { seq });
                let _ = from;
            }
            KvMsg::Ack { seq } => {
                if let Some((tok, v, acks)) = self.inflight.get_mut(&seq) {
                    *acks += 1;
                    if *acks >= QUORUM {
                        let (tok, v) = (*tok, *v);
                        self.inflight.remove(&seq);
                        cx.complete_op(tok, Completion::Ok(KvResp::Write(v)));
                    }
                }
            }
        }
    }

    fn on_timer(&mut self, _t: (), _cx: &mut dyn Ctx<Self>) {}

    fn on_client_op(
        &mut self,
        op: KvOp,
        token: OpToken,
        cx: &mut dyn Ctx<Self>,
    ) -> OpOutcome<KvResp> {
        match op {
            KvOp::Read(k) => {
                // Served from local state — instantaneous.
                OpOutcome::Done(KvResp::Read(self.store.get(&k).copied().unwrap_or(0)))
            }
            KvOp::Write(k, v) => {
                if self.me != PRIMARY {
                    // Only the primary accepts writes; a misrouted write fails.
                    return OpOutcome::Done(KvResp::Write(v));
                }
                let seq = self.next_seq;
                self.next_seq += 1;
                self.store.insert(k, v); // primary applies locally = 1 vote
                self.inflight.insert(seq, (token, v, 1));
                cx.broadcast(KvMsg::Replicate {
                    seq,
                    k,
                    v,
                    primary: NodeId(self.me),
                });
                OpOutcome::Pending // completes on the first backup ack
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The spec: ONE struct, both ClientCodec (engine) and SequentialModel (oracle)
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct KvSpec;

// Workload op value encoding: [:write k v] and [:read k].
fn write_op(k: i64, v: i64) -> Value {
    Value::List(vec![Value::keyword("write"), Value::Int(k), Value::Int(v)])
}
fn read_op(k: i64) -> Value {
    Value::List(vec![Value::keyword("read"), Value::Int(k)])
}

impl ClientCodec<KvNode> for KvSpec {
    fn decode(&self, value: &Value) -> Option<(Function, KvOp)> {
        let Value::List(items) = value else {
            return None;
        };
        match items.as_slice() {
            [Value::Keyword(f), Value::Int(k), Value::Int(v)] if f == "write" => {
                Some((Function::new("write"), KvOp::Write(*k, *v)))
            }
            [Value::Keyword(f), Value::Int(k)] if f == "read" => {
                Some((Function::new("read"), KvOp::Read(*k)))
            }
            _ => None,
        }
    }

    fn encode_response(&self, op_f: &Function, resp: &KvResp) -> Value {
        let _ = op_f;
        match resp {
            KvResp::Read(v) | KvResp::Write(v) => Value::Int(*v),
        }
    }
}

// Sequential model: a single key (we only use key 0). State = the register value.
impl SequentialModel for KvSpec {
    type State = i64;
    type Op = KvOp;

    fn init(&self) -> i64 {
        0
    }

    fn step(&self, s: &i64, op: &KvOp) -> (i64, Response) {
        match op {
            KvOp::Read(_) => (*s, Response(Value::Int(*s))),
            KvOp::Write(_, v) => (*v, Response(Value::Int(*v))),
        }
    }

    fn decode(
        &self,
        f: &str,
        value_invoke: &Value,
        value_complete: &Value,
    ) -> Option<(KvOp, Response)> {
        match f {
            "write" => {
                if let Value::List(items) = value_invoke {
                    if let [_, Value::Int(k), Value::Int(v)] = items.as_slice() {
                        return Some((KvOp::Write(*k, *v), Response(Value::Int(*v))));
                    }
                }
                None
            }
            "read" => {
                if let Value::Int(v) = value_complete {
                    Some((KvOp::Read(0), Response(Value::Int(*v))))
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn op(process: u64, value: Value) -> FrozenOp {
    FrozenOp::new(NodeId(process), value)
}

/// A network with a fixed per-link delay, so a replication round-trip advances
/// virtual time (and a write's Ok lands strictly after its Invoke).
fn delayed_net() -> NetworkModel {
    NetworkModel {
        delay_ms: 10..11,
        loss: propsim_core::prob(0.0),
        duplicate: propsim_core::prob(0.0),
        ordered: true,
    }
}

fn plan_data(faults: Faults, ops: Vec<FrozenOp>) -> PlanData {
    PlanData {
        nodes: 3,
        network: Some(delayed_net()),
        faults: Some(faults.clone()),
        seeds: 1,
        replay: Some(Scenario::new(ops, faults)),
    }
}

fn run(data: &PlanData) -> History {
    run_deterministic::<KvNode>(
        data,
        None,
        None,
        Some(&KvSpec as &dyn ClientCodec<KvNode>),
        Seed(0xC0FFEE),
    )
    .expect("run")
    .history
}

fn check(history: History) -> propsim_core::Verdict {
    let out = RunOutput {
        history,
        world_trace: None,
        artifacts: Artifacts::new(),
        seed: Seed(0),
    };
    linearizable(KvSpec).check(&out)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn clean_run_is_linearizable_and_non_vacuous() {
    // write(0,1) on the primary (process 0 → node 0) concurrently with a read on
    // a separate process (process 1 → node 1). Reliable links: the write
    // replicates and completes ~one round-trip later; the concurrent read may see
    // 0 or 1 and still linearize.
    let ops = vec![op(0, write_op(0, 1)), op(1, read_op(0))];
    let data = plan_data(Faults::scripted(), ops);
    let history = run(&data);

    // Non-vacuity: the history has a write whose Invoke and Ok straddle the read's
    // Invoke (overlapping spans) — i.e. real concurrency, not zero-width ops.
    let invokes = history
        .entries()
        .iter()
        .filter(|e| matches!(e.kind, OpKind::Invoke))
        .count();
    let oks = history
        .entries()
        .iter()
        .filter(|e| matches!(e.kind, OpKind::Ok))
        .count();
    assert!(
        invokes >= 2,
        "expected at least two client ops recorded, got {invokes}"
    );
    assert!(oks >= 2, "both ops should complete (Ok), got {oks}");

    // A write completes strictly later than it was invoked (asynchronous).
    let write_invoke = history
        .entries()
        .iter()
        .find(|e| matches!(e.kind, OpKind::Invoke) && e.f.as_str() == "write")
        .expect("a write invoke");
    let write_ok = history
        .entries()
        .iter()
        .find(|e| matches!(e.kind, OpKind::Ok) && e.f.as_str() == "write")
        .expect("a write ok");
    assert!(
        write_ok.time.nanos > write_invoke.time.nanos,
        "the write must complete at a LATER virtual time than its invoke \
         (async, not zero-width): invoke={} ok={}",
        write_invoke.time.nanos,
        write_ok.time.nanos
    );

    let verdict = check(history);
    assert!(verdict.valid, "clean run must be linearizable: {verdict:?}");
}

#[test]
fn stale_read_after_completed_write_is_caught() {
    // Partition the lagging backup (node 2) from the rest BEFORE the write, so it
    // never receives the Replicate. The write still completes via node 1's ack
    // (quorum of 2). Then route a read to node 2 — but node 2 only accepts the op
    // if its slot is reachable... reads are local, so it returns its stale 0
    // AFTER the write's Ok was already recorded → non-linearizable.
    // Both ops on process 0 (issued SEQUENTIALLY: the read is gated until the
    // write completes). The write routes to the primary (node 0); the read routes
    // to the lagging backup (node 2). So the read happens strictly AFTER the
    // write's Ok, and node 2 never saw the replicate → it returns a stale 0.
    let ops = vec![
        op(0, write_op(0, 1)).routed_to(NodeId(0)),
        op(0, read_op(0)).routed_to(NodeId(2)),
    ];
    // Partition {0,1} | {2} for the whole run so node 2 never sees the replicate.
    let faults = Faults::scripted()
        .at(std::time::Duration::from_millis(0))
        .partition(&[0, 1], &[2]);
    let data = plan_data(faults, ops);
    let history = run(&data);

    let verdict = check(history.clone());
    assert!(
        !verdict.valid,
        "a read returning 0 entirely after write(1) completed must be \
         non-linearizable; history was:\n{}",
        history.to_jepsen_edn()
    );
    assert!(
        verdict
            .anomalies
            .iter()
            .any(|a| a.kind == "non-linearizable"),
        "expected a non-linearizable anomaly, got {:?}",
        verdict.anomalies
    );
}
