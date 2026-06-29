//! The [`Executor`] plugin trait and its normalized output (architecture.md §9.1).
//!
//! Tier 1 defines the trait; concrete executors (the deterministic simulator, the
//! Jepsen runner) live in later-tier crates.

use std::collections::BTreeMap;
use std::path::PathBuf;

use propsim_history::History;

use crate::needs::Produces;
use crate::plan::PlanData;
use crate::seed::Seed;
use crate::world::WorldTrace;

/// An executor runs a plan at a seed and yields a normalized [`RunOutput`].
///
/// This is the surface an out-of-tree executor compiles against. It is
/// deliberately small and slow to change.
pub trait Executor {
    /// What this executor can produce — checked against each oracle's
    /// [`Needs`](crate::Needs) before any seed runs.
    fn capabilities(&self) -> Produces;

    /// Execute one run of `plan` at `seed`.
    fn run(&self, plan: &PlanData, seed: Seed) -> RunOutput;
}

/// The normalized output every executor yields, regardless of backend.
#[derive(Clone, Debug)]
pub struct RunOutput {
    /// Always present, Jepsen-shaped (architecture.md §10).
    pub history: History,
    /// `Some(..)` iff [`capabilities().world_snapshots`](Produces::world_snapshots).
    pub world_trace: Option<WorldTrace>,
    /// Backend extras: `store/` paths, plots, graphviz witnesses, …
    pub artifacts: Artifacts,
    /// The seed this output was produced from.
    pub seed: Seed,
}

/// Backend-specific output files, keyed by a short logical name.
///
/// Kept generic (a name→path map) so the stable contract does not enumerate
/// every artifact kind a future backend might emit.
#[derive(Clone, Debug, Default)]
pub struct Artifacts {
    files: BTreeMap<String, PathBuf>,
}

impl Artifacts {
    /// An empty artifact set.
    pub fn new() -> Self {
        Artifacts::default()
    }

    /// Record an artifact path under `name` (e.g. `"store"`, `"latency.svg"`).
    pub fn insert(&mut self, name: impl Into<String>, path: impl Into<PathBuf>) {
        self.files.insert(name.into(), path.into());
    }

    /// Look up an artifact path by name.
    pub fn get(&self, name: &str) -> Option<&PathBuf> {
        self.files.get(name)
    }

    /// Iterate over `(name, path)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &PathBuf)> {
        self.files.iter()
    }
}
