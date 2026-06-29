//! Properties: the white-box vocabulary over a [`World`] (architecture.md §7.2).
//!
//! Vocabulary borrowed from `stateright`'s `Always` / `Sometimes` / `Eventually`,
//! extended with a quantitative deadline because we have a real virtual clock.
//!
//! Every property is state-based and therefore white-box: it reads internal node
//! state and so declares [`Needs::World`](crate::Needs). History-based checks
//! (linearizability, transactional isolation) are [`Oracle`](crate::Oracle)s, not
//! properties — that is the seam that keeps the portable checks portable.

use std::time::Duration;

use crate::needs::Needs;
use crate::world::World;

/// A boxed property predicate: takes a world snapshot, returns whether the claim
/// holds in it. Boxed so non-capturing `fn` predicates and capturing closures
/// (the `impl Fn` builder form, architecture.md §7.2) share one type.
type Predicate<N> = Box<dyn Fn(&World<'_, N>) -> bool + Send + Sync>;

/// A named correctness claim evaluated against a [`World`].
///
/// `N` is the node state type.
pub struct Property<N> {
    name: String,
    kind: PropertyKind,
    predicate: Predicate<N>,
    /// For a bounded-liveness property, the event after which the deadline clock
    /// starts. `None` means "from the start of the run".
    after: Option<Event>,
}

/// The temporal flavor of a [`Property`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PropertyKind {
    /// Must hold in every reachable state (safety / invariant).
    Always,
    /// Must hold in at least one state (reachability / non-triviality).
    Sometimes,
    /// Must become true within `deadline` of the (optional) start event
    /// (bounded-time liveness).
    EventuallyWithin {
        /// How long after the start event the property has to hold.
        deadline: Duration,
    },
}

/// A run event that a bounded-liveness deadline can be anchored to.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Event {
    /// All injected partitions have healed.
    NetworkHealed,
    /// A crashed node has rejoined.
    NodeRejoined,
    /// A named, protocol-specific marker.
    Custom(String),
}

impl<N> Property<N> {
    /// The property's name (used in reproduction blocks and negotiation).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The temporal flavor.
    pub fn kind(&self) -> &PropertyKind {
        &self.kind
    }

    /// Every property here is white-box: it reads internal node state.
    pub fn needs(&self) -> Needs {
        Needs::World
    }

    /// The event the deadline is anchored to, if any.
    pub fn after_event(&self) -> Option<&Event> {
        self.after.as_ref()
    }

    /// Evaluate the predicate against a world snapshot.
    pub fn eval(&self, world: &World<'_, N>) -> bool {
        (self.predicate)(world)
    }

    /// Anchor a bounded-liveness deadline to start at `event`.
    pub fn after(mut self, event: Event) -> Self {
        self.after = Some(event);
        self
    }
}

/// The three property constructors, re-exported as the `property` module at the
/// crate root so callers write `property::always(..)`.
pub mod constructors {
    use super::*;

    /// SAFETY / INVARIANT — must hold in every reachable state.
    pub fn always<N>(
        name: impl Into<String>,
        predicate: impl Fn(&World<'_, N>) -> bool + Send + Sync + 'static,
    ) -> Property<N> {
        Property {
            name: name.into(),
            kind: PropertyKind::Always,
            predicate: Box::new(predicate),
            after: None,
        }
    }

    /// REACHABILITY / NON-TRIVIALITY — guards against a vacuous spec.
    pub fn sometimes<N>(
        name: impl Into<String>,
        predicate: impl Fn(&World<'_, N>) -> bool + Send + Sync + 'static,
    ) -> Property<N> {
        Property {
            name: name.into(),
            kind: PropertyKind::Sometimes,
            predicate: Box::new(predicate),
            after: None,
        }
    }

    /// BOUNDED-TIME LIVENESS — must become true within `deadline`.
    ///
    /// Anchor the deadline's start with [`Property::after`].
    pub fn eventually_within<N>(
        name: impl Into<String>,
        deadline: Duration,
        predicate: impl Fn(&World<'_, N>) -> bool + Send + Sync + 'static,
    ) -> Property<N> {
        Property {
            name: name.into(),
            kind: PropertyKind::EventuallyWithin { deadline },
            predicate: Box::new(predicate),
            after: None,
        }
    }
}

/// Construct a [`Duration`] of `n` seconds — a readable deadline in tests.
pub fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// Construct a [`Duration`] of `n` milliseconds.
pub fn millis(n: u64) -> Duration {
    Duration::from_millis(n)
}
