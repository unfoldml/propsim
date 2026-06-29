//! Timestamps with provenance.
//!
//! A bounded-liveness oracle must know whether a timestamp came from the
//! simulator's virtual clock or from a real cluster's wall clock, because
//! "reconverges within 5s" means different things against each (architecture.md
//! §10, §14.1). Making the provenance part of the type is what keeps the two from
//! being silently compared.

/// Provenance of a [`Time`] value.
///
/// Part of the type by design: a virtual timestamp from the sim and a wall-clock
/// timestamp from Jepsen are both valid, but they are never interchangeable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Clock {
    /// The simulator's compressible virtual clock. Deadlines are exact and free.
    Virtual,
    /// A real wall clock (e.g. a Jepsen cluster). Subject to machine jitter.
    Wall,
}

/// A timestamp: a nanosecond value tagged with the clock it was read from.
///
/// Nanoseconds keep the type integral and lossless across the EDN/JSON seam;
/// Jepsen histories record `:time` in nanoseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Time {
    /// Nanoseconds since an executor-defined origin.
    pub nanos: i64,
    /// Which clock `nanos` was read from.
    pub clock: Clock,
}

impl Time {
    /// A virtual-clock timestamp (the simulator path).
    pub const fn virtual_nanos(nanos: i64) -> Self {
        Time {
            nanos,
            clock: Clock::Virtual,
        }
    }

    /// A wall-clock timestamp (the real-cluster path).
    pub const fn wall_nanos(nanos: i64) -> Self {
        Time {
            nanos,
            clock: Clock::Wall,
        }
    }
}
