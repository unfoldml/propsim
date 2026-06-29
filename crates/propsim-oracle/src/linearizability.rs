//! A Porcupine-style linearizability checker (Wing & Gong / Lowe), run over a
//! recorded [`History`] against a user [`SequentialModel`] (architecture.md §8.1).
//!
//! The search linearizes one *minimal* (earliest-finishing) currently-callable
//! operation at a time, applies it to the model, recurses, and backtracks on a
//! response mismatch or dead end. A visited set of `(linearized-bitset, state)`
//! prunes revisited search nodes — the speedup that beats the Knossos approach.

use std::collections::HashSet;

use propsim_core::history::{OpEntry, OpKind, Value};
use propsim_core::{Anomaly, Needs, Oracle, RunOutput, Verdict, Witness};

use crate::model::{Response, SequentialModel};

/// A linearizability oracle for `M`'s semantics, in real-time mode.
pub fn linearizable<M: SequentialModel>(model: M) -> LinearizabilityOracle<M> {
    LinearizabilityOracle {
        model,
        real_time: true,
        name: "linearizable".to_string(),
    }
}

/// A sequential-consistency oracle (drops the real-time ordering constraint).
pub fn sequentially_consistent<M: SequentialModel>(model: M) -> LinearizabilityOracle<M> {
    LinearizabilityOracle {
        model,
        real_time: false,
        name: "sequentially-consistent".to_string(),
    }
}

/// The configured checker.
pub struct LinearizabilityOracle<M: SequentialModel> {
    model: M,
    real_time: bool,
    name: String,
}

/// A completed operation span: an invoke paired with its completion.
struct Span<Op> {
    op: Op,
    response: Response,
    /// Invoke order index in the history (the real-time "start").
    start: usize,
    /// Completion order index in the history (the real-time "finish").
    finish: usize,
}

impl<M: SequentialModel> Oracle for LinearizabilityOracle<M> {
    fn name(&self) -> &str {
        &self.name
    }

    fn needs(&self) -> Needs {
        Needs::History
    }

    fn check(&self, out: &RunOutput) -> Verdict {
        let spans = match self.parse_spans(out.history.entries()) {
            Ok(s) => s,
            Err(msg) => {
                return Verdict::invalid(vec![Anomaly::with_detail("history-decode-failed", msg)])
            }
        };
        if self.search(&spans) {
            Verdict::valid()
        } else {
            Verdict::invalid(vec![Anomaly::new(if self.real_time {
                "non-linearizable"
            } else {
                "not-sequentially-consistent"
            })])
            .with_witness(Witness::Text(format!(
                "no valid {} ordering of {} operations",
                self.name,
                spans.len()
            )))
        }
    }
}

impl<M: SequentialModel> LinearizabilityOracle<M> {
    /// Pair Invoke→complete entries per process into operation spans.
    fn parse_spans(&self, entries: &[OpEntry]) -> Result<Vec<Span<M::Op>>, String> {
        let mut spans = Vec::new();
        // Track the open invoke per process.
        let mut open: Vec<(usize, &OpEntry)> = Vec::new(); // (history-pos, invoke)
        for (pos, e) in entries.iter().enumerate() {
            match e.kind {
                OpKind::Invoke => open.push((pos, e)),
                OpKind::Ok | OpKind::Fail | OpKind::Info => {
                    // Match the most recent open invoke for this process.
                    if let Some(idx) = open.iter().rposition(|(_, inv)| inv.process == e.process) {
                        let (start, inv) = open.remove(idx);
                        if matches!(e.kind, OpKind::Fail) {
                            continue; // a definite no-op; excluded from the order
                        }
                        if let Some((op, resp)) =
                            self.model.decode(e.f.as_str(), &inv.value, &e.value)
                        {
                            spans.push(Span {
                                op,
                                response: resp,
                                start,
                                finish: pos,
                            });
                        }
                        // Info (indeterminate) ops are dropped in this Tier-2
                        // pass: a more complete checker would treat them as
                        // optionally-present.
                    }
                }
            }
        }
        let _ = Value::Nil;
        Ok(spans)
    }

    /// Depth-first linearization search with a visited-set prune.
    fn search(&self, spans: &[Span<M::Op>]) -> bool {
        let n = spans.len();
        if n == 0 {
            return true;
        }
        let mut visited: HashSet<(Vec<bool>, M::State)> = HashSet::new();
        let done = vec![false; n];
        self.recurse(spans, self.model.init(), done, n, &mut visited)
    }

    fn recurse(
        &self,
        spans: &[Span<M::Op>],
        state: M::State,
        done: Vec<bool>,
        remaining: usize,
        visited: &mut HashSet<(Vec<bool>, M::State)>,
    ) -> bool {
        if remaining == 0 {
            return true;
        }
        if !visited.insert((done.clone(), state.clone())) {
            return false; // already explored this node — prune
        }

        // The earliest finish time among not-yet-linearized ops: only an op
        // whose start precedes that minimal finish is callable now (real-time).
        let min_finish = spans
            .iter()
            .enumerate()
            .filter(|(i, _)| !done[*i])
            .map(|(_, s)| s.finish)
            .min()
            .unwrap_or(usize::MAX);

        for (i, span) in spans.iter().enumerate() {
            if done[i] {
                continue;
            }
            if self.real_time && span.start > min_finish {
                // This op started after another op already finished, so it
                // cannot be linearized before that op — not callable yet.
                continue;
            }
            let (next_state, resp) = self.model.step(&state, &span.op);
            if resp != span.response {
                continue; // response mismatch — this op can't go here
            }
            let mut next_done = done.clone();
            next_done[i] = true;
            if self.recurse(spans, next_state, next_done, remaining - 1, visited) {
                return true;
            }
        }
        false
    }
}
