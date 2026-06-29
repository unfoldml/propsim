//! # propsim-history
//!
//! The Jepsen-shaped operation-history interchange type for `propsim` — the
//! "narrow waist" that lets every oracle be written once and run on either
//! executor (architecture.md §10).
//!
//! This crate is Tier 1 of the `propsim` family: a slow-cadence stable contract.
//! It has **zero required dependencies** — the EDN/JSON codecs are hand-written —
//! and is independently useful to any Rust project that wants to read or emit
//! Jepsen-shaped histories.
//!
//! ## Example
//!
//! ```
//! use propsim_history::{History, OpEntry, OpKind, ProcessId, Function, Value, Time};
//!
//! let h = History::new(vec![OpEntry {
//!     index: 0,
//!     time: Time::virtual_nanos(0),
//!     kind: OpKind::Invoke,
//!     process: ProcessId(0),
//!     f: Function::new("read"),
//!     value: Value::Nil,
//! }]);
//!
//! let edn = h.to_jepsen_edn();
//! let back = History::from_jepsen(&edn).unwrap();
//! assert_eq!(h, back);
//! ```

#![forbid(unsafe_code)]

mod edn;
mod history;
mod op;
mod time;

pub use history::History;
pub use op::{Function, OpEntry, OpKind, ProcessId, Value};
pub use time::{Clock, Time};

use std::fmt;

/// An error parsing a Jepsen history from EDN or JSON.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    msg: String,
}

impl ParseError {
    pub(crate) fn new(msg: impl Into<String>) -> Self {
        ParseError { msg: msg.into() }
    }

    /// The human-readable description.
    pub fn message(&self) -> &str {
        &self.msg
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "history parse error: {}", self.msg)
    }
}

impl std::error::Error for ParseError {}
