//! [`Backend`] composition and capability negotiation (architecture.md §9.1, §9.2).
//!
//! A backend is one [`Executor`] plus N [`Oracle`]s. Negotiation is a **runtime**
//! check, by deliberate design (architecture.md §9.2): it runs once, before any
//! seed executes, and fails with a sentence naming the offending oracle rather
//! than an inscrutable trait-bound wall. This keeps the one-line backend swap
//! (`fast`/`rigorous`/`full` returning the same `TestPlan<N>` type) intact.

use std::fmt;

use crate::executor::Executor;
use crate::needs::Needs;
use crate::oracle::Oracle;

/// One executor composed with the oracles that check its output.
pub struct Backend {
    executor: Box<dyn Executor>,
    oracles: Vec<Box<dyn Oracle>>,
}

impl Backend {
    /// Begin composing a custom backend around `exec`. Concrete presets
    /// (`deterministic()`, `rigorous()`, `jepsen(..)`) live in the facade crate,
    /// which bundles the executors they name; Tier 1 ships only the composition
    /// machinery and the negotiation rule.
    pub fn custom(exec: impl Executor + 'static) -> BackendBuilder {
        BackendBuilder {
            executor: Box::new(exec),
            oracles: Vec::new(),
        }
    }

    /// The composed executor.
    pub fn executor(&self) -> &dyn Executor {
        self.executor.as_ref()
    }

    /// The composed oracles.
    pub fn oracles(&self) -> &[Box<dyn Oracle>] {
        &self.oracles
    }

    /// Run capability negotiation: reject any white-box oracle paired with a
    /// black-box executor. Returns the offending oracle names on failure.
    ///
    /// This is what `run`/`try_run` call first, before executing any seed.
    pub fn validate(&self) -> Result<(), BackendError> {
        validate(self.executor.as_ref(), &self.oracles)
    }
}

/// Incrementally attach oracles to a chosen executor.
pub struct BackendBuilder {
    executor: Box<dyn Executor>,
    oracles: Vec<Box<dyn Oracle>>,
}

impl BackendBuilder {
    /// Attach an oracle.
    pub fn oracle(mut self, oracle: impl Oracle + 'static) -> Self {
        self.oracles.push(Box::new(oracle));
        self
    }

    /// Attach a boxed oracle (useful when the set is built dynamically).
    pub fn boxed_oracle(mut self, oracle: Box<dyn Oracle>) -> Self {
        self.oracles.push(oracle);
        self
    }

    /// Finish composing the backend. Does **not** validate — negotiation happens
    /// at `run`/`try_run` time so the same backend value can be inspected first.
    pub fn build(self) -> Backend {
        Backend {
            executor: self.executor,
            oracles: self.oracles,
        }
    }
}

/// The capability-negotiation rule, factored out so it can be unit-tested
/// against dummy executors/oracles without constructing a full `Backend`.
///
/// A white-box property (`Needs::World`) cannot run on a black-box executor
/// (one whose `capabilities().world_snapshots == false`).
pub fn validate(exec: &dyn Executor, oracles: &[Box<dyn Oracle>]) -> Result<(), BackendError> {
    let caps = exec.capabilities();
    if caps.world_snapshots {
        return Ok(());
    }
    let offenders: Vec<String> = oracles
        .iter()
        .filter(|o| matches!(o.needs(), Needs::World))
        .map(|o| o.name().to_owned())
        .collect();
    if offenders.is_empty() {
        Ok(())
    } else {
        Err(BackendError::WhiteBoxOraclesOnBlackBoxExecutor(offenders))
    }
}

/// An error from composing or negotiating a backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendError {
    /// One or more white-box oracles were paired with a black-box executor.
    /// Carries the offending oracle names.
    WhiteBoxOraclesOnBlackBoxExecutor(Vec<String>),
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackendError::WhiteBoxOraclesOnBlackBoxExecutor(names) => {
                write!(
                    f,
                    "the following oracle(s) read internal node state, but the chosen \
                     executor is black-box (produces no world snapshots): {}. \
                     Use the deterministic executor, or target a history-based oracle.",
                    names.join(", ")
                )
            }
        }
    }
}

impl std::error::Error for BackendError {}
