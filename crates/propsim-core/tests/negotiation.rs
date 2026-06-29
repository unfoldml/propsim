//! Capability negotiation: a white-box oracle must be rejected on a black-box
//! executor, fail-fast and by name (architecture.md §9.2).

use propsim_core::history::History;
use propsim_core::{
    validate, Artifacts, Executor, Needs, Oracle, PlanData, Produces, RunOutput, Seed, Verdict,
};

/// A black-box executor (no world snapshots), like the Jepsen executor.
struct BlackBoxExecutor;
impl Executor for BlackBoxExecutor {
    fn capabilities(&self) -> Produces {
        Produces::REAL_CLUSTER
    }
    fn run(&self, _plan: &PlanData, seed: Seed) -> RunOutput {
        RunOutput {
            history: History::default(),
            world_trace: None,
            artifacts: Artifacts::new(),
            seed,
        }
    }
}

/// A white-box executor (produces world snapshots), like the deterministic sim.
struct WhiteBoxExecutor;
impl Executor for WhiteBoxExecutor {
    fn capabilities(&self) -> Produces {
        Produces::DETERMINISTIC
    }
    fn run(&self, _plan: &PlanData, seed: Seed) -> RunOutput {
        RunOutput {
            history: History::default(),
            world_trace: None,
            artifacts: Artifacts::new(),
            seed,
        }
    }
}

struct WorldOracle;
impl Oracle for WorldOracle {
    fn name(&self) -> &str {
        "at most one leader per term"
    }
    fn needs(&self) -> Needs {
        Needs::World
    }
    fn check(&self, _out: &RunOutput) -> Verdict {
        Verdict::valid()
    }
}

struct HistoryOracle;
impl Oracle for HistoryOracle {
    fn name(&self) -> &str {
        "strict-serializable"
    }
    fn needs(&self) -> Needs {
        Needs::History
    }
    fn check(&self, _out: &RunOutput) -> Verdict {
        Verdict::valid()
    }
}

#[test]
fn white_box_oracle_rejected_on_black_box_executor() {
    let oracles: Vec<Box<dyn Oracle>> = vec![Box::new(WorldOracle), Box::new(HistoryOracle)];
    let err = validate(&BlackBoxExecutor, &oracles).unwrap_err();
    let msg = err.to_string();
    // Names the offending white-box oracle...
    assert!(
        msg.contains("at most one leader per term"),
        "msg was: {msg}"
    );
    // ...but not the history-based one, which is fine on a black-box executor.
    assert!(!msg.contains("strict-serializable"), "msg was: {msg}");
}

#[test]
fn history_oracle_passes_on_black_box_executor() {
    let oracles: Vec<Box<dyn Oracle>> = vec![Box::new(HistoryOracle)];
    assert!(validate(&BlackBoxExecutor, &oracles).is_ok());
}

#[test]
fn white_box_oracle_passes_on_white_box_executor() {
    let oracles: Vec<Box<dyn Oracle>> = vec![Box::new(WorldOracle), Box::new(HistoryOracle)];
    assert!(validate(&WhiteBoxExecutor, &oracles).is_ok());
}

#[test]
fn backend_validate_matches_free_function() {
    use propsim_core::Backend;
    let backend = Backend::custom(BlackBoxExecutor)
        .oracle(WorldOracle)
        .build();
    assert!(backend.validate().is_err());
}
