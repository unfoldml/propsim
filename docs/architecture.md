# `propsim` — Design

**In-memory, property-based testing for distributed protocols in Rust — one test definition, runnable on a deterministic in-process simulator, a sound transactional checker ([Elle](https://github.com/jepsen-io/elle)), and/or a real cluster ([Jepsen](https://github.com/jepsen-io/jepsen)).**

> Status: Draft / RFC · Working name `propsim` is provisional · Audience: contributors and prospective users.
>
> This document unifies three earlier drafts — the core design, the polymorphic-backends RFC, and the API-&-packaging RFC — into a single source of truth. Where those drafts disagreed (chiefly the builder shape and the crate layout), the resolutions are recorded in [§16 Reconciliations](#16-reconciliations).

---

## Table of contents

1. [TL;DR](#1-tldr)
2. [Can I test my algorithm with this? (read this first)](#2-can-i-test-my-algorithm-with-this-read-this-first)
3. [Goals and non-goals](#3-goals-and-non-goals)
4. [The shape of the design: two axes behind one knob](#4-the-shape-of-the-design-two-axes-behind-one-knob)
5. [Anatomy of a test](#5-anatomy-of-a-test)
6. [Integration styles: how your code plugs in](#6-integration-styles-how-your-code-plugs-in)
7. [Authoring a test plan (the backend-neutral surface)](#7-authoring-a-test-plan-the-backend-neutral-surface)
8. [Oracles](#8-oracles)
9. [Backends in depth](#9-backends-in-depth)
10. [The interchange format: Jepsen's history shape](#10-the-interchange-format-jepsens-history-shape)
11. [Worked example: failure recovery within a deadline](#11-worked-example-failure-recovery-within-a-deadline)
12. [The determinism contract](#12-the-determinism-contract)
13. [Adoption cost: what you trade](#13-adoption-cost-what-you-trade)
14. [The cross-backend pipeline](#14-the-cross-backend-pipeline)
15. [Relationship to neighboring tools](#15-relationship-to-neighboring-tools)
16. [Reconciliations](#16-reconciliations)
17. [Packaging: a crate family by stability boundary](#17-packaging-a-crate-family-by-stability-boundary)
18. [Architecture sketch](#18-architecture-sketch)
19. [Phasing](#19-phasing)
20. [Open questions](#20-open-questions)
21. [References (prior art)](#21-references-prior-art)

---

## 1. TL;DR

`propsim` lets you get *evidence* about the correctness of a distributed protocol —
"no two leaders per term", "every committed write survives a crash", "the cluster re-converges
within 5 seconds of a partition healing" — **without deploying anything**. Your nodes run inside a
single thread on a deterministic, in-memory simulated network with a virtual clock. Workloads and
faults are generated with [`proptest`](https://github.com/proptest-rs/proptest), so the tool
searches the failure space for you, and any failure replays from an 8-byte seed and shrinks to a
minimal counterexample.

The design is a thin composition of proven layers rather than a new paradigm:

```
proptest (generation + shrinking)
        │   you bring: a Strategy<Op> and a set of Properties
        ▼
propsim core: virtual clock · seeded RNG · in-memory Transport · fault scheduler
        │   you bring: a real implementation behind a Transport adapter (primary), or a Node
        ▼
oracles: invariants · reachability · bounded-time liveness · linearizability (Porcupine-style) · Elle
```

Two things make it more than "yet another simulator":

- **It is transport-agnostic.** The first adapter targets [iroh](https://github.com/n0-computer/iroh),
  so you can drive a real `iroh` protocol handler over an in-memory transport, but nothing in the
  core is iroh-specific. The primary integration path runs your *real* implementation unmodified;
  the trade that makes that possible — best-effort rather than total determinism — is spelled out in
  [§13](#13-adoption-cost-what-you-trade).

- **One test definition runs on several backends.** You author a single backend-neutral `TestPlan`,
  then choose how to execute and check it: a deterministic in-process simulator (the millisecond
  inner loop), the same execution checked by Elle (a *sound* transactional oracle, still no cluster),
  or a real cluster via Jepsen (production-observable fidelity, slow outer loop). The `and/or` in
  "Elle and/or Jepsen" is two independent dials, explained in [§4](#4-the-shape-of-the-design-two-axes-behind-one-knob).

---

## 2. Can I test my algorithm with this? (read this first)

Run this checklist before reading further. It should take about a minute.

### ✅ Good fit if *all* of these are true

- **Your protocol is message-passing** between a set of nodes (fixed or dynamic membership).
- **Correctness is expressible as a property** over observable state or an operation history — an
  invariant, a reachability claim, or a bounded-time liveness claim (see
  [§7.2](#72-properties-what-you-can-express)).
- **You can inject time and randomness.** Your code (and its hot dependencies) does not
  *unavoidably* read the wall clock (`Instant::now()`) or OS entropy (`rand::thread_rng()`) out of
  band — or you are willing to route those through the harness. See
  [§12](#12-the-determinism-contract).
- **You want a fast, reproducible, in-process inner loop** for CI — not (only) a real cloud
  deployment.

If you fit, here are systems people have tested with this *class* of tool: consensus
(Raft / Paxos / Viewstamped Replication), primary-backup replication, gossip / epidemic broadcast,
membership & failure detection, leader election, CRDT / eventually-consistent sync, distributed
locks, and bespoke `iroh` protocols.

### ⛔ Use a different tool if

| If you need to…                                                        | Reach for instead                                                                 |
|------------------------------------------------------------------------|-----------------------------------------------------------------------------------|
| Validate real wire bytes, NIC/kernel behavior, or actual NAT traversal | Integration tests · [iroh `netsim`](https://github.com/n0-computer/iroh) (Linux netns) · real deployment · or `propsim`'s own `jepsen(cluster)` backend |
| Find single-node concurrency bugs (data races, atomics, memory order)  | [`loom`](https://github.com/tokio-rs/loom) (exhaustive) · [`shuttle`](https://github.com/awslabs/shuttle) (randomized) |
| Get an *exhaustive proof* on small instances / verify a design         | [`stateright`](https://github.com/stateright/stateright) · TLA+                    |
| Benchmark throughput or latency                                        | A real load test — virtual time is **not** wall-clock performance                 |
| Test code you cannot make deterministic and cannot restructure         | You will fight the [determinism contract](#12-the-determinism-contract); start there |

### The one hard precondition

**All nondeterminism must flow through the harness** — the network, the clock, and the RNG. For
your *own* code that means routing those through the seams; for code you cannot edit (dependencies,
iroh internals) the harness virtualizes them by *interception* (a libc-level shim). The *strength*
of the guarantee scales with how completely that is achieved, and you keep it honest with a
[determinism-regression guard](#14-the-cross-backend-pipeline). See
[§13](#13-adoption-cost-what-you-trade) for the realistic churn and the central
reuse-vs-determinism trade.

This precondition bounds the **deterministic executor** specifically. The other backends relax it
deliberately — the Jepsen executor runs real binaries and is *not* seed-replayable, and that is the
point of it. The trick is that you author one plan and pay the determinism tax only on the cell that
provides the determinism guarantee.

---

## 3. Goals and non-goals

**Goals**

1. **Reuse the real implementation.** The primary path (Style B) runs your shipped code and its real
   substrate unmodified, virtualizing time and entropy by interception rather than asking you to
   reimplement against a model. This is the line that separates simulation testing from model
   checking (see [§15](#15-relationship-to-neighboring-tools)) — and the reason the greenfield-only
   Style A is offered with a warning, not as the default.
2. **Property-based.** You state *what* must hold; the tool generates workloads and faults and
   searches for a violation, scaling to realistic state sizes and long virtual histories.
3. **Deterministic & reproducible.** On the simulator, one seed replays one execution, byte-for-byte,
   and failures shrink to a minimal counterexample.
4. **Bounded-time liveness is first-class.** "Recovers within *T*" is a primary use case, not an
   afterthought — which requires a real virtual clock you can advance and compress.
5. **Transport-agnostic**, with iroh as the first adapter and a thin enough `Transport` trait that
   adding another transport is a small job.
6. **One definition, swap the backend.** The same test is the cheap inner loop, the sound rigorous
   check, and the high-fidelity cluster run; only the `.run(backend)` line changes. The honesty
   constraints across that seam are enforced by the API, not left to discipline.

**Non-goals**

- Not an exhaustive model checker. We *sample* the failure space; a passing run is strong evidence,
  not a proof. (Pair with [`stateright`](https://github.com/stateright/stateright)/TLA+ if you want
  exhaustiveness on small instances.)
- Not a concurrency checker for a single node's data structures (use
  [`loom`](https://github.com/tokio-rs/loom)/[`shuttle`](https://github.com/awslabs/shuttle)).
- Not a performance tool.
- Not a real-network or real-NAT test on the simulator (use the `jepsen(cluster)` backend, iroh's
  `netsim`, or integration tests for that fidelity).

---

## 4. The shape of the design: two axes behind one knob

The request that motivates the multi-backend design is "a deterministic backend and a rigorous
backend." That is the right *surface* — a single `.run(backend)` switch on an otherwise identical
test. But "rigorous" is not one monolithic alternative, because **"check with Elle" and "run on a
real cluster" are independent choices**:

- [Elle](https://github.com/jepsen-io/elle) checks a *history*. It does not care whether that history
  came from a real cluster or from our in-process simulator.
  [`elle-cli`](https://github.com/ligurio/elle-cli) exists precisely to check JSON/EDN histories
  produced by Jepsen tests "as well as other Jepsen-similar frameworks that produce histories."
- [Jepsen](https://github.com/jepsen-io/jepsen) executes against real binaries on a real cluster.
  *That* is what buys production-observable fidelity.

So the principled decomposition is **two orthogonal axes** behind one ergonomic surface:

```
                 Oracle axis  ───────────────────────────────►
                 native (white-box + Porcupine)   Elle / Porcupine (history)
Executor    ┌──────────────────────────────────┬───────────────────────────────┐
axis        │  deterministic()                 │  rigorous()                   │
 │          │  in-proc sim + native oracles    │  in-proc sim + Elle           │
 │ in-proc  │  • white-box state properties    │  • same execution, no cluster │
 │ sim      │  • virtual time, seed-replayable │  • sound transactional check  │
 ▼          ├──────────────────────────────────┼───────────────────────────────┤
 real       │  (rarely useful: black-box exec  │  jepsen(cluster)              │
 cluster    │   with native white-box oracles  │  real binary + Elle           │
 (Jepsen)   │   — §9, mostly rejected)         │  • highest fidelity, slow     │
            └──────────────────────────────────┴───────────────────────────────┘
```

The **"and/or"** in the brief maps exactly to these two dials. `rigorous()` turns on the Elle
*oracle* over our own deterministic execution (no JVM cluster, still seed-replayable, still virtual
time — it loses nothing and gains a sound checker). `jepsen(cluster)` turns on the Jepsen *executor*.
You can turn on either or both.

**The narrow waist that makes any cell composable is a single shared interchange type: the operation
history, in Jepsen's shape** ([§10](#10-the-interchange-format-jepsens-history-shape)). Because the
history is portable, history-based oracles (our native linearizability checker *and* Elle) are
written once and run identically on either executor. Everything else in this document follows from
that one type.

---

## 5. Anatomy of a test

A test is authored once as a backend-neutral `TestPlan`. Only the `.run(backend)` call changes
between the fast inner loop, the rigorous check, and the full-fidelity cluster run.

```rust
use propsim::prelude::*;

fn plan() -> TestPlan<Replica> {
    Simulation::plan()
        .nodes(5)
        .transport(InMemory::unordered_lossy())     // network semantics
        .state_machine::<Replica>()                 // Style A  — OR  .spawn_each(|ep| …) for Style B
        .workload(client_ops())                     // a proptest Strategy<Value = Op>
        .faults(
            Faults::swarm()                         // omit a random fault subset per seed
                .partitions().crash_restart().latency_ms(0..200)
                .mode(Mode::Liveness),              // injure, then heal, then assert progress
        )
        .check([
            // history-based  → portable across every executor:
            oracle::linearizable::<RegisterModel>(),
            // white-box      → deterministic executor only; run() rejects it under jepsen():
            property::always("at most one leader per term", no_two_leaders),
            // bounded-time liveness → exact under virtual time; noisy on a real cluster (§14):
            property::eventually_within("reconverges in 5s", secs(5), all_agree)
                .after(Event::NetworkHealed),
        ])
        .seeds(2_000)                               // 2000 randomized executions
        .finish()                                   // -> TestPlan<Replica>
}

#[test]
fn fast()     { plan().run(Backend::deterministic()); }   // ms, laptop, plain CI
#[test]
fn rigorous() { plan().run(Backend::rigorous()); }        // same exec + Elle; still no cluster
#[test]
#[ignore = "needs a cluster + JVM; nightly/outer loop"]
fn full()     { plan().run(Backend::jepsen(ClusterSpec::aws_5_node())); }
```

`Backend::rigorous()` is the sweet spot and the primary meaning of "rigorous": identical
deterministic execution to `fast()`, so the white-box property and the virtual-time deadline still
work, *plus* Elle over the same in-process history. No JVM cluster, no SSH, no VMs — just an
`elle-cli` jar on the path.

If a property is violated, `run` aborts the test with a reproduction block:

```text
FAILED  property "re-converges within 5s after heal"
  seed     = 0x9f3a17c4e2b01a55   (re-run: PROPSIM_SEED=0x9f3a17c4e2b01a55 cargo test fast)
  shrunk   = 3 nodes · 2 ops · 1 partition·heal
  trace    : t=0ms      n0→n1  AppendEntries(term=2)
             t=1500ms   ── partition heal {n2} | {n0,n1} ──
             t=6500ms   n2.state=Follower(term=1)  ≠  quorum.term=2   ← deadline exceeded
  scenario : exported → ce.scenario   (reproduce on another backend with PROPSIM_SCENARIO=ce.scenario)
```

The **seed reproduces the deterministic executor only**. The portable artifact that crosses to a
real cluster is the shrunk `Scenario`, not the seed — see [§14](#14-the-cross-backend-pipeline).

### 5.1 `run` vs `try_run`

```rust
impl<N: Node> TestPlan<N> {
    /// Panics on violation. The panic message is the reproduction block above. Use in `#[test]`.
    pub fn run(self, backend: Backend) -> Report;

    /// Never panics. Errors carry the shrunk Scenario so a pipeline can hand it to another backend.
    pub fn try_run(self, backend: Backend) -> Result<Report, RunFailure>;
}
```

`run` is what you write in a `#[test]`. `try_run` is for the cross-backend pipeline in
[§14](#14-the-cross-backend-pipeline), where a failure is *data* you forward, not an assertion that
ends the process.

---

## 6. Integration styles: how your code plugs in

The harness can drive your protocol two ways. They are **not symmetric**, and the asymmetry is the
single most important thing to understand before adopting `propsim`: *only one of them reuses your
real implementation.* Both styles share the entire workload / fault / property / oracle / backend
surface above — only the node-definition step differs.

### 6.1 Style B — real implementation over a `Transport` (the primary path)

You keep your real async code — for iroh, the real `Endpoint`, `Router`, `ProtocolHandler`, and the
real noq QUIC stack — and the harness installs an in-memory transport beneath it. This is the path
that honours the "reuse as much of the system-under-test as possible" principle: your protocol logic
*and* its substrate run unmodified.

The catch is that time and entropy reached by your code or its transitive dependencies are not
controlled by swapping the transport alone. `propsim` closes that gap by **interception** —
overriding `clock_gettime`/`getrandom` at the libc seam so the *unmodified* binary reads virtual
time and the seeded RNG, the technique S2 documents in [`mad-turmoil`](https://s2.dev/blog/dst) and
that [`madsim`](https://github.com/madsim-rs/madsim) uses. Interception is precisely what lets us
reuse code we cannot edit. Its honest limit: determinism is **"very high, not total"** — completeness
equals the set of time/entropy code paths actually intercepted (see
[§13](#13-adoption-cost-what-you-trade)).

### 6.2 Style A — `Node` state machine (greenfield only; this is a rewrite, not reuse)

You implement a small synchronous trait where *every* side effect goes through a harness-owned
context. This is the [sled](http://sled.rs/simulation.html) / Polar Signals
[state-machine](https://www.polarsignals.com/blog/posts/2025/07/08/dst-rust) pattern, and it gives
**total determinism** with no interception, because nondeterminism has no path to leak.

But be clear-eyed: for an *existing* protocol, writing it against the `Node` trait is
**reimplementing it in a second form** — exactly the model-vs-implementation divergence that
simulation testing exists to avoid (a `stateright`/TLA+ model carries the same drawback). Style A
earns its keep only when you are building the protocol from scratch and choose this architecture
deliberately; then the "model" and the "implementation" are the same artifact and there is nothing
to diverge.

> **Rule of thumb.** Existing real stack (the iroh case, and most adopters) → **Style B**.
> Greenfield protocol you are designing now and are willing to structure as state machines →
> **Style A**. Porting working code into Style A purely for a cleaner determinism story is usually
> the wrong trade: you surrender reuse, which was the point.

### 6.3 Style A in code

*Greenfield path — you write the protocol against the trait; there is no pre-existing implementation
to reuse:*

```rust
use propsim::{Node, Ctx, NodeId};

struct Replica { role: Role, term: Term, log: Log /* … */ }

impl Node for Replica {
    type Msg   = RaftMsg;
    type Timer = RaftTimer;

    fn on_start(&mut self, cx: &mut Ctx<Self>) {
        // randomized election timeout — note: cx owns the clock and the RNG
        cx.set_timer(RaftTimer::Election, cx.rng().duration_ms(150..300));
    }

    fn on_msg(&mut self, from: NodeId, msg: RaftMsg, cx: &mut Ctx<Self>) {
        // pure logic; ALL effects are routed through cx
        match msg {
            RaftMsg::RequestVote { term, .. } if term > self.term => {
                self.term = term;
                cx.send(from, RaftMsg::Vote { term, granted: true });
            }
            _ => {}
        }
    }

    fn on_timer(&mut self, t: RaftTimer, cx: &mut Ctx<Self>) {
        if let RaftTimer::Election = t {
            self.start_election(cx); // calls cx.broadcast(RequestVote { … }) etc.
        }
    }
}
```

The `Ctx` is the entire effect surface: `cx.send`, `cx.broadcast`, `cx.set_timer`,
`cx.cancel_timer`, `cx.now()` (virtual), `cx.rng()` (seeded). Because there is no other way to
observe time or randomness, runs are perfectly deterministic.

### 6.4 Style B in code (iroh)

Your protocol is unchanged:

```rust
// Real iroh code — exactly what ships.
#[derive(Clone)]
struct GossipProtocol { /* … */ }

impl iroh::protocol::ProtocolHandler for GossipProtocol {
    async fn accept(&self, conn: iroh::endpoint::Connection) -> Result<()> {
        // real gossip logic over a real QUIC connection
        Ok(())
    }
}
```

The harness binds real `Endpoint`s on an in-memory transport (using iroh's pluggable
[custom-transport](https://github.com/n0-computer/iroh/issues/3848) seam, which lists "test
transports" as an explicit motivation), and runs your handler on each — the `.spawn_each(…)`
node-definition verb:

```rust
Simulation::plan()
    .nodes(8)
    .transport(IrohInMemory::default())          // implements iroh's transport trait
    .spawn_each(|endpoint| {
        Router::builder(endpoint)
            .accept(ALPN, Arc::new(GossipProtocol::new()))
            .spawn()
    })
    // …workload / faults / properties identical to Style A…
    .finish()
```

What is *not* automatic, and worth stating plainly: iroh's internals read the clock (idle timeouts,
keep-alives, hole-punch timers) and draw entropy (connection IDs), and so does noq. Swapping the
transport makes message *delivery* deterministic, but those internal timers still fire at wall-clock
instants — and would also defeat time-compression — unless iroh's clock *is* the harness's virtual
clock. Today iroh does not expose an injected clock/RNG, so Style B relies on interception to
virtualize those reads from outside the code. That keeps reuse maximal; the residual risk is a path
that fetches time or entropy in a way the shim does not cover, which the
[determinism-regression guard](#14-the-cross-backend-pipeline) exists to catch. The principled
endgame is upstream injection seams in iroh/noq (an accepted `Clock`/`Rng`), which would make this
total rather than very-high — a contribution effort, not something `propsim` can do alone.

### 6.5 Builder verbs

`.build::<N>()` from the earliest draft is **retired** — it conflated "node type" with "finalize,"
and "build" now belongs to backend validation. The replacements separate the concerns:

| Concern | Method | Notes |
|---|---|---|
| Cluster size | `.nodes(n)` | fixed membership; dynamic membership is a later `.membership(…)` |
| Node definition (Style A) | `.state_machine::<N>()` | `N: Node`; greenfield, total determinism |
| Node definition (Style B) | `.spawn_each(\|ep\| …)` | real handlers over a `Transport`; the iroh path |
| Finalize | `.finish()` | yields `TestPlan<N>` |
| Execute | `.run(backend)` / `.try_run(backend)` | capability check happens *here*, first |

Everything between `.transport(…)` and `.seeds(…)` is identical across the two styles and across all
backends.

---

## 7. Authoring a test plan (the backend-neutral surface)

### 7.1 Workloads: a `proptest` `Strategy`

Operations issued by simulated clients are generated, not enumerated, so the search reaches deep
states. You reuse [`proptest`](https://github.com/proptest-rs/proptest) directly, which means
composable generators and free shrinking:

```rust
fn client_ops() -> impl Strategy<Value = Op> {
    prop_oneof![
        any::<Key>().prop_map(Op::Read),
        (any::<Key>(), any::<Value>()).prop_map(|(k, v)| Op::Write(k, v)),
        Just(Op::Cas).prop_flat_map(/* … */),
    ]
}
```

### 7.2 Properties: what you can express

Property vocabulary is borrowed from `stateright`'s `Always` / `Sometimes` / `Eventually`
([rationale](https://docs.rs/stateright/latest/stateright/actor/index.html)), extended with a
quantitative deadline because we have a real virtual clock. A predicate receives a read-only
`World<N>`: a snapshot of every node's state plus the recorded operation history and the current
virtual time — `w.nodes()`, `w.history()`, `w.now()`.

**Portability is the honest core here, and it splits properties into two kinds:**

| Property kind | Reads | Deterministic exec | Jepsen exec |
|---|---|---|---|
| **State-based** (`always`/`sometimes`/`eventually` over `World`) | every node's *internal* state | ✅ (white-box) | ❌ — Jepsen is black-box; it cannot see internal node state |
| **History-based** (linearizability, transactional isolation) | the recorded *client operation history* | ✅ | ✅ |

This yields a rule enforced in the builder/negotiation layer rather than discovered at runtime:

> **A white-box property cannot run on a black-box executor.** Pair
> `property::always("one leader per term", …)` with `Backend::jepsen(...)` and you get a *fail-fast*
> error at `run()` naming the offending property — not a silent skip. The corollary is the pleasant
> part: because the history is portable, history-based oracles run identically on either executor.

The three property constructors:

```rust
// 1) SAFETY / INVARIANT — must hold in every reachable state. (Needs::World)
const NO_TWO_LEADERS: Property<Replica> = property::always(
    "at most one leader per term",
    no_two_leaders,                                 // a non-capturing fn(&World<Replica>) -> bool
);

// 2) REACHABILITY / NON-TRIVIALITY — guards against a vacuous spec
//    ("does the system ever actually commit anything?"). (Needs::World)
const CAN_COMMIT: Property<Replica> = property::sometimes(
    "some write commits",
    |w: &World<Replica>| w.history().any(|e| e.is_committed_write()),
);

// 3) BOUNDED-TIME LIVENESS — the headline use case. (Needs::World)
//    "After the network heals, all nodes agree within 5 seconds of virtual time."
fn reconverges(deadline: Duration) -> Property<Replica> {
    property::eventually_within("re-converges after heal", deadline, move |w| {
        w.all_nodes_agree_on_log()
    })
    .after(Event::NetworkHealed)                    // the clock for the deadline starts at this event
}
```

**A documentation nuance worth stating explicitly:** the `const` form works *only* for non-capturing
predicates (they coerce to `fn`). Anything that closes over configuration (a deadline, a cluster
size) uses the `impl Fn` builder form, built at runtime. Both exist on purpose; the `const` form
lets a non-capturing predicate be a module-level constant reused across tests.

For consistency models, use a dedicated **oracle** that checks a recorded history against an
executable model rather than a `World` predicate (see [§8](#8-oracles)).

### 7.3 Faults: a swarm-configurable schedule with safety vs. liveness modes

Two ideas from the systems lineage are built in. **Swarm testing**
([Groce et al., ISSTA 2012](https://www.flux.utah.edu/paper/groce-issta12)): each seed *omits* a
random subset of fault kinds, which empirically finds more bugs than always enabling everything.
**Safety vs. liveness modes**
([TigerBeetle](https://tigerbeetle.com/blog/2023-07-06-simulation-testing-for-liveness/)): liveness
can only be checked if the scheduler *heals* the environment and then asserts progress.

```rust
let faults = Faults::swarm()
    .partitions()                 // arbitrary cuts, including asymmetric
    .crash_restart()              // crash with durable state, later restart & rejoin
    .latency_ms(0..200)           // quantitative per-link latency / jitter
    .drop(prob(0.01))             // packet loss
    .duplicate(prob(0.005))       // re-delivery
    .reorder()                    // out-of-order delivery
    .mode(Mode::Liveness);        // Mode::Safety injects uniformly; Liveness injures→heals→checks
```

A `Faults` value is a **backend-neutral description**. The deterministic executor interprets it
against its in-memory scheduler; the Jepsen executor lowers it to a nemesis (iptables partitions,
`killall` crashes, clock skew on real VMs). The vocabulary maps cleanly because it was lifted from
this lineage in the first place; quantitative latency/jitter is faithful on the sim and approximate
on real hardware — and the verdict says which.

A `Faults::scripted()` form expresses a specific, non-random scenario (used in the worked example,
[§11](#11-worked-example-failure-recovery-within-a-deadline)).

### 7.4 Network semantics

The in-memory transport exposes a semantics taxonomy (extending `stateright`'s `Ordered` /
`UnorderedNonDuplicating` / `UnorderedDuplicating`) with quantitative delay:

```rust
InMemory::ordered()            // per-link FIFO, no loss
InMemory::unordered_lossy()    // reorder + loss + duplication
InMemory::with(NetworkModel {
    delay_ms: 5..50,
    loss: prob(0.02),
    duplicate: prob(0.0),
    ordered: false,
})
```

### 7.5 Workload & fault portability across the seam

**Workload — frozen schedule by default.** `proptest` is Rust-native; Jepsen's generators are
Clojure. Rather than maintain generation logic twice:

> The deterministic executor's proptest `Strategy` generates a *concrete* op stream per seed; the
> Jepsen executor *replays that exact stream* as a fixed generator.

This keeps a single source of generation truth, makes the workload identical across backends
(essential for the pipeline in [§14](#14-the-cross-backend-pipeline)), and — crucially — keeps
**shrinking** working: proptest shrinks on the deterministic executor; the Jepsen executor replays
the already-shrunk scenario. A real cluster cannot be cheaply re-run, so shrinking there is
infeasible by construction; **we shrink cheap and reproduce expensive.** (An optional
`WorkloadMode::NativeJepsenGenerator` exists for teams who want Jepsen's own generative diversity, at
the cost of a second generation path and loss of shrink-portability. Off by default.)

---

## 8. Oracles

An oracle decides pass/fail from observed behavior. Every oracle is tagged by what it `Needs`
(`World` or `History`), which is what capability negotiation checks against the chosen executor.

### 8.1 Native oracles (no external process)

- **Invariant / reachability / bounded-liveness** — `Needs::World`. Evaluated from the `World` /
  `WorldTrace` as shown in [§7.2](#72-properties-what-you-can-express). Deterministic executor only.
- **Native linearizability & sequential consistency** — `Needs::History`. A
  [Porcupine](https://github.com/anishathalye/porcupine)-style checker (Wing & Gong / Lowe
  algorithms) run over the recorded client history against an executable model you supply. This is
  the same approach S2 uses to
  [validate linearizability from DST histories](https://s2.dev/blog/linearizability), and it is far
  faster than the original [Knossos](https://github.com/jepsen-io/knossos) approach. Portable across
  executors; the fast default for single-object registers.

```rust
// You describe the sequential semantics; the oracle searches for a valid linearization.
struct RegisterModel;
impl SequentialModel for RegisterModel {
    type State = Option<Value>;
    type Op    = RegisterOp;
    fn step(&self, s: &Self::State, op: &Self::Op) -> (Self::State, Response) { /* … */ }
}

let linearizable = oracle::linearizable::<RegisterModel>();   // Needs::History
```

### 8.2 The Elle bridge — the rigorous oracle

[Elle](https://github.com/jepsen-io/elle) is the reason to reach outside Rust at all: it is **sound**
against every non-predicate anomaly in Adya/Liskov/O'Neil and is roughly *linear* in history length
and constant in concurrency, where a linearizability checker is NP-complete. The bridge is a thin
shell-out — exactly the integration mode Elle documents for non-JVM callers ("write your history to a
file or stream, and call a small wrapper program to produce output") — over
[`elle-cli`](https://github.com/ligurio/elle-cli).

```rust
pub struct Elle {
    models:    Vec<Model>,        // strict-serializable (default), serializable,
                                  // snapshot-isolation, repeatable-read, read-committed, …
    anomalies: Vec<AnomalyKind>,  // G0 (default), G1a, G1b, G1c, G-single, G2, *-process, *-realtime
    datatype:  ElleDatatype,      // ListAppend (preferred) | Register (weaker)
    jar:       ElleCliJar,        // discovered, or bundled and pinned
}

impl Oracle for Elle {
    fn needs(&self) -> Needs { Needs::History }
    fn check(&self, out: &RunOutput) -> Verdict {
        // 1. out.history.to_jepsen_edn()  ->  temp file
        // 2. java -jar elle-cli.jar -m list-append \
        //        --consistency-models strict-serializable  history.edn
        // 3. parse stdout: { :valid? , :anomaly-types , :not , :also-not , … }
        //    plus the Graphviz witness Elle renders for each cycle
        // 4. map -> Verdict { valid, anomalies: [G-single, …], witness: Some(graphviz_path) }
    }
}
```

**The one real constraint, surfaced not hidden.** Elle's strongest inference needs histories where
"reads of an object yield its entire version history and a unique mapping exists between versions and
transactions" — i.e. **append-only lists**. It can make *limited* inferences from read-write
registers but "shines with append-only lists." So the Elle oracle requires the workload's ops to be
expressible in Elle's transaction language, encoded as a trait:

```rust
/// Required for the Elle oracle. list-append is strongly preferred; register is the fallback.
pub trait ElleEncodable {
    fn encode(&self) -> ElleTxn;  // Vec<[:append k v] | [:r k _] | [:w k v]>
}
```

If your protocol's operations cannot be modeled as list-append or rw-register, the Elle oracle does
not apply — you fall back to native oracles on the same execution. `run()` reports this as a
fail-fast error rather than producing meaningless results. This is the honest boundary of the
rigorous *oracle*, just as the determinism contract is the honest boundary of the deterministic
*executor*.

A soundness caveat we pass through rather than paper over: Elle itself recommends checking reported
anomalies by hand, since for black-box systems an anomaly claim means "in any compatible Adya
history, this anomaly or something worse occurred." We surface that text alongside Elle verdicts.

### 8.3 Porcupine bridge (optional)

Same shell-out pattern for teams that prefer the standalone Go
[Porcupine](https://github.com/anishathalye/porcupine) binary for single-object linearizability:
serialize history → invoke → parse. Feature-gated, off by default.

### 8.4 Roadmap oracles

Behind the stable `Oracle` seam: lineage-driven fault injection
([Alvaro et al., SIGMOD 2015](https://mwhittaker.github.io/papers/html/alvaro2015lineage.html)) and
model-/coverage-guided exploration ([arXiv:2410.02307](https://arxiv.org/pdf/2410.02307)). Both slot
in additively without touching the test-authoring surface.

---

## 9. Backends in depth

### 9.1 The plugin contract (the stable core)

Two plugin traits (`Executor`, `Oracle`), one composition type (`Backend`), one interchange type
(`History`), and a capability negotiation that turns incompatible combinations into clear errors.
This is the surface external executors and oracles compile against; it must be small and slow to
change.

```rust
/// What an executor can produce for oracles to consume.
#[derive(Clone, Copy)]
pub struct Produces {
    pub world_snapshots: bool, // white-box per-node internal state
    pub history:         bool, // client operation history (true in practice)
    pub virtual_time:    bool, // deadlines in compressible virtual time
    pub seed_replayable: bool, // byte-for-byte replay from an 8-byte seed
}

/// What an oracle needs to render a verdict.
pub enum Needs { World, History }

pub trait Executor {
    fn capabilities(&self) -> Produces;
    fn run(&self, plan: &PlanData, seed: Seed) -> RunOutput;
}

pub trait Oracle {
    fn name(&self) -> &str;
    fn needs(&self) -> Needs;
    fn check(&self, out: &RunOutput) -> Verdict;
}

/// Every executor yields the same normalized output shape.
pub struct RunOutput {
    pub history:     History,             // always present, Jepsen-shaped (§10)
    pub world_trace: Option<WorldTrace>,  // Some(..) iff capabilities().world_snapshots
    pub artifacts:   Artifacts,           // backend extras: store/ paths, plots, graphviz
    pub seed:        Seed,
}

/// Every oracle yields the same normalized verdict shape.
pub struct Verdict {
    pub valid:     bool,
    pub anomalies: Vec<Anomaly>,          // G0 / G-single / "two leaders in term 4" / …
    pub witness:   Option<Witness>,       // minimal failing sub-history or shrunk scenario
}

pub struct Backend { executor: Box<dyn Executor>, oracles: Vec<Box<dyn Oracle>> }

impl Backend {
    pub fn deterministic() -> Self;                          // sim + native oracles
    pub fn rigorous()      -> Self;                          // sim + native + Elle (no cluster)
    pub fn jepsen(c: ClusterSpec) -> Self;                   // Jepsen executor + Elle
    pub fn custom(exec: impl Executor + 'static) -> BackendBuilder; // arbitrary combos
}
```

### 9.2 Capability negotiation is a runtime check, by deliberate choice

The hard rule — *a white-box property cannot run on a black-box executor* — could in principle be a
**compile-time** error via typestate (encode each property's `Needs` in the plan's type; bound
backends to match). We reject that for the primary surface:

- Typestate would make `TestPlan<N>`'s type encode its property set, so `plan()` could no longer
  return a single named type reused across `fast/rigorous/full` — it would defeat the one-line
  backend swap that is the whole point.
- The error a beginner would hit is an inscrutable trait-bound wall, not a sentence.

Instead, `run`/`try_run` validate **once, before any seed executes**, and fail with a sentence that
names the offender:

```rust
fn validate(exec: &dyn Executor, oracles: &[Box<dyn Oracle>]) -> Result<(), BackendError> {
    let caps = exec.capabilities();
    let offenders: Vec<_> = oracles.iter()
        .filter(|o| matches!(o.needs(), Needs::World) && !caps.world_snapshots)
        .map(|o| o.name().to_owned())
        .collect();
    if offenders.is_empty() { Ok(()) }
    else { Err(BackendError::WhiteBoxOraclesOnBlackBoxExecutor(offenders)) }
    // e.g. "property `at most one leader per term` reads internal state; Backend::jepsen is black-box"
}
```

Fail-fast-with-a-good-message beats compile-time-with-a-bad-message for an adoption-focused testing
library. (We still get *some* static help: `property::always` is only constructible against a node
type, so a register-only Elle plan simply never mentions one.)

### 9.3 `DeterministicExecutor` (the core)

The existing `propsim` engine: single-threaded scheduler, virtual clock, seeded RNG, in-memory
`Transport`, supporting both Style A and Style B (the latter with libc-level interception).

```rust
Produces { world_snapshots: true, history: true, virtual_time: true, seed_replayable: true }
```

It is the only executor that can satisfy white-box properties, virtual-time deadlines, and
byte-for-byte seed replay. Everything in [§12 the determinism contract](#12-the-determinism-contract)
and [§14 the determinism-regression guard](#14-the-cross-backend-pipeline) applies to *this executor
specifically*.

### 9.4 `JepsenExecutor` (Phase 2)

Drives a real Jepsen run and ingests its output.

```rust
Produces { world_snapshots: false, history: true, virtual_time: false, seed_replayable: false }
```

Note the falses: no internal state (black-box), real wall-clock (no compression), and **no seed
replay** — execution is against a real cluster over a real network. (Jepsen 0.3.10 added a seedable
RNG for its *own choices* and an Antithesis integration for determinism, but plain cluster execution
is nondeterministic; we do not pretend otherwise.)

**Bridge mechanism — data-driven, not codegen.** We do **not** generate a bespoke Clojure program
per test. Instead:

1. `propsim` serializes the `TestPlan` (topology, the *frozen op stream* from
   [§7.5](#75-workload--fault-portability-across-the-seam), the nemesis schedule, the chosen checker,
   the db/os plugin name) to a backend-neutral **EDN test spec**.
2. A single, fixed, `propsim`-maintained Clojure runner (shipped as an uberjar) reads that spec and
   constructs a Jepsen test map — Jepsen tests *are* just maps — then runs it.
3. `propsim` spawns the JVM, waits, and ingests `store/<test>/<date>/`: the `:valid?` verdict, the
   history (parsed back via `History::from_jepsen`), and artifacts (perf plots, availability plots,
   op-color plots).

This keeps the Clojure surface **small and fixed** (one generic runner we test once) instead of an
open-ended code generator.

**The irreducible Clojure.** How to install and start *your* system on each node is inherently
system-specific — Jepsen's `db`/`os` protocols. We cannot abstract that away. The design isolates it
as a named plugin the user supplies (or selects from a small registry), and nothing else in the Rust
surface leaks Clojure:

```rust
ClusterSpec::aws_5_node()
    .db(JepsenDb::from_registry("etcd"))      // or JepsenDb::custom(path_to_clj_ns)
    .os(JepsenOs::Debian);
```

The "rarely useful" bottom-left cell of the [§4](#4-the-shape-of-the-design-two-axes-behind-one-knob)
matrix (Jepsen executor + native white-box oracle) is rejected by capability negotiation for
state-based properties; only *history-based* native oracles could run there, which is a niche we
allow but do not advertise.

---

## 10. The interchange format: Jepsen's history shape

We do **not** invent a history format; we adopt Jepsen's operation-entry model so every downstream
checker (`elle-cli`, Porcupine, [`history.sim`](https://github.com/jepsen-io/history.sim)) speaks it
for free. `history.sim` emits exactly this in EDN or JSON "so you can benchmark checkers written in
other languages," and `elle-cli` consumes it directly. The one addition over a naive port is that
**`time` carries its provenance**, because a bounded-liveness oracle must know whether it is reading
a virtual or a wall-clock timestamp.

```rust
pub enum Clock { Virtual, Wall }                  // provenance is part of the type

pub struct OpEntry {
    pub index:   u64,
    pub time:    Time,                            // value + Clock tag
    pub kind:    OpKind,                          // Invoke | Ok | Fail | Info
    pub process: ProcessId,
    pub f:       Function,                        // :txn :read :write …
    pub value:   Value,                           // op payload / return value
}

pub struct History(pub Vec<OpEntry>);

impl History {
    pub fn to_jepsen_edn(&self)  -> String;       // default for the Elle path (fewer edge cases)
    pub fn to_jepsen_json(&self) -> String;       // generic interop
    pub fn from_jepsen(s: &str)  -> Result<History, ParseError>; // ingest a Jepsen store/ history
}
```

Two things designed in from day one:

1. **`time` carries its provenance.** A virtual timestamp from the sim and a wall-clock timestamp
   from Jepsen are both valid `OpEntry::time` values, but a bounded-liveness oracle must know which
   it is reading ([§14](#14-the-cross-backend-pipeline)). Tag it.
2. **EDN is the safer wire for Elle.** `elle-cli` notes that JSON→Clojure conversion "may fail" in
   some cases and recommends [`jet`](https://github.com/borkdude/jet) as a JSON↔EDN fallback. We emit
   EDN by default for the Elle path and keep JSON for generic interop; `jet` is an optional shim, not
   a hard dependency.

This type is the **narrow waist** of the whole design: it is what lets every oracle be written once
and run on either executor, and it is independently useful to any Rust project that wants to read or
emit Jepsen-shaped histories. That independent value is why it earns its own crate
([§17](#17-packaging-a-crate-family-by-stability-boundary)).

---

## 11. Worked example: failure recovery within a deadline

This is the canonical use case — *"recovers from failure after a certain event, within a set time"* —
end to end, on the deterministic executor.

```rust
use propsim::prelude::*;

fn gossip_plan() -> TestPlan<GossipNode> {
    Simulation::plan()
        .nodes(7)
        .transport(InMemory::unordered_lossy())
        .state_machine::<GossipNode>()
        .workload(updates())                        // clients publish values to random nodes
        .faults(
            Faults::scripted()                      // a specific, not random, scenario
                .at(secs(1)).partition(&[0, 1, 2], &[3, 4, 5, 6])
                .at(secs(3)).heal_all()
                .mode(Mode::Liveness),
        )
        .check([
            // Safety (white-box): a node never reports a value that was never published.
            property::always("no fabricated values", |w: &World<GossipNode>| {
                let published: HashSet<_> = w.history().published_values().collect();
                w.nodes().flat_map(|n| n.known_values()).all(|v| published.contains(&v))
            }),
            // Liveness (white-box): within 2s of the heal, every node holds every published value.
            property::eventually_within(
                "full convergence within 2s of heal",
                secs(2),
                |w: &World<GossipNode>| {
                    let all: HashSet<_> = w.history().published_values().collect();
                    w.nodes().all(|n| n.known_values().collect::<HashSet<_>>() == all)
                },
            ).after(Event::NetworkHealed),
        ])
        .seeds(1_000)
        .finish()
}

#[test]
fn gossip_reconverges_after_partition() {
    gossip_plan().run(Backend::deterministic());
}
```

What the harness does with this: for each of 1,000 seeds it builds the cluster, drives generated
updates while honoring the scripted partition/heal timeline, advances virtual time (compressing the
idle 2-second convergence window into microseconds of wall time), and after the heal checks that
convergence is reached before the deadline. The safety property is checked in *every* state along the
way. A violation of either prints a seed and a shrunk trace.

Because time is virtual, the 2-second deadline costs nothing to test, and you can sweep deadlines,
cluster sizes, and loss rates cheaply — the same lever that lets simulation testing turn
[an hour of wall time into a month of coverage](https://tigerbeetle.com/blog/2023-07-11-we-put-a-distributed-database-in-the-browser/).

**Backend note:** both properties here are white-box (`Needs::World`), so this plan runs on
`deterministic()` and `rigorous()` but is *rejected at `run()`* under `jepsen(cluster)` with a named
offender — exactly the negotiation in [§9.2](#92-capability-negotiation-is-a-runtime-check-by-deliberate-choice).
To exercise this protocol on a real cluster you would add a history-based oracle and target *that*.

---

## 12. The determinism contract

`propsim`'s deterministic executor is only as deterministic as the nondeterminism you let it own.
The contract (and the scope: this table is about the **`DeterministicExecutor`** specifically):

| Source            | Style A (`Node`)                         | Style B (real code over `Transport`)                                  |
|-------------------|------------------------------------------|-----------------------------------------------------------------------|
| Network I/O       | Controlled (only `cx.send`/`broadcast`)  | Controlled (in-memory transport)                                      |
| Time              | Controlled (`cx.now()` only)             | Virtualized by interception (libc shim); residual risk on un-intercepted paths |
| Randomness        | Controlled (`cx.rng()` only)             | Virtualized by interception (libc shim); residual risk on un-intercepted paths |
| Thread scheduling | N/A (single-threaded by construction)    | Must run on the harness runtime; no escaping OS threads               |
| `HashMap` order   | Use a seeded hasher or `BTreeMap`        | Same — iteration order seeds from OS entropy per process              |

The last two rows are the usual culprits behind "deterministic except in CI" and are called out
explicitly because every team rediscovers them (see the
[turmoil determinism notes](https://docs.rs/turmoil/latest/turmoil/) and S2's
[`mad-turmoil`](https://s2.dev/blog/dst)). The "residual risk" cells are not hand-waving: they are
exactly what the [determinism-regression guard](#14-the-cross-backend-pipeline) makes visible. If you
cannot satisfy this contract and cannot restructure, this is not the right tool — and that is a
legitimate reason to fall back to integration testing or the `jepsen(cluster)` backend, which does
not need it.

---

## 13. Adoption cost: what you trade

An earlier draft claimed Style B needs "zero or near-zero code changes." That is true only in the
best case, and honesty matters more than the pitch: **adoption cost is dominated by how your
system-under-test was already written, not by `propsim`.** If your code already injects a clock, an
RNG, and a transport (dependency inversion), churn really is near-zero. If it reaches for ambient
`Instant::now()`, `rand::thread_rng()`, `tokio::spawn`, and `tokio::net::*` directly at every call
site, churn is moderate. Beneath the churn is a structural tension you cannot escape:

> **You cannot simultaneously have all three of: (1) zero code change, (2) total determinism, and
> (3) reuse of the real implementation.** Interception buys (1)+(3) and pays in (2). Clean dependency
> injection buys (2)+(3) and pays in (1). A deterministic hypervisor
> ([Antithesis](https://antithesis.com/docs/resources/deterministic_simulation_testing/)) buys all
> three but is not an in-process library. `propsim`'s primary path (Style B + interception)
> deliberately picks **reuse**, and accepts "very high, not total" determinism as the price.

This tension bites hardest exactly where you most want reuse. Running the real iroh stack is great
for reuse — and it is also the code whose internal time/entropy reads you cannot reach from the
outside without interception. Reuse and controllability pull against each other; the design picks
reuse and then works to shrink the uncontrolled remainder.

### 13.1 What it costs (one-time, mechanical)

- **Network substitution.** For iroh: a constructor argument (the custom-transport seam), not a
  refactor. For a generic tokio codebase: the `cfg`-module pattern — a `net` module that swaps
  `tokio::net` for simulated types under a feature flag, then routing every networking call site
  through it. Pervasive but shallow churn, proportional to how many sites touch the network.
- **Runtime substitution.** Your code runs on the harness executor. With `madsim` that means building
  under `--cfg madsim` and using madsim-compatible versions of async deps; if a dependency isn't
  supported you fork it or you are blocked. This is the real-world tax of the "no code change"
  runtimes, and it should be checked against your dependency tree *before* committing.
- **Your own ambient calls.** Each `Instant::now()`, `SystemTime::now()`, `thread_rng()`, and raw
  `std::thread::spawn` at your call sites becomes a call through a harness handle. Mechanical, but
  this is the actual refactor surface for the parts of Style B that interception does not cover.

### 13.2 What it costs (ongoing, as discipline)

- A **determinism-regression guard** in CI ([§14](#14-the-cross-backend-pipeline)) — because the
  contract degrades silently.
- `HashMap`-ordering hygiene forever (seeded hashers or `BTreeMap`).
- **Dependency risk:** a routine version bump can reintroduce nondeterminism you did not write and
  cannot see without the guard.

### 13.3 Realistic adoption tiers

| Your code today                                                                 | Expected churn |
|---------------------------------------------------------------------------------|----------------|
| Already injects clock + RNG + transport; deps are simulation-friendly           | **Near-zero** — wire up the harness |
| Real iroh protocol over the custom-transport seam, internals via interception   | **Low** — a transport adapter + the determinism guard; iroh internals covered by the shim |
| A focused protocol with ambient `now()`/`thread_rng()` calls, cooperative deps  | **A few days** — route your own ambient calls; lean on interception for deps |
| Leans on ambient nondeterminism in unforkable deps, or is multi-threaded by design | **High or infeasible** — this is the "use a different tool" case |

### 13.4 What you give up

- **A boundary you respect forever.** Once testable, IO/time/randomness must keep flowing through the
  seams; new code obeys or the harness rots. (Framed positively: this is just good architecture.)
- **Total determinism**, on the reuse-maximizing path, until the gaps are closed or upstream
  injection seams land.
- **Build complexity:** feature flags, possibly patched dependency versions, longer CI.
- **An added test layer, not a replacement.** Fidelity is bounded by the faults you modeled; real
  wire/NAT/kernel bugs will not surface on the simulator. You keep integration tests / `netsim` /
  real deployment — or reach for the `jepsen(cluster)` backend.
- **Single-threaded execution semantics.** If correctness genuinely depends on multi-core
  parallelism, the simulation diverges from production.

---

## 14. The cross-backend pipeline

The reason to unify these backends under one plan is a concrete pipeline that no single tool gives
you:

```
  deterministic()           rigorous()                    jepsen(cluster)
  ───────────────           ──────────                    ───────────────
  Find a violation     →    Confirm it is a REAL      →   Reproduce it on the
  in milliseconds,          consistency anomaly,          REAL binary over a
  shrink to a minimal       not a propsim modeling        real network, for a
  scenario, replay          artifact (Elle is sound       production-observable
  from an 8-byte seed.      and points to the witness     bug report.
                            transactions).
```

### 14.1 A real API handle: `Scenario`

The seed reproduces the *deterministic executor only*. The portable debugging artifact across the
seam is a **`Scenario`**: the frozen, already-shrunk op stream + fault schedule.

```rust
pub struct Scenario { /* concrete op stream + fault schedule; serde */ }

impl RunFailure { pub fn scenario(&self) -> &Scenario; }          // from try_run on the sim
impl Scenario   { pub fn save(&self, path: &Path) -> io::Result<()>;
                  pub fn load(path: &Path) -> io::Result<Scenario>; }
impl<N> TestPlan<N> { pub fn replay(self, s: Scenario) -> Self; }  // pin the workload+faults
```

```rust
// 1) cheap: find + shrink on the deterministic executor
let failure = plan().try_run(Backend::deterministic()).unwrap_err();
failure.scenario().save("ce.scenario".as_ref())?;

// 2) confirm it is a real anomaly, still no cluster
plan().replay(Scenario::load("ce.scenario".as_ref())?)
      .run(Backend::rigorous());

// 3) expensive: reproduce the *same* scenario on a real binary, for a production bug report
plan().replay(Scenario::load("ce.scenario".as_ref())?)
      .run(Backend::jepsen(ClusterSpec::aws_5_node()));
```

This makes two honesty constraints *enforceable* rather than aspirational:

1. **The seed reproduces the deterministic executor only.** On Jepsen the seed controls Jepsen's
   *choices*, not the cluster's timing. We document the seed as backend-local and never imply
   cross-backend seed reproduction; the `Scenario` is what crosses the seam. **Shrink cheap,
   reproduce expensive.**
2. **Bounded-time liveness means different things per executor.** "Reconverges within 5s" over a
   *virtual-time* history is exact and compressible — the 5s costs microseconds and you can sweep
   deadlines cheaply. Over a *real* Jepsen history the same predicate measures wall-clock recovery on
   real machines: meaningful but noisy, and not free. Because `OpEntry::time` carries its provenance
   ([§10](#10-the-interchange-format-jepsens-history-shape)), the oracle annotates its verdict
   accordingly — *"deadline evaluated against virtual time"* vs *"… against wall-clock; subject to
   machine jitter"* — instead of silently comparing incomparable clocks.

### 14.2 The determinism-regression guard

Because the reuse path's determinism is best-effort, the gap must be **visible** — a named failing
test, not a flaky CI run discovered months later. `propsim` ships this as a first-class check in the
prelude: run the same seed twice and assert the recorded event trace is byte-identical.

```rust
// Ships in the prelude — run on every CI build.
#[test]
fn simulation_is_deterministic() {
    // If a dependency starts reading wall-clock time or OS entropy through an un-intercepted
    // path, this fails immediately and points at the first divergent event.
    propsim::assert_deterministic(plan(), Seed(0xC0FFEE));
}
```

It converts "deterministic except in CI" from a debugging nightmare into a failing test with a
pointer to the offending event — and it is the standing evidence that a reuse-maximizing setup is
*still* sound after a dependency bump. It does not make determinism total; it makes the residual gap
**observable**, which is what makes the trade safe to live with.

### 14.3 Determinism ergonomics

```rust
pub struct Seed(pub u64);                      // Display/FromStr round-trip the 0x… form
// PROPSIM_SEED=0x9f3a17c4e2b01a55 cargo test fast        -> replay one seed (deterministic exec)
// PROPSIM_SCENARIO=ce.scenario     cargo test full       -> replay an exported Scenario (§14.1)
```

These mirror the `PROPTEST_*` env idioms so the muscle memory transfers. A `tracing` integration
ships too: events are stamped with **virtual** time, so logs are reproducible alongside the trace.

```rust
let _g = propsim::trace_to(tracing_subscriber::fmt());   // virtual-time-stamped, reproducible logs
```

---

## 15. Relationship to neighboring tools

`propsim` deliberately occupies an empty spot. Here is how to place it.

| Tool                                                            | Layer it owns                                  | Why `propsim` is different                                                                 |
|----------------------------------------------------------------|------------------------------------------------|-------------------------------------------------------------------------------------------|
| [`proptest`](https://github.com/proptest-rs/proptest)          | Input generation + shrinking                   | We **use** it as the front-end; `propsim` adds the network, clock, faults, and oracles.   |
| [`turmoil`](https://github.com/tokio-rs/turmoil) / [`madsim`](https://github.com/madsim-rs/madsim) | Deterministic in-memory runtime | Those are *runtimes*; they don't bring property generation or a protocol oracle. We can sit on one. |
| [`stateright`](https://github.com/stateright/stateright)       | Exhaustive model checking of a Rust model      | We **sample** (scales to realistic state) and test the **real implementation**, not a model; and we have a quantitative virtual clock for bounded-time liveness. |
| [`loom`](https://github.com/tokio-rs/loom) / [`shuttle`](https://github.com/awslabs/shuttle) | Thread interleavings of one node | Different axis: single-node concurrency, not the distributed protocol. Complementary.     |
| [Jepsen](https://github.com/jepsen-io/jepsen)                  | Faults against a *real cluster*                | We run in-process and deterministically by default — *and* we can drive Jepsen itself as our high-fidelity backend; reproduction on the sim is a seed, on the cluster a `Scenario`. |
| [Elle](https://github.com/jepsen-io/elle)                      | Sound transactional-anomaly checking of a history | We **adopt** it as the rigorous oracle over our own deterministic history — no cluster required. |
| iroh [`netsim`](https://github.com/n0-computer/iroh)           | Realistic netns simulation                     | That is the high-fidelity, slow outer loop; `propsim`'s sim is the millisecond inner loop. |

**Closest prior art.** The nearest existing thing to this whole stack is
**[TickLoom](https://github.com/unmeshjoshi/tickloom)** — a single-threaded tick-loop DST framework
*with Jepsen integration* — but it is **Java**, and history-checker-only. `propsim` is the
Rust-native, `proptest`-driven take with a real virtual clock and bounded-time liveness as a
first-class oracle. The other live reference point for "DST framework + Jepsen checker" is listed
among Jepsen's related projects: <https://github.com/jepsen-io/jepsen>.

The intended workflow is **layered, not exclusive**:
**`stateright`/TLA+ for design-level confidence on small instances → `propsim` `deterministic()` for
the implementation at scale under virtual time → `propsim` `rigorous()` to confirm anomalies are real
→ `propsim` `jepsen(cluster)` / `netsim` / real deployment for wire-level fidelity.**

---

## 16. Reconciliations

This document supersedes three earlier drafts. The substantive resolutions, for reviewers who knew
them:

1. **One builder shape: `plan() … finish() → run(backend)`.** The earliest draft's terminal
   `Simulation::builder() … .check(&[…]).run()` — written before backends existed — is retired. The
   two-phase surface is the only shape that makes "one definition, swap the backend" literal.
2. **Node-definition verbs split.** The overloaded `.build::<N>()` is replaced by
   `.state_machine::<N>()` (Style A) and `.spawn_each(|ep| …)` (Style B); "build" now denotes backend
   validation, nothing else.
3. **Capability negotiation stays runtime**, an explicit choice over compile-time typestate, to
   preserve the one-line backend swap and to fail with a sentence instead of a trait-bound wall.
4. **`Scenario` is first-class**, giving the find→confirm→reproduce pipeline an actual API handle and
   making "the seed is backend-local; the `Scenario` crosses the seam" enforceable rather than merely
   documented.
5. **`Clock` provenance is part of `OpEntry::time`**, making "never compare virtual to wall-clock"
   structural.
6. **`Seed` newtype + `PROPSIM_SEED` / `PROPSIM_SCENARIO`**, with `assert_deterministic` and a
   virtual-time `tracing` layer shipped in the prelude.
7. **Packaging is grouped by stability boundary** (next section), refining an earlier 9-crate sketch:
   `propsim-history` is elevated as independently valuable, the `unsafe` interception is isolated in
   its own crate, and a no-default-features litmus test is made the invariant.

---

## 17. Packaging: a crate family by stability boundary

**A crate family, not a single package — but split on *stability boundaries*, not on every functional
seam.** Three tiers, so a pure-Rust user **never** pulls a JVM, a Go binary, a cluster, or `unsafe`.

### Tier 1 — the stable contract (aim for 1.0 early; change rarely)

| Crate | Contents | Deps |
|---|---|---|
| `propsim-history` | `OpEntry`, `History`, Jepsen EDN/JSON serde, `Clock` provenance | `serde` (optional), nothing else |
| `propsim-core` | the plugin traits (`Executor`, `Oracle`, `Transport`, `Node`), `Produces`/`Needs`, `RunOutput`, `Verdict`, `Backend` + negotiation, `Property`/`World`, `Faults`/`NetworkModel`, `Seed`, `Scenario` | `propsim-history`, `proptest` |

These are what an **external** plugin author compiles against. `propsim-history` is split out (rather
than folded into core) because it has standalone value: any Rust project — not just propsim — may
want to read or emit Jepsen-shaped histories. This is the `http`-crate pattern (a tiny shared type
crate under `hyper`/`reqwest`/`axum`): <https://github.com/hyperium/http>.

### Tier 2 — the batteries-included engine (faster cadence; pre-1.0 longer is fine)

| Crate | Contents | Deps |
|---|---|---|
| `propsim` (**facade**) | prelude, `Simulation::plan()`, `Backend` presets, **re-exports `propsim-core`**, bundles the default deterministic executor + native oracles | core + the three below |
| `propsim-sim` | `DeterministicExecutor`: single-thread scheduler, virtual clock, seeded RNG, in-mem `Transport`, `Node`/`Transport` seams | core |
| `propsim-oracle` | native invariant / reachability / bounded-liveness + native (Porcupine-style) linearizability | core, history |
| `propsim-intercept` | the libc `clock_gettime`/`getrandom` shim for Style B — **the only crate with `unsafe`**, platform-gated | libc |

`propsim` is what ~90% of users depend on, and with **default features it is pure Rust**: no JVM, no
Go, no cluster. The facade re-exports core types so users never name `propsim-core` directly (which
keeps them off the version-coupling treadmill); plugin authors depend on `propsim-core` directly.

Isolating the `unsafe` interception in its own platform-gated crate lets *everything else*
`#![forbid(unsafe_code)]` — the unsafe blast radius is one small, separately-auditable crate, and the
Style-A-only user never compiles it at all.

> Whether `propsim-sim`/`propsim-oracle` stay separate crates or become modules *inside* the facade
> is a minor internal call (they share a cadence and have no external deps). The non-negotiable part
> is that the **traits live in `-core`**, so an out-of-tree oracle does not have to pull the whole
> engine.

### Tier 3 — opt-in heavy adapters (a `cargo build` default never reaches these)

| Crate | Feature | Pulls in |
|---|---|---|
| `propsim-iroh` | `iroh` | iroh `Transport` adapter |
| `propsim-elle` | `elle` | Elle bridge — **JVM + `elle-cli` jar** |
| `propsim-jepsen` | `jepsen` | Jepsen executor — **JVM + cluster + db/os plugin** |
| `propsim-porcupine` | `porcupine` | standalone Go Porcupine binary |

Users opt in through facade features (`propsim = { features = ["elle"] }`), which transitively pull
the adapter crate — the "one dependency, turn on what you need" UX, with the dependency graph still
telling the truth. This is exactly how `serde`'s `derive` feature pulls `serde_derive`
(<https://github.com/serde-rs/serde>) and how `tokio` gates functionality
(<https://github.com/tokio-rs/tokio>).

`propsim-iroh` being a *separate, optional* crate is not just hygiene — it **enforces** the principle
that "iroh is an adapter, not an assumption" ([§20 Q2](#20-open-questions)). The crate boundary makes
the build system police the abstraction: iroh cannot leak into core because core does not depend on
it.

### 17.1 Why not a single package

- **Heavy/foreign deps must be opt-in.** A JVM (Elle, Jepsen) or a Go binary (Porcupine) in the
  default dependency closure of a Rust testing crate is a non-starter. Cargo features alone within
  one crate can gate *compilation*, but a separate-crate boundary is what keeps the heavy code out of
  `cargo doc`, `cargo audit`, and the lockfile for the pure-Rust user.
- **External plugin authors need a lightweight trait crate.** The entire trait architecture exists so
  third parties can add executors/oracles. If the traits lived in the facade, an out-of-tree oracle
  would depend on the whole sim engine to get `trait Oracle`. The facade/core split is the
  `tracing` / `tracing-core` (<https://github.com/tokio-rs/tracing>) and `axum` / `axum-core`
  (<https://github.com/tokio-rs/axum>) pattern, and it is the right call here.

### 17.2 Why not the maximal split either

Every **independently versioned** crate that shares types is a release-coordination tax and a
version-skew hazard: a breaking change in `-core` forces a synchronized bump everywhere, and users
can land in "mismatched `-core`" hell. So the split tracks **stability**: a conservative,
aim-for-1.0-early contract (`-history`, `-core`) vs. a faster-moving engine (the facade and its
sub-crates) vs. genuinely-optional adapters. Functional seams that share a cadence and have no
external deps (sim, native oracles) do **not** each deserve their own SemVer timeline.

### 17.3 The litmus test

> `cargo test` with **default features** runs the full deterministic + native-oracle inner loop on a
> laptop and in plain CI, pulling **no JVM, no Go, no cluster, and no `unsafe` unless Style B is
> used**. If a change breaks that invariant, the change is wrong.

### 17.4 SemVer footnotes worth writing down now

- `proptest` is part of `propsim-core`'s public API (the `Strategy` you pass to `.workload`), so a
  `proptest` major bump implies a `propsim-core` major bump. That coupling is the price of the
  deliberate "reuse proptest as the front-end" decision and is acceptable — but document it.
- Keep `-history`/`-core` `#![forbid(unsafe_code)]` and on a slow cadence; let the facade carry the
  churn. Re-export aggressively from the facade so application code imports one crate.

---

## 18. Architecture sketch

```
┌──────────────────────────────────────────────────────────────────────────┐
│ Test (#[test] fn)                                                          │
│   Simulation::plan() … .check([properties]).finish()  ──►  .run(backend)   │
└───────────────┬───────────────────────────────────────────────────────────┘
                │ one backend-neutral TestPlan
┌───────────────▼───────────────────────────────────────────────────────────┐
│ Backend = one Executor + N Oracles   (capability negotiation runs first)   │
└───────┬───────────────────────────────────────────────┬───────────────────┘
        │ Executor::run(plan, seed)                      │ Oracle::check(out)
┌───────▼────────────────────────────┐         ┌─────────▼──────────────────────┐
│ Executors                          │         │ Oracles                        │
│  • DeterministicExecutor (core):   │         │  • always/sometimes/eventually │
│      virtual clock · seeded RNG ·  │  Run-   │      (Needs::World)            │
│      event queue · FaultSchedule · │  Output │  • native linearizability      │
│      InMemory / IrohInMemory ◄─────┼─ real ─ │      (Porcupine-style)         │
│      Transport · libc interception │  iroh   │  • Elle bridge (Needs::History)│
│  • JepsenExecutor (real cluster)   │  ─────► │  • Porcupine bridge (optional) │
└───────┬────────────────────────────┘         └────────────────────────────────┘
        │ produces                                          ▲ consumes
        └──────────────────►  History (Jepsen-shaped; the narrow waist)  ◄───────┘
                              + Option<WorldTrace> (white-box, sim only)
┌────────────────────────────────────────────────────────────────────────────┐
│ Nodes:  Style A `Node` impls (.state_machine)  OR  Style B real handlers     │
│         (.spawn_each — iroh, …) over a Transport                             │
└──────────────────────────────────────────────────────────────────────────────┘
```

**Build-vs-reuse decisions** (with rationale from the survey):

- **Front-end:** reuse `proptest`. Don't reinvent shrinking; it is settled community consensus.
- **Property vocabulary & network taxonomy:** borrow from `stateright`.
- **Interchange:** adopt Jepsen's history shape; do not invent a format.
- **Runtime core:** the primary path is real-async code over `turmoil`/`madsim` with libc-level
  interception (Style B — the iroh case and most adopters); accept that determinism is very-high, not
  total, and ship the determinism-regression guard to keep the gap observable. The state-machine core
  (Style A) is offered for greenfield protocols that adopt that architecture deliberately, where it
  gives total determinism with no interception and no reuse to lose.
- **Oracle:** reuse/port Porcupine's algorithm (not the slower Knossos approach) as the native
  default; shell out to Elle for sound transactional checking.
- **Fault diversity & reproduction:** swarm testing + seed replay (sim) / `Scenario` replay (cluster),
  treated as first-class output.

---

## 19. Phasing

Don't innovate on too many axes at once.

- **Phase 1 (ships first, no cluster anywhere):** `DeterministicExecutor` + native oracles + the
  **Elle bridge** + the `History` interchange + the determinism-regression guard. This already
  delivers `deterministic()` and `rigorous()` — the full "Elle" half of "Jepsen and/or Elle" — with
  nothing heavier than an `elle-cli` jar. It is the highest value-to-cost ratio in the whole design.
- **Phase 2:** the `JepsenExecutor` (data-driven EDN runner + JVM orchestration + `store/` ingestion
  + the `db`/`os` plugin seam). This is the heavy, lower-leverage half; gating it behind a later
  phase keeps Phase 1 honest and shippable.
- **Later:** additional executors (`netsim` outer loop, `turmoil`/`madsim` variants) and oracles
  (Elle predicate checking, coverage-guided exploration) — all additive behind the same two traits.

---

## 20. Open questions

1. **Default runtime for Style B (the load-bearing fork).** Build `propsim-sim`'s Style-B runtime on
   [`turmoil`](https://github.com/tokio-rs/turmoil) (Tokio-native, simpler) or
   [`madsim`](https://github.com/madsim-rs/madsim) (intercepts time/entropy, fewer determinism gaps)
   — or feature-gate both? This needs a spike against the real iroh custom-transport API
   (<https://github.com/n0-computer/iroh/issues/3848>) before the `propsim-sim` ⇄ `propsim-intercept`
   boundary can be frozen, because "how much the runtime already virtualizes" determines how much the
   libc shim must cover. The cautionary history matters here: tokio's own `tokio-rs/simulation` was
   abandoned and replaced by the explicitly "experimental" turmoil precisely because Tokio was not
   designed for determinism
   (<https://notes.eatonphil.com/2024-08-20-deterministic-simulation-testing.html>).
2. **How much to lean on the iroh seam vs. a generic `Transport`.** Keeping the core trait thin is
   what preserves general-purpose-ness; iroh must remain *an adapter* (its own optional crate), not
   an assumption.
3. **Liveness deadlines under swarm faults.** A bounded-liveness property is only meaningful once the
   scheduler guarantees a healthy window; the API must make "this fault config can never heal" a loud
   error rather than a silently-vacuous pass.
4. **Whole-system vs. subsystem testing.** Per TigerBeetle's
   [*Tale of Four Fuzzers*](https://tigerbeetle.com/blog/2025-11-28-tale-of-four-fuzzers/), allow
   targeting one protocol in isolation so deep layers are actually exercised.
5. **Interception vs. upstream injection.** The libc shim maximizes reuse but caps determinism at
   "very high." Persuading iroh/noq to accept an injected `Clock`/`Rng` would make it total, at the
   cost of an upstream contribution effort. Worth pursuing, tracked separately; until then the
   determinism-regression guard keeps the residual gap honest.
6. **EDN vs JSON to Elle by default.** EDN avoids the documented JSON→Clojure conversion edge cases
   but couples us to an EDN writer; JSON is the generic interop format. *Lean:* EDN for the Elle path,
   JSON for everything else, [`jet`](https://github.com/borkdude/jet) as an optional shim.
7. **Bundle vs discover `elle-cli`.** Bundling a pinned jar makes `rigorous()` work out of the box
   but ships a JVM artifact in a Rust crate; discovery is lighter but adds setup friction. *Lean:*
   discover-with-helpful-error, plus a documented `cargo xtask fetch-elle`.
8. **How thin can the fixed Jepsen runner be?** Find the minimal EDN spec that covers topology +
   frozen workload + nemesis schedule + checker selection, and push everything system-specific into
   the `db`/`os` plugin — without re-inventing Jepsen's test map.
9. **Native checker vs always-Elle.** If the Elle bridge is robust, is the in-Rust Porcupine-style
   checker still worth maintaining? *Lean:* keep it as the zero-dependency, no-JVM default for
   register linearizability; treat Elle as the rigorous upgrade.
10. **Antithesis as a third executor.** Jepsen-inside-Antithesis is the one way to get *deterministic*
    real-binary execution. Out of scope for now (paid, out-of-process), but it fits the `Executor`
    trait cleanly if demand appears.

---

## 21. References (prior art)

**Property-based testing**
- proptest — <https://github.com/proptest-rs/proptest>
- proptest-stateful (Readyset) — <https://github.com/readysettech/proptest-stateful>
- quickcheck / arbtest / test-strategy overview — <https://rustprojectprimer.com/testing/property.html>
- J. Hughes, *Testing the Hard Stuff and Staying Sane* — <https://www.cs.tufts.edu/~nr/cs257/archive/john-hughes/quviq-testing.pdf>
- `quickcheck-dynamic` (dynamic logic / reachability) — <https://github.com/input-output-hk/quickcheck-dynamic>
- PBT frameworks overview — <https://github.com/jmid/pbt-frameworks>

**Deterministic simulation testing**
- sled simulation guide — <http://sled.rs/simulation.html>
- turmoil — <https://github.com/tokio-rs/turmoil>
- madsim — <https://github.com/madsim-rs/madsim>
- S2, *Deterministic simulation testing for async Rust* — <https://s2.dev/blog/dst>
- Polar Signals, *A Theater of State Machines* — <https://www.polarsignals.com/blog/posts/2025/07/08/dst-rust>
- FoundationDB / Will Wilson (Strange Loop 2014) — <https://www.youtube.com/watch?v=4fFDFbi3toc>
- TigerBeetle VOPR — <https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/vopr.md>
- TigerBeetle, simulation testing for liveness — <https://tigerbeetle.com/blog/2023-07-06-simulation-testing-for-liveness/>
- TigerBeetle, *We Put a Distributed Database in the Browser* — <https://tigerbeetle.com/blog/2023-07-11-we-put-a-distributed-database-in-the-browser/>
- TigerBeetle, *A Tale of Four Fuzzers* — <https://tigerbeetle.com/blog/2025-11-28-tale-of-four-fuzzers/>
- P. Helland-style overview / eatonphil — <https://notes.eatonphil.com/2024-08-20-deterministic-simulation-testing.html>
- awesome-DST — <https://github.com/ivanyu/awesome-deterministic-simulation-testing>
- Antithesis (deterministic hypervisor) — <https://antithesis.com/docs/resources/deterministic_simulation_testing/>

**Model checking & concurrency**
- stateright — <https://github.com/stateright/stateright>
- loom — <https://github.com/tokio-rs/loom>
- shuttle (AWS) — <https://github.com/awslabs/shuttle>

**Checking / oracles**
- Jepsen — <https://github.com/jepsen-io/jepsen> · stated tradeoffs (opaque-box, nondeterministic) — <https://jepsen.io/analyses>
- Elle (repo + paper) — <https://github.com/jepsen-io/elle> · <https://arxiv.org/abs/2003.10554>
- `elle-cli` (the wrapper we shell out to) — <https://github.com/ligurio/elle-cli>
- `history.sim` (deterministic Jepsen histories; EDN/JSON emission for cross-language checkers) — <https://github.com/jepsen-io/history.sim>
- Porcupine (linearizability) — <https://github.com/anishathalye/porcupine>
- Knossos (Jepsen's original linearizability checker) — <https://github.com/jepsen-io/knossos>
- `jet` (JSON↔EDN shim) — <https://github.com/borkdude/jet>
- Elle / transactional anomalies (VLDB) — <http://www.vldb.org/pvldb/vol14/p268-alvaro.pdf>
- aphyr, *Strong consistency models* — <https://aphyr.com/posts/313-strong-consistency-models>
- S2, linearizability from DST — <https://s2.dev/blog/linearizability>

**Principled fault selection**
- Swarm testing (Groce et al., ISSTA 2012) — <https://www.flux.utah.edu/paper/groce-issta12>
- Lineage-driven fault injection (Alvaro et al., SIGMOD 2015) — <https://mwhittaker.github.io/papers/html/alvaro2015lineage.html>
- Model-guided fuzzing of distributed systems — <https://arxiv.org/pdf/2410.02307>

**Closest prior art / positioning**
- TickLoom (single-threaded tick-loop DST framework + Jepsen integration; Java) — <https://github.com/unmeshjoshi/tickloom>
- DST-framework + Jepsen-checker projects, listed among Jepsen's related projects — <https://github.com/jepsen-io/jepsen>

**Packaging precedents**
- core/facade split: tracing/tracing-core — <https://github.com/tokio-rs/tracing> · axum/axum-core — <https://github.com/tokio-rs/axum>
- features pulling sub-crates: serde/serde_derive — <https://github.com/serde-rs/serde> · tokio — <https://github.com/tokio-rs/tokio>
- tiny shared interchange crate: http — <https://github.com/hyperium/http>

**Target system**
- iroh — <https://github.com/n0-computer/iroh>
- iroh custom transports (issue) — <https://github.com/n0-computer/iroh/issues/3848>