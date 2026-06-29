# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

- **propsim** — in-memory, property-based testing for distributed protocols (specified in [docs/architecture.md](docs/architecture.md)). grid-xla uses it to simulate node loss without re-implementing protocols.

## Engineering practices

- **Use the Rust Analyzer plugin** for symbol references, go-to-definition, and warnings rather than grepping (ask "What's the definition for this symbol?").
- **Property tests over unit tests** for algorithms and data transformations (`proptest`). Test "business" logic, not trivial data-structure properties.
- **In-memory / referentially-pure** implementations wherever possible — pure algorithms do no IO (disk, sockets), which keeps them in-memory-testable.
- **No mocking** — test real implementations only.
- Architecture and designs live in [docs/](docs/) and must be periodically reviewed. Mark or remove assumptions/conventions that no longer hold or are speculative.

## Architecture

One backend-neutral test definition (`Simulation::plan()…finish()`), runnable on a deterministic in-process simulator, a sound transactional checker (Elle), and/or a real cluster (Jepsen). The narrow waist that makes one test portable across all three is the Jepsen-shaped `History` type; the single knob that swaps executors is `.run(backend)`. **Capability negotiation runs at `run()`** (not compile time): a white-box property (`Needs::World`) on a black-box executor (no world snapshots) is rejected up front, by name, rather than silently skipped.

Crates are grouped by **stability tier** (architecture.md §17). All crates are `#![forbid(unsafe_code)]`.

- **Tier 1 — the stable contract (complete; slow cadence, aim for 1.0):**
  - **`propsim-history`** — `OpEntry`/`History`, Jepsen EDN/JSON interchange, `Clock` provenance. Zero required deps (hand-written codecs).
  - **`propsim-core`** — plugin traits (`Executor`, `Oracle`, `Transport`, `Node`), `Backend` + capability negotiation, `Property`/`World`, `Faults`, `Seed`, `Scenario`. `proptest` is part of its **public API** (the `Strategy` passed to `.workload(..)`), so a proptest major bump implies a propsim-core major bump (§17.4).
- **Tier 2 — the batteries-included engine (implemented and usable):**
  - **`propsim-sim`** — `DeterministicExecutor`: single-threaded, virtual-time discrete-event scheduler with a seeded RNG and in-memory transport (Style-A `Node` state machines).
  - **`propsim-oracle`** — native, no-external-process oracles; flagship is a Porcupine-style `linearizable()` checker.
  - **`propsim`** — the batteries-included **facade**: prelude, the `deterministic()` backend preset, re-exports the contract and bundles the simulator + native oracles. Pure-Rust at default features.
  - *Gaps:* Style-B `spawn_each` executor and the rigorous/Elle backend preset are not implemented yet.
- **Tier 3 — heavy adapters (not implemented):** `propsim-iroh` is **shape-only** (pins the transport surface, does not pull `iroh`); Elle (`elle`) and Jepsen (`jepsen`) are off-by-default facade stubs.

## Determinism contract

The core non-obvious convention. The simulator is only sound because **all nondeterminism flows through harness-owned seams**:

- Side effects only via the node `Ctx`: `cx.send`/`cx.broadcast` (network), `cx.now()` (time), `cx.rng()` (randomness). Never call ambient `now()`/`thread_rng()`.
- Never rely on `HashMap` iteration order (it seeds from OS entropy) — use a seeded hasher or `BTreeMap`.
- `Time` carries a `Clock` tag (`Virtual` vs `Wall`); **never compare a virtual timestamp to a wall-clock one** (architecture.md §14).
- `assert_deterministic` (in the prelude) is the regression guard: run the same seed twice, assert the event trace is byte-identical. Keep it green — a dependency bump can silently reintroduce nondeterminism.
- `PROPSIM_SEED=0x…` overrides the first seed for reproduction.

The **litmus test** (§17.3): `cargo test` at default features runs the full deterministic + native-oracle loop pulling no JVM, no Go, no cluster, and no `iroh`. `make guard-no-iroh` enforces the iroh half; if a change breaks the invariant, the change is wrong.

## Commands

The `Makefile` at the repo root is the entry point (`make help` lists every target). cargo is not on `PATH` in this environment — prefer `make` (each recipe sources `~/.cargo/env`), or `source "$HOME/.cargo/env"` before invoking cargo directly.

```bash
make check             # the full gate: fmt-check + clippy + test + guard-no-iroh
make test              # default-feature unit + integration tests (alias for test-propsim)
make test-unit         # library unit tests only
make test-all-features # every feature closure (still offline / pure-Rust)
make guard-no-iroh     # assert no real iroh in the default build (litmus test)

# one integration-test file (test names: jepsen_roundtrip, negotiation,
# register_linearizable, worked_example, replicated_kv_linearizable,
# end_to_end, determinism_guard):
cargo test --manifest-path Cargo.toml --test jepsen_roundtrip
```

## Feature flags

`serde` is off by default everywhere — it adds serde derives on public types (and `Scenario` save/load in propsim). `elle`, `jepsen`, and `iroh` are off-by-default facade flags gating Tier-3 adapters (stubs / shape-only until their backends land). Default features keep the dependency closure minimal so the litmus test holds.
