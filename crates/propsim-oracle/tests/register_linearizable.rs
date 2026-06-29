//! A single register's linearizability: a valid history passes, a stale-read
//! history is rejected.

use propsim_core::history::{Function, OpEntry, OpKind, ProcessId, Value};
use propsim_core::{Artifacts, History, Oracle, RunOutput, Seed, Time};
use propsim_oracle::{linearizable, Response, SequentialModel};

/// A read/write register holding an `i64` (0 initially).
struct RegisterModel;

#[derive(Clone)]
enum RegOp {
    Read,
    Write(i64),
}

impl SequentialModel for RegisterModel {
    type State = i64;
    type Op = RegOp;

    fn init(&self) -> i64 {
        0
    }

    fn step(&self, s: &i64, op: &RegOp) -> (i64, Response) {
        match op {
            RegOp::Read => (*s, Response(Value::Int(*s))),
            RegOp::Write(v) => (*v, Response(Value::Int(*v))),
        }
    }

    fn decode(
        &self,
        f: &str,
        value_invoke: &Value,
        value_complete: &Value,
    ) -> Option<(RegOp, Response)> {
        match f {
            "write" => {
                if let Value::Int(v) = value_invoke {
                    Some((RegOp::Write(*v), Response(Value::Int(*v))))
                } else {
                    None
                }
            }
            "read" => {
                if let Value::Int(v) = value_complete {
                    Some((RegOp::Read, Response(Value::Int(*v))))
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

fn op(index: u64, kind: OpKind, process: u64, f: &str, value: i64) -> OpEntry {
    OpEntry {
        index,
        time: Time::virtual_nanos(index as i64),
        kind,
        process: ProcessId(process),
        f: Function::new(f),
        value: Value::Int(value),
    }
}

fn run_output(history: History) -> RunOutput {
    RunOutput {
        history,
        world_trace: None,
        artifacts: Artifacts::new(),
        seed: Seed(0),
    }
}

#[test]
fn valid_register_history_is_linearizable() {
    // P0 writes 1 (ok), then P1 reads 1 (ok). Sequential and obviously valid.
    let h = History::new(vec![
        op(0, OpKind::Invoke, 0, "write", 1),
        op(1, OpKind::Ok, 0, "write", 1),
        op(2, OpKind::Invoke, 1, "read", 0),
        op(3, OpKind::Ok, 1, "read", 1),
    ]);
    let oracle = linearizable(RegisterModel);
    let verdict = oracle.check(&run_output(h));
    assert!(verdict.valid, "should be linearizable: {verdict:?}");
}

#[test]
fn stale_read_is_rejected() {
    // P0 writes 1 and completes; afterward P1 reads 0 — impossible for a register
    // (the write already committed before the read began).
    let h = History::new(vec![
        op(0, OpKind::Invoke, 0, "write", 1),
        op(1, OpKind::Ok, 0, "write", 1),
        op(2, OpKind::Invoke, 1, "read", 0),
        op(3, OpKind::Ok, 1, "read", 0),
    ]);
    let oracle = linearizable(RegisterModel);
    let verdict = oracle.check(&run_output(h));
    assert!(!verdict.valid, "stale read must be non-linearizable");
    assert!(verdict
        .anomalies
        .iter()
        .any(|a| a.kind == "non-linearizable"));
}

#[test]
fn concurrent_read_can_linearize_either_way() {
    // P0's write(1) overlaps P1's read; the read may see 0 or 1. Reading 0 is
    // valid because the read can be linearized BEFORE the write.
    let h = History::new(vec![
        op(0, OpKind::Invoke, 0, "write", 1),
        op(1, OpKind::Invoke, 1, "read", 0),
        op(2, OpKind::Ok, 1, "read", 0),
        op(3, OpKind::Ok, 0, "write", 1),
    ]);
    let oracle = linearizable(RegisterModel);
    assert!(oracle.check(&run_output(h)).valid);
}
