//! The user-supplied sequential semantics a linearizability check searches
//! against (architecture.md §8.1).

use propsim_core::history::Value;

/// The response a sequential operation must have produced — compared against the
/// recorded `OpEntry.value` of the completing entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Response(pub Value);

impl Response {
    /// Wrap a value as the expected response of a sequential operation.
    pub fn ok(value: Value) -> Self {
        Response(value)
    }
}

/// An executable sequential model. The checker searches for an ordering of the
/// concurrent operations that, applied to this model, reproduces every observed
/// response.
pub trait SequentialModel {
    /// The model state. `Clone + Eq + Hash` so the search can memoize visited
    /// `(linearized-set, state)` pairs and prune.
    type State: Clone + Eq + std::hash::Hash;
    /// The decoded operation type.
    type Op: Clone;

    /// The initial state.
    fn init(&self) -> Self::State;

    /// Apply `op` to `state`, returning the next state and the response it must
    /// have produced.
    fn step(&self, state: &Self::State, op: &Self::Op) -> (Self::State, Response);

    /// Decode one history operation (the function name and its value payload)
    /// into a model op plus the response observed on completion.
    ///
    /// `value_on_complete` is the `:value` of the completing (`Ok`) entry, which
    /// for a read carries the value actually returned.
    fn decode(
        &self,
        f: &str,
        value_invoke: &Value,
        value_complete: &Value,
    ) -> Option<(Self::Op, Response)>;
}
