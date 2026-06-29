//! The backend-neutral [`TestPlan`] and the `Simulation::plan() … finish()`
//! builder shape (architecture.md §5, §6.5).
//!
//! Tier 1 fixes the *surface* — the builder verbs, `run`/`try_run`/`replay`, and
//! the report/failure types. Execution itself belongs to an [`Executor`], which
//! lives in a later-tier crate; with no executor available, `run`/`try_run`
//! return [`RunFailure::NoExecutor`] rather than panicking.

use std::marker::PhantomData;

use proptest::strategy::BoxedStrategy;

use crate::backend::BackendError;
use crate::faults::Faults;
use crate::node::{ClientCodec, Node};
use crate::oracle::Verdict;
use crate::property_impl::Property;
use crate::scenario::Scenario;
use crate::seed::Seed;
use crate::transport::NetworkModel;

/// Entry point for authoring a test. `Simulation::plan()` opens the builder.
pub struct Simulation;

impl Simulation {
    /// Open a new plan builder.
    pub fn plan<N: Node>() -> PlanBuilder<N> {
        PlanBuilder::default()
    }
}

/// How the nodes are defined — the one step that differs between Style A and B
/// (architecture.md §6.5). Held opaquely in Tier 1; the executor crate provides
/// the concrete spawner.
pub enum NodeDef<N> {
    /// Style A: a `Node` state machine. The marker carries the type only.
    StateMachine(PhantomData<fn() -> N>),
    /// Style B: real handlers spawned per endpoint. Tier 1 records that the
    /// style was chosen; the spawner closure is owned by the executor crate.
    SpawnEach,
}

/// The mutable builder. Everything between `.transport(..)` and `.seeds(..)` is
/// identical across both integration styles and all backends.
pub struct PlanBuilder<N> {
    nodes: usize,
    network: Option<NetworkModel>,
    node_def: Option<NodeDef<N>>,
    workload: Option<BoxedStrategy<crate::scenario::FrozenOp>>,
    client: Option<Box<dyn ClientCodec<N>>>,
    faults: Option<Faults>,
    properties: Vec<Property<N>>,
    seeds: usize,
    replay: Option<Scenario>,
}

impl<N> Default for PlanBuilder<N> {
    fn default() -> Self {
        PlanBuilder {
            nodes: 0,
            network: None,
            node_def: None,
            workload: None,
            client: None,
            faults: None,
            properties: Vec::new(),
            seeds: 0,
            replay: None,
        }
    }
}

impl<N: Node> PlanBuilder<N> {
    /// Fixed cluster size.
    pub fn nodes(mut self, n: usize) -> Self {
        self.nodes = n;
        self
    }

    /// Network semantics (see [`InMemory`](crate::InMemory)).
    pub fn transport(mut self, model: NetworkModel) -> Self {
        self.network = Some(model);
        self
    }

    /// Style A node definition: a `Node` state machine (greenfield, total
    /// determinism).
    pub fn state_machine(mut self) -> Self {
        self.node_def = Some(NodeDef::StateMachine(PhantomData));
        self
    }

    /// Style B node definition: real handlers over a `Transport` (the iroh path).
    ///
    /// The spawner closure's concrete type is owned by the executor crate; Tier 1
    /// records only that Style B was selected.
    pub fn spawn_each(mut self) -> Self {
        self.node_def = Some(NodeDef::SpawnEach);
        self
    }

    /// The workload: a `proptest` strategy generating concrete client ops.
    pub fn workload(mut self, strategy: BoxedStrategy<crate::scenario::FrozenOp>) -> Self {
        self.workload = Some(strategy);
        self
    }

    /// The client codec that decodes generated op `Value`s into this node's typed
    /// `Op` (and encodes responses back). Required for a workload to actually
    /// drive nodes; without it, generated ops are recorded as `Fail`.
    pub fn client(mut self, codec: impl ClientCodec<N> + 'static) -> Self {
        self.client = Some(Box::new(codec));
        self
    }

    /// The fault schedule.
    pub fn faults(mut self, faults: Faults) -> Self {
        self.faults = Some(faults);
        self
    }

    /// The properties to check. Adds to any already attached.
    pub fn check(mut self, properties: impl IntoIterator<Item = Property<N>>) -> Self {
        self.properties.extend(properties);
        self
    }

    /// How many randomized executions to run.
    pub fn seeds(mut self, n: usize) -> Self {
        self.seeds = n;
        self
    }

    /// Finalize into a [`TestPlan`].
    pub fn finish(self) -> TestPlan<N> {
        TestPlan {
            data: PlanData {
                nodes: self.nodes,
                network: self.network,
                faults: self.faults,
                seeds: self.seeds,
                replay: self.replay,
            },
            node_def: self.node_def,
            workload: self.workload,
            client: self.client,
            properties: self.properties,
        }
    }
}

/// The backend-neutral, serializable description an executor consumes.
///
/// Holds the parts that do not carry the node type `N`; the typed parts
/// (`node_def`, `workload`, `properties`) live on [`TestPlan`] alongside it.
#[derive(Debug, Default)]
pub struct PlanData {
    /// Cluster size.
    pub nodes: usize,
    /// Network model.
    pub network: Option<NetworkModel>,
    /// Fault schedule.
    pub faults: Option<Faults>,
    /// Seed count.
    pub seeds: usize,
    /// A pinned scenario to replay instead of generating, if any.
    pub replay: Option<Scenario>,
}

/// The owned parts of a [`TestPlan`], as returned by [`TestPlan::into_parts`]:
/// the backend-neutral data, the node definition, the workload strategy, the
/// client codec, and the properties.
pub type PlanParts<N> = (
    PlanData,
    Option<NodeDef<N>>,
    Option<BoxedStrategy<crate::scenario::FrozenOp>>,
    Option<Box<dyn ClientCodec<N>>>,
    Vec<Property<N>>,
);

/// A finalized, backend-neutral test. Only `.run(backend)` changes between the
/// fast inner loop, the rigorous check, and the cluster run.
pub struct TestPlan<N> {
    data: PlanData,
    node_def: Option<NodeDef<N>>,
    workload: Option<BoxedStrategy<crate::scenario::FrozenOp>>,
    client: Option<Box<dyn ClientCodec<N>>>,
    properties: Vec<Property<N>>,
}

impl<N: Node> TestPlan<N> {
    /// The backend-neutral plan data.
    pub fn data(&self) -> &PlanData {
        &self.data
    }

    /// The attached properties.
    pub fn properties(&self) -> &[Property<N>] {
        &self.properties
    }

    /// How the nodes are defined (Style A vs Style B). Read by the executor crate.
    pub fn node_def(&self) -> Option<&NodeDef<N>> {
        self.node_def.as_ref()
    }

    /// The workload strategy, if any. Read by the executor crate.
    pub fn workload(&self) -> Option<&BoxedStrategy<crate::scenario::FrozenOp>> {
        self.workload.as_ref()
    }

    /// The client codec, if any. Read by the executor crate.
    pub fn client_codec(&self) -> Option<&dyn ClientCodec<N>> {
        self.client.as_deref()
    }

    /// Consume the plan into its parts. The executor crate needs ownership of the
    /// workload strategy to re-run it during shrinking.
    pub fn into_parts(self) -> PlanParts<N> {
        (
            self.data,
            self.node_def,
            self.workload,
            self.client,
            self.properties,
        )
    }

    /// Pin the workload + faults to a frozen [`Scenario`], so a failure found on
    /// one backend reproduces on another (architecture.md §14.1).
    pub fn replay(mut self, scenario: Scenario) -> Self {
        self.data.replay = Some(scenario);
        self
    }

    // NOTE: `run`/`try_run` are deliberately NOT inherent methods. Tier-1 core
    // ships no executor, and an inherent method would *shadow* the facade's
    // `Run` extension trait (inherent methods win over trait methods in Rust),
    // making `plan.run(..)` silently dispatch to a no-op. The real `run`/`try_run`
    // live in the `propsim` facade's `Run` trait, which has the engine in scope.
}

/// A verdict tagged with the name of the property or oracle that produced it, so
/// a passing report still identifies what was checked.
#[derive(Clone, Debug, PartialEq)]
pub struct NamedVerdict {
    /// The property or oracle name.
    pub name: String,
    /// Its verdict.
    pub verdict: Verdict,
}

impl NamedVerdict {
    /// Pair a property/oracle name with its verdict.
    pub fn new(name: impl Into<String>, verdict: Verdict) -> Self {
        NamedVerdict {
            name: name.into(),
            verdict,
        }
    }
}

/// A passing run's report.
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// Per-property and per-oracle verdicts, each tagged with its source name.
    pub verdicts: Vec<NamedVerdict>,
    /// How many seeds were executed.
    pub seeds_run: usize,
}

/// Why a run did not produce a passing [`Report`].
#[derive(Debug)]
pub enum RunFailure {
    /// Capability negotiation rejected the backend.
    Negotiation(BackendError),
    /// A property/oracle was violated; carries the shrunk reproduction artifact.
    Violation {
        /// The name of the violated property/oracle.
        name: String,
        /// The seed that reproduces it on the deterministic executor.
        seed: Seed,
        /// The portable, shrunk scenario for cross-backend reproduction.
        scenario: Scenario,
    },
    /// No executor was available to run the plan (Tier-1 state).
    NoExecutor,
}

impl RunFailure {
    /// The shrunk scenario, when the failure carries one.
    pub fn scenario(&self) -> Option<&Scenario> {
        match self {
            RunFailure::Violation { scenario, .. } => Some(scenario),
            _ => None,
        }
    }
}

impl std::fmt::Display for RunFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunFailure::Negotiation(e) => write!(f, "{e}"),
            RunFailure::Violation { name, seed, .. } => {
                write!(f, "FAILED property `{name}`  (re-run: PROPSIM_SEED={seed})")
            }
            RunFailure::NoExecutor => write!(
                f,
                "no executor available: this build ships only the propsim-core \
                 contract (Tier 1). Add the deterministic executor (propsim-sim) \
                 to run plans."
            ),
        }
    }
}

impl std::error::Error for RunFailure {}
