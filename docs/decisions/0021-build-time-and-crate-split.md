# ADR 0021 - Build time: line-table debuginfo now, an mc-core split later

**Date:** 2026-11-05  
**Status:** Step 1 accepted by the owner and implemented (2026-11-05). Step 2
(settlements crate pilot) designed and not started: the design recommends
against it at today's coupling. Step 3 is the future plan below. The cycle
ratchet (owner-approved 2026-11-07, implemented) makes code-health fail when
the module cycle grows.  
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

## Step 3 (future plan)

A crate split pays off only after `mc-core`'s cycle is cut at real ownership
boundaries. A split that adds a hook layer around shared state does not
qualify. The order, each step measured with the Step 1 method before the next:

1. **Feature unification.** Make `cargo build` and `cargo test` resolve the same
   dependency features: wait for stable `resolver.feature-unification`, or use
   one workspace dependency-features crate. This saves the 3–4 minute rebuild of
   lower crates after rebases. No `mc-core` change is needed.
2. **Leaf crates first.** `world_conversion` has no production inbound
   references (4.8k production plus 6.7k test lines) and can become a crate
   above `mc-core` with no hook. Other zero-inbound modules can follow.
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
