# ADR 0021 - Build time: line-table debuginfo now, an mc-core split later

**Date:** 2026-11-05  
**Status:** Step 1 accepted by the owner and implemented (2026-11-05). Step 2
(settlements crate pilot) designed and not started: the design recommends
against it at today's coupling. Step 3 root-only feature unification passes
workspace build, formatting and code-health; consumer test scope is below.
Comparative timings await Main's batched quiet window.
Leaf extraction is implemented and under acceptance; cycle cuts have not landed.
The cycle ratchet
(owner-approved 2026-11-07, implemented) makes code-health fail when the module
cycle grows.
**Related:** [0011](0011-core-crate-topology-and-threading.md) (rejected
one-crate-per-domain without an enforceable boundary),
[0013](0013-core-model-v2.md) (current crate ownership),
[0017](0017-tick-system-graph.md) (declared tick systems).

## Problem

Agents spend most build time in the edit loop, not in cold builds. Quiet-window
measurements on tree `e085b815` follow, all with a 6-core quota, incremental
on, mold, no sccache, and one run each. Receipts are in
`.analysis/codex-logs/build-time/20261105T115219Z/`.

| Scenario | Wall time |
|---|---|
| Cold server binary (`-p mc-edge --bin solaris-v2`) | 6m32s |
| Cold `mc-core` unit-test binary, after the above | 4m22s |
| One-line `mc-core` edit → server binary | 59–71 s |
| One-line `mc-core` edit → `mc-core` unit-test binary | 76–99 s |
| One-line `mc-edge` edit → server binary | 8 s |

- **`mc-core` is one crate of about 400k lines**: about 201k production and
  199k in `*_tests.rs`.
- **Incremental compilation barely helps it.** A trivial edit recompiles the
  `mc-core` lib in 42 s against 55 s cold. `mc-edge` downstream then rebuilds in
  22–28 s against 22 s cold.
- **Where a cold `mc-core` compile goes:** about 18 s front end (type and
  borrow checking), 9 s metadata, 7 s monomorphization and about 30 s
  codegen/LLVM.
- **A parallel front end gave nothing:** with `-Z threads=8`, the build took
  60 s.
- **Build and test configurations share no workspace artifacts.**
  `cargo build -p mc-edge`, `cargo build -p mc-core` and
  `cargo test -p mc-core` resolve different features for low-level
  dependencies, among them `bitflags`, `log`, `smallvec`, `digest` and
  `typenum`. The first test build after a change to a lower crate therefore
  rebuilds `mc-data`, `mc-world`, `mc-worldgen`, `mc-script` and
  `mc-plugin-host` again, about 3–4 minutes. Cargo's
  `resolver.feature-unification` is unstable on 1.99.
- **`mc-core`'s modules are one dependency cycle.** 189 of its 197 top-level
  modules (185k of 201k lines) are one strongly connected component; `tick`
  alone reaches 148 modules. The first version of this ADR said "of 421": that
  was the number of top-level file and directory names in `src/`, 194 test
  files included. The line figures are raw lines: 201k of every
  non-`*_tests.rs` file, 185k of each cycle module's `<m>.rs` and `<m>/` files,
  inline tests included. The cycle ratchet below counts by the module tree,
  tests excluded: 188k of 195k.

## Step 1 (implemented): line-table debuginfo

`[profile.dev] debug = "line-tables-only"`. The test profile inherits it, and
the per-command `CARGO_PROFILE_*_DEBUG` overrides are gone from the tooling
guide. Panics and backtraces keep file:line. Debuggers lose local variables.
Receipts: `.analysis/codex-logs/build-time/20261105T123032Z-line-tables/`.

| Scenario | Before | After |
|---|---|---|
| Cold server binary | 6m32s | 4m19s |
| `mc-core` edit → server binary | 59–71 s | 43–58 s |
| `mc-core` edit → `mc-core` test binary | 76–99 s | 60–71 s |
| `mc-edge` edit → server binary | 8 s | 16 s |
| Cold `mc-core` lib alone | 55 s | 42 s |
| Peak RSS of the cold build | 4.1 GB | 3.0 GB |

The result is mixed. Every `mc-core` path and the cold build got faster. The
binary crate's share of an `mc-core` edit fell from 13–22 s to 2 s. The
`mc-edge`-only edit took longer in this single run: its `mc_edge` incremental
compile went from 3 s to 11 s, and the cause is unexplained.

## Step 2 (designed, not started): settlements crate pilot

The pilot design is in `receipts/CrateSplitDesign/design.md`, outside Git,
along with the reference tables behind its counts.

- **What moves:** settlements, the villager AI behaviours,
  `plugin_structures`, `plugin_settlements` and the settlement command
  handler. That is 34.4k production and 46.5k test lines.
- **Why it is expensive:** settlement state is part of `mc-core`'s persisted
  model, through `TickState`, the villager cold-row column and `JournalEntry`.
  The smallest cut needs:
  - splitting 27 files into state and behaviour halves, with about 3.4k lines
    moving into `mc-core`;
  - a 17-method `SettlementSystems` hook trait called at 20 sites and wired by
    `mc-edge`;
  - widening about 495 private or `pub(crate)` items to `pub`;
  - turning 12 `cfg(test)` seams into production code;
  - touching 286 files.
- **Expected gain [INFERENCE]:** a settlements unit-test build of about
  21–26 s instead of the current 60–71 s. Other `mc-core` edits would change
  little.
- **Risks:**
  - Release speed: villager brains would lose cross-crate inlining, and the
    workspace has no LTO.
  - The internals made `pub` stay public for good, and every new settlements
    feature grows the hook layer. That is the boundary 0011 rejected.
  - Merge conflicts: 71 of the 302 `mc-core` commits since 2026-10-22 touched
    this area.

The design's recommendation is no-go until Step 3 narrows the coupling.

## Step 3 (in progress): feature unification, leaf extraction, ownership cuts

A crate split pays off only after `mc-core`'s cycle is cut at real ownership
boundaries. A split that adds a hook layer around shared state does not
qualify. Measurement is deferred to one batched quiet window (owner host,
2026-11-08: `-x` windows stall while campaigns and real-client gates run). That
window measures, with the Step 1 method, three separate comparisons: base
`168023ec`, feature unification `521714a8`, and the leaf-extraction commit.
Until then, none of these steps claims a build-time gain. The order:

1. **Feature unification.** `workspace-features` is the one workspace dependency
   feature owner, described below. Its effect is unmeasured: the historical
   3–4 minute lower-crate rebuild is not yet shown to be saved.
2. **Leaf crates first.** `world_conversion` has no production inbound references
   from other `mc-core` modules (4.8k production plus 6.7k test lines).
   `mc-edge::operator_cli` calls it, and several core tests also use it.
   Move those callers/tests without a reverse core dev-dependency when extracting
   a crate above `mc-core`. Other zero-inbound modules can follow.
3. **Narrow state ownership inside `mc-core` first.** Settlement and villager
   AI state moves out of `TickState`, the villager column and `JournalEntry`
   behind one owned state type, with persistence unchanged byte for byte.
   0017's declared systems give the tick a place to call domain systems
   without a hand-written hook trait. When the remaining inbound references to
   settlements are only that state type plus the system registration, extract
   the crate.
4. **Stop rule:** stop when a domain's unit-test loop is under about 30 s and
   the server-binary loop under about 45 s. Further splitting is not a goal in
   itself.
### Shared dependency feature owner

`mc-edge` depends on `workspace-features` normally; `mc-core` uses it only as a
dev dependency. Standalone lower crates keep their own small dependency graphs.
The feature owner's library has no runtime API; its manifest activates the union
of third-party features used by the server and core tests. It depends on no Solaris
crate, so it cannot introduce a reverse domain dependency. Cargo's
[`resolver.feature-unification`](https://doc.rust-lang.org/cargo/reference/unstable.html#feature-unification)
still requires an unstable flag; setting the configuration alone is ignored.
This implementation needs neither nightly nor `RUSTC_BOOTSTRAP`.

Resolver 2 separates host/build dependencies from target dependencies. The
proc-macro `syn` union is therefore a build dependency, activated by the crate's
build target; ordinary dependency features stay on the target side. Unix rustix
and Linux raw-sys features retain their platform predicates. Optional GPU
backends and test-only frameworks are not pulled into production just to unify
their own features.

On `4d32f280`, separate `mc-edge` build and `mc-core` test resolution differed in
11 shared package contexts: digest, getrandom, hashbrown, lazy_static, num-traits,
rand_core, regex-syntax, semver, smallvec, typenum (target), and syn (host).
The original all-workspace variant made `mc-edge` build, `mc-core` build,
`mc-core` tests and workspace tests agree in their common dependency contexts on
`x86_64-unknown-linux-gnu` with default features. However, it expanded standalone
mc-nbt's dependency graph from 11 to 84 packages. The root-only variant avoids
that leaf cost; its comparative rebuild measurements remain pending. The lockfile
adds only the local feature-owner package, not a new third-party version.

This is **dependency-resolution evidence, not build or performance acceptance**.
Receipts outside Git: `receipts/CrateSplit/feature-comparison.json`,
`*-context.txt`, `*-unified.txt`, and `feature-union-before.json`. The plan and
manifest graph are in that directory too. Cargo tree commands use
`--locked --offline --prefix depth --format '{p}|{f}'`, with `-e normal,build`
for builds and `-e normal,build,dev` for tests. Compare package version and
host/target context, not just package name or an aggregate feature union.
On merged base `168023ec`, root-only server-build and core-test resolution has
222 shared package contexts and zero feature differences
(`receipts/CrateSplit/root-only-comparison.json`). The standalone mc-nbt tree
contains nine distinct packages (`root-only-leaf.txt`), with no unifier edge.

Ordinary acceptance on this feature tree: workspace build, formatting,
code-health and documentation links passed. The core/edge unit run passed
4601 of 4602 tests (81 skipped) before its 3600-second command deadline.
`adopted_village_with_resident_shears_produces_stock_and_builds` remained running.
Main identified it as a pre-existing slow test outside this feature diff and
directed no rerun; it is not claimed passed. Receipts:
`root-only-workspace-build-02`, `root-only-consumer-tests-01/acceptance.json`,
`root-only-fmt-01`, `root-only-code-health-01` and `root-only-doclinks.json`.

When changing dependencies, update the shared feature owner to the resolved
union, preserving platform/host distinctions and excluding test-only packages.
Check the same feature contexts, then build and run the affected tests. A
feature-tree match does not prove that profile or compiler-option differences
cannot still trigger a rebuild.

| Scenario on the current Step 3 tree | Before | After |
|---|---|---|
| Cold server build | 271.233 s, clean `168023ec`, quiet lane | Awaiting batched quiet window |
| Trivial `mc-core` edit → server | Awaiting batched quiet window | Awaiting batched quiet window |
| Trivial `mc-edge` edit → server | Awaiting batched quiet window | Awaiting batched quiet window |
| Lower-crate edit → first core test build after server | Awaiting batched quiet window | Awaiting batched quiet window |

Use the Step 1 quiet-lane method for both columns, on frozen trees with
incremental compilation, existing dev/test profiles, clang/mold and no sccache.
The historical tables above are not substitutes for these measurements.
The clean baseline's first core test compilation took 250.887 s and rebuilt
eight lower libraries after the server build. This cold test is not the
lower-edit measurement. Receipts: `baseline-server-cold-02` and
`baseline-core-test-cold` under `receipts/CrateSplit`. Main deferred further
timings while campaign gates run: ordinary `-m` validation is not performance
evidence; neither cancelled `-x` nor cancelled `-p` attempts supply a comparison.

### Offline ownership boundaries

`mc-world-conversion` owns Anvil import/export, census and probes.
`mc-world-tools` owns pregeneration, native prune/compact and worldgen revision
upgrades. The edge's operator CLI depends on both; neither production nor test
dependencies point back from the core to these operator crates.

Core-native checkpoint codecs, recovery, metadata and plugin-startup records
remain with their existing owners. These are actual public persistence APIs,
not injected hooks or wrappers around `TickState`; the extraction changes no
encoded records. The native entity hydration implementation now belongs to
`native_checkpoint` and is shared by live dimension recovery and import.

`ChunkSource::new_native` owns the immutable store, generator, registry and
population inputs used by both live worlds and offline pregeneration. The
existing worker pool owns its queues and worker lifetimes. Offline tools use
the same terrain factory, admission bounds and load/generate path as serving;
no second generator or reduced-fidelity import path is introduced.

Private core scenario assertions remain in core tests. Cross-owner round trips
live with the converter and exercise public `Core` inputs/outputs or the native
store/journal format. In particular, mount cargo still crosses a real joined
player's menu, checkpoint, export and reimport; the Java oracle and malformed
projectile import checks move with the converter. The core retains its direct
WAL recovery, despawn, POI exclusivity, warning-clock and bee custody assertions.
Both offline crates supply the actual vanilla cache in tests rather than
relying on the core's `cfg(test)` defaults or publishing private fixture APIs.

Harness source paths and test discovery follow the new owners. The path-only
matrix migration is kept separate from implementation commits.

Canonical structural measurements on the extracted tree: 193 top-level core
modules and 191830 production lines; the largest SCC remains 189 modules.
These four offline modules were already outside the SCC: this extraction is
not a cycle reduction. The core's vanilla-literal and path-comparison ceilings
drop to the measured 2617 and 114; the other ceilings remain unchanged.
The local crate graph has 18 crates and 55 edges including development edges,
with no cycle. Receipts: `offline-root-only-code-health-01` and
`crate-edges-root-only-extracted.json` under `receipts/CrateSplit`.

Ordinary extraction acceptance passed on merged base `d5f3a8c9`: the complete
workspace build, 153 selected tests across both offline crates and affected core
recovery scenarios, formatting, and the tightened code-health ratchets.
The test runner skipped 3781 tests outside this selection or marked ignored;
this is not a full-workspace correctness claim. Receipts under
`receipts/CrateSplit`: `offline-root-only-workspace-build-02`,
`offline-root-only-affected-tests-01`, `offline-root-only-fmt-01`, and
`offline-root-only-code-health-02`.

The `config -> block_log` ownership cut and final whole-workspace correctness
gate remain unperformed at this checkpoint. The cycle ceiling is still 189.
Build-time comparisons remain deferred to the authorized batched quiet window;
ordinary acceptance durations establish no performance gain.


## Cycle ratchet (implemented 2026-11-07)

The owner approved a gate that keeps the cycle from growing while Step 3 waits.
`cargo run -p xtask -- code-health` builds `mc-core`'s top-level module graph
from production code with `syn`. Edges are `crate::` and `super::` paths in
`use` items, expressions, types and macro bodies; root re-exports resolve to
their module. `#[cfg(test)]` items and `*_tests.rs` files are skipped. Tarjan
gives the strongly connected components.

- **Gate:** no module may join the largest component. Ceiling set on
  `ffd52010`: 189 modules. Receipts: `receipts/CoherenceRatchet/`, outside
  Git. The component's production lines (188,451 on `ffd52010`) are a printed
  trend, never a failure: every feature inside a cycle module adds lines, so a
  line ceiling would only be raised by ritual.
- **Ceilings only go down by default.** A lower measurement prints
  `ceiling can drop to N`, and that change lowers the constant. Raising a
  ceiling is an explicit edit of the constant in
  `crates/xtask/src/code_health/ratchet.rs` with a reason comment.
- **Hints:** the report prints the most connected modules in the cycle and the
  modules that leave it for the fewest cut edges. On `ffd52010`, cutting
  `config -> block_log` frees 4 modules.
- **Limit:** only named module paths are edges. A dependency through a type's
  methods without a module path does not count.
- **New xtask dependencies: `syn` and `proc-macro2`.** Before: text counters that
  missed macro bodies, counted comments and could not resolve modules. That
  made a failing gate impossible: no exact count, no module graph. `syn`
  replaces the text counters, which are deleted; both crates were already in
  `Cargo.lock` through other crates.

The same gate ratchets four anti-pattern counts; see
[agent tooling](../AGENT_TOOLING.md#code-health-ratchets).
