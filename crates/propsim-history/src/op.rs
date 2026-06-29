//! The operation-entry model — Jepsen's history shape (architecture.md §10).
//!
//! We do not invent a format; we adopt Jepsen's so every downstream checker
//! (`elle-cli`, Porcupine, `history.sim`) speaks it for free.

use crate::time::Time;

/// A process (logical client/actor) in the history.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ProcessId(pub u64);

/// The lifecycle kind of an operation entry — Jepsen's `:type`.
///
/// An operation is `Invoke`d, then completes as `Ok` (succeeded), `Fail`
/// (definitely did not happen), or `Info` (indeterminate — may or may not have
/// taken effect, e.g. a timeout).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum OpKind {
    /// The operation was issued.
    Invoke,
    /// The operation succeeded.
    Ok,
    /// The operation definitely did not take effect.
    Fail,
    /// The operation's outcome is indeterminate (e.g. a timeout).
    Info,
}

impl OpKind {
    /// The Jepsen `:type` keyword name (without the leading colon).
    pub fn keyword(self) -> &'static str {
        match self {
            OpKind::Invoke => "invoke",
            OpKind::Ok => "ok",
            OpKind::Fail => "fail",
            OpKind::Info => "info",
        }
    }

    /// Parse a Jepsen `:type` keyword name.
    pub fn from_keyword(s: &str) -> Option<Self> {
        match s {
            "invoke" => Some(OpKind::Invoke),
            "ok" => Some(OpKind::Ok),
            "fail" => Some(OpKind::Fail),
            "info" => Some(OpKind::Info),
            _ => None,
        }
    }
}

/// The operation function — Jepsen's `:f` (e.g. `:txn`, `:read`, `:write`).
///
/// Kept as an interned keyword string: the set is open-ended per protocol, and
/// keeping it a string avoids baking a closed vocabulary into the stable Tier-1
/// contract. Rendered as an EDN/JSON keyword.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Function(pub String);

impl Function {
    /// Build a function from any string-like name.
    pub fn new(name: impl Into<String>) -> Self {
        Function(name.into())
    }

    /// The function name as a string slice (no leading colon).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<S: Into<String>> From<S> for Function {
    fn from(s: S) -> Self {
        Function(s.into())
    }
}

/// A self-describing op payload / return value, sufficient to render and parse
/// Jepsen-shaped EDN and JSON.
///
/// Deliberately minimal (architecture.md §10 calls this out as the one design
/// judgement call): the cases below cover Jepsen/Elle transaction values —
/// `[:append k v]`, `[:r k [v..]]`, `[:w k v]` micro-ops are lists of lists of
/// these scalars. `Keyword` carries Elle/Jepsen keywords like `:append`/`:r`/`:w`
/// and the not-yet-known read sentinel `nil`.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Value {
    /// EDN `nil` / JSON `null`.
    Nil,
    /// A boolean.
    Bool(bool),
    /// A 64-bit signed integer.
    Int(i64),
    /// A double, stored as bits for `Eq`/`Hash`-free exact round-tripping is not
    /// needed here; we keep it a plain `f64` and do not derive `Eq`.
    Float(f64),
    /// An EDN/JSON string (double-quoted).
    Str(String),
    /// An EDN keyword (`:foo`) — e.g. Elle micro-op tags `:append`, `:r`, `:w`.
    Keyword(String),
    /// An ordered list — EDN vector `[..]` / JSON array.
    List(Vec<Value>),
    /// A map — EDN `{..}` / JSON object. Keys are themselves values (Jepsen map
    /// keys are commonly keywords or ints).
    Map(Vec<(Value, Value)>),
}

impl Value {
    /// Convenience constructor for an EDN keyword value.
    pub fn keyword(name: impl Into<String>) -> Self {
        Value::Keyword(name.into())
    }
}

/// One entry in a [`History`](crate::History) — Jepsen's operation map.
///
/// Renders as `{:index i :time t :type :ok :process p :f :read :value v}`.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct OpEntry {
    /// Monotonic position in the history.
    pub index: u64,
    /// Timestamp, carrying its clock provenance ([`Time`]).
    pub time: Time,
    /// Lifecycle kind (`:type`).
    pub kind: OpKind,
    /// Issuing process (`:process`).
    pub process: ProcessId,
    /// Operation function (`:f`).
    pub f: Function,
    /// Payload / return value (`:value`).
    pub value: Value,
}
