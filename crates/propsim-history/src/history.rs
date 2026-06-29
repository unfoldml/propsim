//! The [`History`] interchange type and its Jepsen EDN/JSON codecs.

use crate::edn::{self};
use crate::op::{Function, OpEntry, OpKind, ProcessId, Value};
use crate::time::{Clock, Time};
use crate::ParseError;

/// A recorded operation history — the narrow waist of the whole design
/// (architecture.md §10). Every executor produces one; every history-based
/// oracle consumes one.
#[derive(Clone, Debug, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct History(pub Vec<OpEntry>);

impl History {
    /// Build a history from its entries, in order.
    pub fn new(entries: Vec<OpEntry>) -> Self {
        History(entries)
    }

    /// The entries in recorded order.
    pub fn entries(&self) -> &[OpEntry] {
        &self.0
    }

    /// Render to Jepsen-shaped EDN — the default for the Elle path, which has
    /// fewer JSON→Clojure conversion edge cases (architecture.md §10).
    ///
    /// Emits one operation map per line:
    /// `{:index 0, :time 0, :clock :virtual, :type :invoke, :process 0, :f :read, :value nil}`
    pub fn to_jepsen_edn(&self) -> String {
        let mut out = String::new();
        for e in &self.0 {
            edn::write_edn(&mut out, &entry_to_value(e));
            out.push('\n');
        }
        out
    }

    /// Render to a JSON array of operation objects, for generic interop.
    pub fn to_jepsen_json(&self) -> String {
        let mut out = String::new();
        out.push('[');
        for (i, e) in self.0.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            edn::write_json(&mut out, &entry_to_value(e));
        }
        out.push(']');
        out
    }

    /// Ingest a Jepsen history. Accepts either:
    /// - EDN: one operation map per line (as emitted by [`Self::to_jepsen_edn`]
    ///   and by Jepsen `store/` histories), or
    /// - a single EDN/JSON vector of operation maps.
    pub fn from_jepsen(s: &str) -> Result<History, ParseError> {
        let trimmed = s.trim_start();
        let is_json = looks_like_json(trimmed);

        // A leading `[`/`(` means a single sequence of maps (EDN or JSON array).
        if trimmed.starts_with('[') || trimmed.starts_with('(') {
            let v = if is_json {
                edn::parse_json(trimmed)?
            } else {
                edn::parse_edn(trimmed)?
            };
            let items = match v {
                Value::List(items) => items,
                _ => return Err(ParseError::new("expected a vector of op maps")),
            };
            let entries = items
                .iter()
                .map(value_to_entry)
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(History(entries));
        }

        // Otherwise: line-delimited op maps (the EDN store/ shape). Blank lines
        // are skipped.
        let mut entries = Vec::new();
        for line in s.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let v = edn::parse_edn(line)?;
            entries.push(value_to_entry(&v)?);
        }
        Ok(History(entries))
    }
}

/// Distinguish a JSON history from an EDN one. Both can start with `[`, so we
/// look at the first object's first key: JSON quotes keys (`"index":`), EDN uses
/// keyword keys (`:index `). Falls back to EDN (the default wire) if no `{` is
/// found.
fn looks_like_json(s: &str) -> bool {
    match s.find('{') {
        Some(i) => s[i + 1..].trim_start().starts_with('"'),
        None => false,
    }
}

/// Build the EDN/JSON map value for one entry.
fn entry_to_value(e: &OpEntry) -> Value {
    Value::Map(vec![
        (Value::keyword("index"), Value::Int(e.index as i64)),
        (Value::keyword("time"), Value::Int(e.time.nanos)),
        (
            Value::keyword("clock"),
            Value::keyword(clock_kw(e.time.clock)),
        ),
        (Value::keyword("type"), Value::keyword(e.kind.keyword())),
        (Value::keyword("process"), Value::Int(e.process.0 as i64)),
        (Value::keyword("f"), Value::keyword(e.f.as_str())),
        (Value::keyword("value"), e.value.clone()),
    ])
}

fn clock_kw(c: Clock) -> &'static str {
    match c {
        Clock::Virtual => "virtual",
        Clock::Wall => "wall",
    }
}

fn clock_from_kw(s: &str) -> Result<Clock, ParseError> {
    match s {
        "virtual" => Ok(Clock::Virtual),
        "wall" => Ok(Clock::Wall),
        other => Err(ParseError::new(format!("unknown clock `:{other}`"))),
    }
}

/// Parse one entry from a map value. Missing `:clock` defaults to `Wall`, since
/// a foreign Jepsen `store/` history (which has no clock tag) is wall-clock.
fn value_to_entry(v: &Value) -> Result<OpEntry, ParseError> {
    let pairs = match v {
        Value::Map(pairs) => pairs,
        _ => return Err(ParseError::new("op entry is not a map")),
    };
    let get = |key: &str| -> Option<&Value> {
        pairs.iter().find_map(|(k, val)| match k {
            Value::Keyword(name) if name == key => Some(val),
            Value::Str(name) if name == key => Some(val),
            _ => None,
        })
    };

    let index = require_int(get("index"), "index")? as u64;
    let nanos = require_int(get("time"), "time")?;
    let clock = match get("clock") {
        Some(Value::Keyword(k)) | Some(Value::Str(k)) => clock_from_kw(k)?,
        Some(_) => return Err(ParseError::new("`:clock` is not a keyword")),
        None => Clock::Wall,
    };
    let kind = match get("type") {
        Some(Value::Keyword(k)) | Some(Value::Str(k)) => OpKind::from_keyword(k)
            .ok_or_else(|| ParseError::new(format!("unknown :type `:{k}`")))?,
        _ => return Err(ParseError::new("missing or malformed `:type`")),
    };
    let process = ProcessId(require_int(get("process"), "process")? as u64);
    let f = match get("f") {
        Some(Value::Keyword(k)) | Some(Value::Str(k)) => Function::new(k.clone()),
        _ => return Err(ParseError::new("missing or malformed `:f`")),
    };
    let value = get("value").cloned().unwrap_or(Value::Nil);

    Ok(OpEntry {
        index,
        time: Time { nanos, clock },
        kind,
        process,
        f,
        value,
    })
}

fn require_int(v: Option<&Value>, field: &str) -> Result<i64, ParseError> {
    match v {
        Some(Value::Int(i)) => Ok(*i),
        // JSON keyword keys may make ints arrive as strings; be lenient.
        Some(Value::Str(s)) => s
            .parse::<i64>()
            .map_err(|_| ParseError::new(format!("`:{field}` is not an integer"))),
        _ => Err(ParseError::new(format!(
            "missing or non-integer `:{field}`"
        ))),
    }
}
