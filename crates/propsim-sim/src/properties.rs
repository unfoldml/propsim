//! White-box property evaluation over a finished typed run (architecture.md §7.2).
//!
//! This is the typed path: properties read `&World<N>` with live node state, so
//! it can only run here, where `N` is in scope — never through the type-erased
//! `Box<dyn Executor>` seam.

use std::time::Duration;

use propsim_core::node::Node;
use propsim_core::{
    Anomaly, Event, NamedVerdict, Property, PropertyKind, Verdict, Witness, World, WorldTrace,
};

use crate::scheduler::RunEvent;

/// Evaluate every property against the recorded frames, returning one verdict
/// per property (tagged with its name).
pub fn evaluate_properties<N: Node>(
    properties: &[Property<N>],
    trace: &WorldTrace<N>,
    events: &[RunEvent],
) -> Vec<NamedVerdict> {
    properties
        .iter()
        .map(|p| NamedVerdict::new(p.name(), evaluate_one(p, trace, events)))
        .collect()
}

fn evaluate_one<N: Node>(
    prop: &Property<N>,
    trace: &WorldTrace<N>,
    events: &[RunEvent],
) -> Verdict {
    match prop.kind() {
        PropertyKind::Always => {
            for (t, nodes) in &trace.frames {
                let world = World::new(nodes, &trace.history, *t);
                if !prop.eval(&world) {
                    return Verdict::invalid(vec![Anomaly::with_detail(
                        "invariant-violated",
                        format!("`{}` false at t={:?}", prop.name(), t),
                    )])
                    .with_witness(Witness::Text(format!(
                        "property `{}` (always) violated at virtual t={:?}",
                        prop.name(),
                        t
                    )));
                }
            }
            Verdict::valid()
        }
        PropertyKind::Sometimes => {
            let ever = trace.frames.iter().any(|(t, nodes)| {
                let world = World::new(nodes, &trace.history, *t);
                prop.eval(&world)
            });
            if ever {
                Verdict::valid()
            } else {
                Verdict::invalid(vec![Anomaly::with_detail(
                    "reachability-unmet",
                    format!("`{}` never held in any state", prop.name()),
                )])
            }
        }
        PropertyKind::EventuallyWithin { deadline } => {
            let t0 = anchor_time(prop.after_event(), events);
            let lo = t0;
            let hi = t0 + *deadline;
            let met = trace.frames.iter().any(|(t, nodes)| {
                if *t < lo || *t > hi {
                    return false;
                }
                let world = World::new(nodes, &trace.history, *t);
                prop.eval(&world)
            });
            if met {
                // Annotate the clock provenance per architecture.md §14.1.
                Verdict::valid().with_witness(Witness::Text(
                    "deadline evaluated against virtual time".to_string(),
                ))
            } else {
                Verdict::invalid(vec![Anomaly::with_detail(
                    "deadline-exceeded",
                    format!(
                        "`{}` not satisfied within {:?} of {:?} (virtual time)",
                        prop.name(),
                        deadline,
                        prop.after_event()
                    ),
                )])
            }
        }
    }
}

/// Resolve the anchor time for a bounded-liveness deadline: the virtual time of
/// the run event the property's `.after(..)` names, or `0` for "from start".
fn anchor_time(after: Option<&Event>, events: &[RunEvent]) -> Duration {
    let Some(event) = after else {
        return Duration::ZERO;
    };
    for re in events {
        match (event, re) {
            (Event::NetworkHealed, RunEvent::NetworkHealed(t)) => return *t,
            (Event::NodeRejoined, RunEvent::NodeRejoined(t, _)) => return *t,
            _ => {}
        }
    }
    // The named event never occurred; anchor at zero so the deadline still has a
    // defined meaning (it will typically fail, which is the honest outcome).
    Duration::ZERO
}
