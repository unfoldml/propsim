//! The `Run` extension trait — the real `run`/`try_run` for a [`TestPlan`].
//!
//! Lives in the facade (not Tier-1 core) so the dependency arrow stays
//! core ← sim ← facade: a bare `propsim-core` consumer (a plugin author) sees
//! only the `NoExecutor` stub, while facade users get the working engine. It is
//! re-exported in the prelude so `plan.run(backend)` resolves to it.

use propsim_core::node::Node;
use propsim_core::{Backend, BackendError, Report, RunFailure, TestPlan};

/// Run a [`TestPlan`] against a [`Backend`]. Shadows the Tier-1 inherent
/// `run`/`try_run` (which return `NoExecutor`).
pub trait Run<N: Node> {
    /// Run, panicking on violation (use in `#[test]`).
    fn run(self, backend: Backend) -> Report;
    /// Run, returning a failure as data (for the cross-backend pipeline).
    fn try_run(self, backend: Backend) -> Result<Report, RunFailure>;
}

impl<N> Run<N> for TestPlan<N>
where
    N: Node + Clone + Default,
    N::Timer: std::fmt::Debug + Clone,
    N::Msg: Clone,
{
    fn try_run(self, backend: Backend) -> Result<Report, RunFailure> {
        // Capability negotiation runs first, before any seed executes. The
        // Tier-1 `validate` checks oracles; here we extend it to white-box
        // *properties*, which are also Needs::World but live on the plan, not in
        // the oracle list. Both must be rejected on a black-box executor.
        backend.validate().map_err(RunFailure::Negotiation)?;
        if !backend.executor().capabilities().world_snapshots && !self.properties().is_empty() {
            let offenders: Vec<String> = self
                .properties()
                .iter()
                .map(|p| p.name().to_owned())
                .collect();
            return Err(RunFailure::Negotiation(
                BackendError::WhiteBoxOraclesOnBlackBoxExecutor(offenders),
            ));
        }

        if backend.executor().capabilities().world_snapshots {
            // Typed white-box path: drive the deterministic sim with N in scope.
            propsim_sim::drive(self, &backend)
        } else {
            // Erased path: black-box executor, history-based oracles only.
            let plan_len = self.properties().len();
            propsim_sim::drive_erased(self.data(), plan_len, &backend)
        }
    }

    fn run(self, backend: Backend) -> Report {
        match self.try_run(backend) {
            Ok(report) => report,
            Err(failure) => panic!("{failure}"),
        }
    }
}
