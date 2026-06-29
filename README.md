# propsim

In-memory, property-based testing for distributed protocols in Rust — author one
backend-neutral test definition and run it on a deterministic in-process
simulator, a sound transactional checker (Elle), and/or a real cluster (Jepsen).

See the full design in [`docs/architecture.md`](docs/architecture.md).

## Status

The family is organized by **stability tier** (architecture.md §17).

- **Tier 1 — the stable contract: complete.** The slow-cadence surface external
  executor/oracle plugins compile against.
- **Tier 2 — the batteries-included engine: usable.** The deterministic
  in-process simulator (Style-A `Node` state machines, virtual time + seeded
  RNG), native oracles (Porcupine-style linearizability), white-box properties,
  and the `propsim` facade. *Gaps:* the Style-B (`spawn_each`) executor and the
  rigorous/Elle backend preset are not implemented yet.
- **Tier 3 — heavy adapters: not implemented.** Elle and Jepsen are opt-in stubs;
  [`propsim-iroh`](crates/propsim-iroh) is **shape-only** (it pins the transport
  surface but does not pull `iroh` yet, pending iroh's pluggable custom-transport
  seam).

| Crate | Tier | What it is |
|---|---|---|
| [`propsim-history`](crates/propsim-history) | 1 | `OpEntry`/`History`, Jepsen EDN/JSON interchange, `Clock` provenance. Zero required deps. |
| [`propsim-core`](crates/propsim-core) | 1 | Plugin traits (`Executor`/`Oracle`/`Transport`/`Node`), `Backend` + capability negotiation, `Property`/`World`, `Faults`, `Seed`, `Scenario`. `proptest` is part of its public API. |
| [`propsim-sim`](crates/propsim-sim) | 2 | The deterministic in-process executor: single-threaded, virtual-time discrete-event simulator with a seeded RNG and in-memory transport. |
| [`propsim-oracle`](crates/propsim-oracle) | 2 | Native, no-external-process oracles; the flagship is a Porcupine-style linearizability checker. |
| [`propsim`](crates/propsim) | 2 | The batteries-included **facade**: re-exports the contract (`propsim-core`) and bundles the simulator and native oracles. Pure-Rust at default features. |
| [`propsim-iroh`](crates/propsim-iroh) | 3 | The iroh `Transport` adapter — **shape only**; real `Endpoint` binding lands later. |

## Quick taste

Author one `TestPlan` — a node state machine, a transport model, fault scripts,
and white-box properties — then run it on the deterministic simulator. (From
[`crates/propsim/tests/end_to_end.rs`](crates/propsim/tests/end_to_end.rs); a
larger partition/heal example is in
[`crates/propsim-sim/tests/worked_example.rs`](crates/propsim-sim/tests/worked_example.rs).)

```rust
use propsim::prelude::*;

let plan = Simulation::plan::<GossipNode>()
    .nodes(3)
    .transport(InMemory::ordered())
    .state_machine()
    .check([property::always("ids only", |w: &World<GossipNode>| {
        w.nodes().flat_map(|n| n.known.iter().copied()).all(|v| v < 3)
    })])
    .seeds(1)
    .finish();

let report = plan.run(propsim::deterministic());
assert!(report.verdicts.iter().all(|nv| nv.verdict.valid));
```

Capability negotiation is enforced at `run()`: a white-box property run against a
black-box backend (no world snapshots — e.g. a future Jepsen cluster) is rejected
up front rather than silently skipped.

## Feature flags

- `serde` (off by default) — adds serde derives on the public types and enables
  `Scenario` save/load. Default features keep the dependency closure minimal.
- `elle`, `jepsen` (facade, off by default) — opt-in Tier-3 adapters; stubs until
  their backend crates land.
- `iroh` (facade, off by default) — gates the [`propsim-iroh`](crates/propsim-iroh)
  shape-only adapter.

## Build & test

```bash
make test-propsim        # this workspace, default features
make test-all-features   # every feature closure (still offline / pure-Rust)
make check               # fmt-check + clippy + test + guard-no-iroh

# or directly:
cargo test --manifest-path Cargo.toml
```
