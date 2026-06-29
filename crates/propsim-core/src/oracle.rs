//! The [`Oracle`] plugin trait and its normalized verdict (architecture.md §9.1).

use std::path::PathBuf;

use crate::executor::RunOutput;
use crate::needs::Needs;

/// An oracle decides pass/fail from a [`RunOutput`].
///
/// Every oracle declares what it [`needs`](Oracle::needs); capability
/// negotiation checks that against the chosen executor before any seed runs.
pub trait Oracle {
    /// A stable, human-facing name used in negotiation errors and verdicts.
    fn name(&self) -> &str;

    /// What this oracle reads — `World` (white-box) or `History` (portable).
    fn needs(&self) -> Needs;

    /// Render a verdict from observed behavior.
    fn check(&self, out: &RunOutput) -> Verdict;
}

/// The normalized verdict every oracle yields.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Verdict {
    /// Whether the observed behavior is valid.
    pub valid: bool,
    /// Anomalies found (`G0` / `G-single` / `"two leaders in term 4"` / …).
    pub anomalies: Vec<Anomaly>,
    /// A minimal failing sub-history or shrunk scenario, if any.
    pub witness: Option<Witness>,
}

impl Verdict {
    /// A clean pass.
    pub fn valid() -> Self {
        Verdict {
            valid: true,
            anomalies: Vec::new(),
            witness: None,
        }
    }

    /// A failure carrying the named anomalies.
    pub fn invalid(anomalies: Vec<Anomaly>) -> Self {
        Verdict {
            valid: false,
            anomalies,
            witness: None,
        }
    }

    /// Attach a witness.
    pub fn with_witness(mut self, witness: Witness) -> Self {
        self.witness = Some(witness);
        self
    }
}

/// A named anomaly, optionally with a free-text detail.
///
/// The kind is an open string (e.g. Elle's `G-single`, or a white-box message
/// like `"two leaders in term 4"`) so the stable contract does not freeze a
/// closed anomaly vocabulary.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Anomaly {
    /// The anomaly kind/name.
    pub kind: String,
    /// Optional human-readable detail.
    pub detail: Option<String>,
}

impl Anomaly {
    /// An anomaly with just a kind/name and no detail.
    pub fn new(kind: impl Into<String>) -> Self {
        Anomaly {
            kind: kind.into(),
            detail: None,
        }
    }

    /// An anomaly with a kind/name and a human-readable detail.
    pub fn with_detail(kind: impl Into<String>, detail: impl Into<String>) -> Self {
        Anomaly {
            kind: kind.into(),
            detail: Some(detail.into()),
        }
    }
}

/// Evidence that explains a failing verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Witness {
    /// A free-text explanation (e.g. a rendered failing trace).
    Text(String),
    /// A path to a rendered artifact (e.g. an Elle Graphviz cycle witness).
    Artifact(PathBuf),
}
