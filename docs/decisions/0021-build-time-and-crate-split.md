# ADR 0021 - Build time: line-table debuginfo now, an mc-core split later

**Date:** 2026-11-05  
**Status:** Step 1 accepted by the owner and implemented (2026-11-05). Step 2
(settlements crate pilot) designed and not started: the design recommends
against it at today's coupling. Step 3 root-only feature unification passes
workspace build, formatting and code-health; consumer test scope is below.
Comparative timings await Main's batched quiet window.
Leaf extraction is implemented; config owns block-log policy. Further cycle cuts
remain under acceptance.
The cycle ratchet
(owner-approved 2026-11-07, implemented) makes code-health fail when the module
cycle grows.
**Direction (owner, 2026-11-16):** no big-bang split now. Every new feature is
built as its own module (or crate) with its state, systems and tests inside, a
narrow interface and one host registration point, and does not grow the module
count of the largest cycle (a submodule of its domain host, or a top-level
module the cycle does not refer to; a submodule's references count as its
host's, so the count is checked on the integrated tree;
[how-to](../development/adding-a-feature.md#new-features-are-modules)).
Moving existing code into modules follows this ADR's cycle-cut plan with
byte-exact moves; it may start before the 21 November announcement only once the
current release tasks are closed, otherwise after (owner, 2026-11-17).
**Related:** retired v1 ADR 0011, in Git history (rejected
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
| Cold server binary (`-p mc-edge --bin krasnokamen`) | 6m32s |
| Cold `mc-core` unit-test binary, after the above | 4m22s |
| One-line `mc-core` edit → server binary | 59–71 s |
| One-line `mc-core` edit → `mc-core` unit-test binary | 76–99 s |
| One-line `mc-edge` edit → server binary | 8 s |

- **Crate size:** about 400k lines: 201k production and 199k in `*_tests.rs`.
- **Incremental cost:** a trivial edit compiles `mc-core` in 42 s versus 55 s
  cold, then rebuilds `mc-edge` in 22–28 s versus 22 s cold. Cold core phases:
  front end/type/borrow checking ~18 s, metadata 9 s, monomorphization 7 s,
  codegen/LLVM ~30 s. Parallel front end (`-Z threads=8`) took 60 s: no gain.
- **Feature mismatch:** separate `mc-edge` build, `mc-core` build and core test
  commands resolve different low-level features (`bitflags`, `log`, `smallvec`,
  `digest`, `typenum`). After a lower-crate edit, the first test build repeats
  `mc-data`, `mc-world`, `mc-worldgen`, `mc-script` and `mc-plugin-host`: ~3–4 min.
  Cargo's `resolver.feature-unification` is unstable on 1.99.
- **Dependency cycle:** 189/197 top-level modules (185k/201k production lines)
  form one SCC; `tick` reaches 148. The original denominator 421 counted file
  and directory names, including 194 test files. These are raw-line counts:
  non-`*_tests.rs` files and cycle modules' `<m>.rs`/`<m>/`, inline tests included.
  The production-module-tree ratchet below excludes tests: 188k/195k lines.

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

Core paths and cold builds improved; the binary's share of a core edit fell
from 13–22 s to 2 s. The single-run edge-only regression is unexplained:
`mc_edge` incremental compilation rose from 3 s to 11 s.

## Step 2 (designed, not started): settlements crate pilot

The pilot design is in `receipts/CrateSplitDesign/design.md`, outside Git,
along with the reference tables behind its counts.

- **Scope:** settlements, villager AI behaviours, `plugin_structures`,
  `plugin_settlements` and settlement commands: 34.4k production/46.5k test lines.
- **Coupling:** persisted state spans `TickState`, the villager cold-row column
  and `JournalEntry`. The minimum cut splits 27 files (about 3.4k lines back
  into core), adds a 17-method `SettlementSystems` trait at 20 call sites wired
  by edge, widens ~495 private/`pub(crate)` items, moves 12 test-only seams into
  production and touches 286 files.
- **Gain [INFERENCE]:** settlements unit-test builds ~21–26 s versus 60–71 s;
  little improvement for other core edits.
- **Risks:** villager brains lose cross-crate inlining without workspace LTO;
  permanently public internals and a hook layer growing with every settlement
  feature repeat the boundary rejected by 0011; 71/302 core commits since
  2026-10-22 touched this area, making conflicts likely.

The design's recommendation was no-go until Step 3 narrows the coupling. The
owner overrode the timing on 2026-11-19 (D65: «Да, начать сейчас, параллельно с
работой над поселениями, и принять конфликты»): the work starts now, in parallel
with the current settlement slices, and their merge conflicts are accepted. The
order stays: Step 3 item 3 (one owned settlement and villager state type,
persistence byte for byte) first, then this crate. Build time (the Step 1
method) and release game speed (villager brains without cross-crate inlining,
no LTO) are measured before and after; neither gain is claimed before that.

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

**D65 in-crate persistence cut (S3).** The dimension-routed `Replay` carries
one `settlements::persistence::Tail`; its records, tombstones, allocator floor
and transit images and their folds belong to settlements, not the journal.
The D63 starter-chest grant counts and their WAL/checkpoint folds share this
tail owner; their existing player/count codec and overwrite semantics stay intact.
Settlement checkpoint projection, stale-row prefixes, image heap accounting
and the census projections live behind that same owner. Recovery reaches one
dimension-admission call before entity hydration and one completion call after
all dimensions and the entity salt, with the legacy-state refusal first.
This is an ownership cut only: existing codecs, validation stages and emitted
checkpoint/WAL/NBT bytes do not change. `tools/dev/bytecheck.sh` compares a
pinned base/candidate pair; no build-time or runtime gain is claimed.

### Shared dependency feature owner

`workspace-features` has no runtime API or local crate dependency: its manifest
owns the third-party union for edge's normal dependencies and core's dev
dependencies, without reverse domain edges or inflating standalone lower crates.
Cargo's unstable
[`resolver.feature-unification`](https://doc.rust-lang.org/cargo/reference/unstable.html#feature-unification)
is ignored without its flag; this implementation needs neither nightly nor
`RUSTC_BOOTSTRAP`.

Resolver 2 separates host/build and target contexts. The proc-macro `syn` union
is a build dependency activated by the build target; ordinary features remain
target dependencies. Preserve Unix rustix/Linux raw-sys predicates. Do not pull
optional GPU backends or test-only frameworks into production for unification.

**Resolution evidence, not build/performance acceptance:**

| Tree/variant | Observation |
|---|---|
| `4d32f280`, separate edge build/core tests | 11 differing shared contexts: target digest, getrandom, hashbrown, lazy_static, num-traits, rand_core, regex-syntax, semver, smallvec, typenum; host syn |
| Original all-workspace union | Edge/core builds, core/workspace tests agree in common contexts on default-feature `x86_64-unknown-linux-gnu`, but mc-nbt grows from 11 to 84 packages |
| Root-only, merged `168023ec` | 222 shared server-build/core-test contexts, zero feature differences; standalone mc-nbt has nine distinct packages and no unifier edge |

Root-only avoids the leaf cost; rebuild gains remain unmeasured. The lockfile
adds only the local feature owner, no third-party version. Evidence, plan and
manifest graph: `receipts/CrateSplit/feature-comparison.json`, `*-context.txt`,
`*-unified.txt`, `feature-union-before.json`, `root-only-comparison.json`,
`root-only-leaf.txt`. Cargo tree uses
`--locked --offline --prefix depth --format '{p}|{f}'`, with `-e normal,build`
for builds and `-e normal,build,dev` for tests. Compare version and host/target
context, not package name or aggregate feature union.

**Ordinary acceptance:** workspace build, formatting, code-health and doclinks
passed. Core/edge tests reached 4601/4602 passes (81 skipped) before the 3600 s
deadline; `adopted_village_with_resident_shears_produces_stock_and_builds` was
still running. Main identified this as pre-existing/outside the diff and directed
no rerun; it is not passed. Receipts under `receipts/CrateSplit`:
`root-only-workspace-build-02`, `root-only-consumer-tests-01/acceptance.json`,
`root-only-fmt-01`, `root-only-code-health-01`, `root-only-doclinks.json`.

Dependency changes update the resolved union with platform/host distinctions
and without test-only packages, check those contexts, then build and run affected
tests. Matching features cannot exclude profile/compiler-option rebuilds.

| Scenario on the current Step 3 tree | Before | After |
|---|---|---|
| Cold server build | 271.233 s, clean `168023ec`, quiet lane | Awaiting batched quiet window |
| Trivial `mc-core` edit → server | Awaiting batched quiet window | Awaiting batched quiet window |
| Trivial `mc-edge` edit → server | Awaiting batched quiet window | Awaiting batched quiet window |
| Lower-crate edit → first core test build after server | Awaiting batched quiet window | Awaiting batched quiet window |

Both columns require frozen trees with Step 1's quiet-lane method: incremental,
existing dev/test profiles, clang/mold, no sccache. Historical timings above are
not substitutes. The clean baseline's first core test compile took 250.887 s,
rebuilding eight lower libraries after the server; it is not a lower-edit
measurement. Receipts under `receipts/CrateSplit`: `baseline-server-cold-02`,
`baseline-core-test-cold`. Main deferred timings during campaign gates; ordinary
`-m` validation and cancelled `-x`/`-p` attempts provide no comparison.

### Offline ownership boundaries

| Owner | Boundary |
|---|---|
| `mc-world-conversion` | Anvil import/export, census, probes |
| `mc-world-tools` | Pregeneration, native prune/compact, worldgen revision upgrades |
| Edge operator CLI | Depends on both; core has no production or test reverse dependency |
| Existing native owners | Checkpoint codecs, recovery, metadata, plugin-startup records; public persistence APIs, not injected `TickState` hooks |
| `native_checkpoint` | Entity hydration shared by live dimension recovery and import |
| `ChunkSource::new_native` / existing worker pool | Immutable store/generator/registry/population inputs and queue/worker lifetimes; offline and live use the same terrain factory, admission bounds and load/generate path |

Encoded records and generation fidelity do not change. Core-private assertions
stay in core; converter round trips use public `Core` I/O or native store/journal
formats. Mount cargo crosses a real joined player's menu, checkpoint, export and
reimport. Java-oracle and malformed-projectile import tests move with conversion;
core retains direct WAL recovery, despawn, POI exclusivity, warning-clock and bee
custody tests. Both offline crates supply the real vanilla cache, not core
`cfg(test)` defaults or newly public fixtures.

`live_capture_tests.rs` retains portable checkpoint round trips for trader
purchases, shrieker warning aging, villager job claims and inhabited time.
Public queues/outputs drive mutations; journal capture and dimension hydration
remain private. Inhabited-time coverage requires a nonzero checkpoint contribution
and later active intervals separated by a spectator interval.

`live_capture_crash_tests.rs` adds a **Linux-only** (`cfg(target_os = "linux")`)
chain: child runs the public-Core scenario, obtains `Core::backup` baseline,
then acts; inotify only wakes observation, while native WAL decoding validates
published payloads, checksums and contiguous coverage through the action tick.
It exits without Core destructors/shutdown checkpoint; parent exports offline
and asserts checkpoint/replay ticks and Anvil/NBT state. This is neither power-loss
nor cross-platform crash coverage. Revised scenarios/capture-negative controls
await execution under the heavy-run hold; prior manually driven `TickState`
results do not validate this revision.

Harness paths/test discovery follow the owners; path-only matrix migration stays
separate from implementation commits.

Historical extraction measurements: 193 core top-level modules/191830 production
lines, SCC still 189 (all four offline modules were already outside it: no cycle
reduction). Literal/path-comparison ceilings drop to 2617/114; others unchanged.
The local graph has 18 crates/55 edges including dev edges, no cycle. Receipts
under `receipts/CrateSplit`: `offline-root-only-code-health-01`,
`crate-edges-root-only-extracted.json`.

On merged `d5f3a8c9`, workspace build, 153 selected offline/core-recovery tests,
formatting and tightened code-health passed; 3781 tests were outside selection
or ignored, **not** a full-workspace correctness claim. Receipts under
`receipts/CrateSplit`: `offline-root-only-workspace-build-02`,
`offline-root-only-affected-tests-01`, `offline-root-only-fmt-01`,
`offline-root-only-code-health-02`.

### Config owns block-log policy

`config::BlockLogSettings` owns enabled/retention/count policy and defaults;
`block_log::BlockLog` owns capture/query/prune/recovery. Core config and edge use
the new owner without an alias; policy, persisted fields and operator keys stay
unchanged.

| Historical evidence | Result |
|---|---|
| `6b594d8f` → policy cut | SCC 189/193 modules, 191,211 production lines → 186/193, 190,745; `config`, `debt`, `carrier_blocks` leave; ceiling 186 |
| Converter prerequisite | Traders, warden warning clocks, villager jobs and inhabited time each pass graceful checkpoint and abrupt-exit checkpoint/WAL export (8 checks) on unchanged production; removing recovered export projections fails all 8 exported-value assertions |
| Affected acceptance | 21 selected block-log, edge-config and live persistence tests pass; no build-time gain established |

Receipts: `receipts/CrateCuts/cut1-cycle/20261109T133700-code-health-4zqnqllh/code-health.log`,
`receipts/CrateCuts/persistence-base-evidence.json`,
`receipts/CrateCuts/persistence-negative.patch`,
`receipts/CrateCuts/cut1-affected/20261109T133909-test-6yma_vz9/test.log`.
Build comparisons await Main's quiet batch, not ordinary acceptance durations.

### Tick owns settled-block orchestration

Top-level `blocks` becomes `tick::blocks`, byte-identical and at the same
`run_tick` position. Falling/placement/plants/sign policy owners remain; no adapter
or callback replaces the module. Three fluid owner tests call the tick child.

Historical SCC: 186/193 after the policy cut → 185/192 modules, 190,746 production
lines; ceiling 185. This removes a false boundary, not a runtime algorithm.
Byte oracle/measurement:
`receipts/CrateCuts/cut2-byte-oracle.json`,
`receipts/CrateCuts/cut2-cycle/20261109T142805-code-health-gpu84yzk/code-health.log`.
All 172 selected block-tool/falling/placement/fluid/live checkpoint-WAL-export
tests, formatting and tightened code-health pass under
`receipts/CrateCuts/cut2-affected/`, `cut2-fmt/`, `cut2-ratchet/`.
Build timing remains in the quiet-window batch: no speed claim.

### Pure book encoding, tick-owned inventory authority

`books::replacement` is pure `mc-data`/`mc-nbt` encoding: original stack,
registry, pages, optional title and explicit author produce a replacement or
refusal; UTF-16 limits/component replacement stay unchanged. `tick::book_actions`
owns session/alive/spectator/slot checks, opening, custody-floor validation
immediately before commit and authoritative inventory publication. Both dispatchers
call it: no second inventory API or injected policy.

Historical SCC: 184/192 modules, 190,691 production lines; `books` leaves, ceiling
184. Receipt:
`receipts/CrateCuts/cut3-cycle/20261109T144106-code-health-xokla2h6/code-health.log`.
The 42 selected book/live checkpoint-WAL-export tests and formatting pass
(`receipts/CrateCuts/cut3-affected/20261109T144409-test-n8x7ak6i/`,
`cut3-fmt/20261109T145156-fmt-uuuzn3x_/`). Coverage includes exact/over-limit
UTF-16, unrelated components, real native custody refusal and both hand dispatchers.

On rebase `9062a976`, the 184-module gate passes
(`receipts/CrateCuts/rebased-ratchet/20261109T150721-code-health-fxfimnf8/`).
Two restored negative mutations fail the original assertions: raising the UTF-16
page bound admits oversized pages; dropping custody-floor validation signs a
reserved book. Receipts:
`receipts/CrateCuts/rebased-utf16-negative/20261109T150825-test-lilybvrm/`,
`rebased-custody-negative/20261109T151112-test-4397i811/`.

Final rebase `8dd2c61f`: strict workspace Clippy passes
(`receipts/CrateCuts/final-clippy/20261111T083013-clippy-utavssiy/`);
workspace/all-target tests pass 7,496/7,497
(`receipts/CrateCuts/final-nextest/20261111T083049-test-82ddtht3/`).
The sole failure is the pre-existing one-tick illusioner fixture after Main's
target-search cadence change. An approved fixture patch preserves all assertions
and journals each tick until acquisition; three requested runs pass under
`receipts/CrateCuts/illusioner-positive-{1,2,3}/`. Earlier regression/formatting
passes are retained, not repeated. This is staged L2, **not** a fresh all-green
`correctness`, real-client or build-speed claim. Exact trees/receipts:
`receipts/CrateCuts/final-manifest.json`; timing awaits Main's frozen quiet batch.


### Entity view pages are a boundary child, motion gate is an entity child

The dense COW page container `entity_views` had only `boundary` as referrer
(a re-export) and out-edge; its records are `boundary::EntityView`. It moves,
tests included, byte-for-byte to `boundary::views`, removing a false top-level
node while keeping public `boundary::EntityViews`.

Motion-lane classification moves verbatim from `entities.rs` to
`entities/motion_gate.rs`. The store retains `flags`/`hints`, dirty queue and
custody; the gate fills them from hot body, cold retained state and blueprint
registry. The child adds no top-level node. `entities` still depends on cyclic
`boundary`/`blueprint` types throughout its store methods and cannot leave.

Historical integration measurement: SCC 186/199 modules, 235,960 production
lines; `entities.rs` 925 → 864 items/statements; ceilings 186/864. No encoded
record, call order or gameplay decision changes. Move script/measurements:
`receipts/CutEntities/`.

### Cycle ratchet (implemented 2026-11-07)

Owner-approved on 2026-11-07:
`cargo run -p xtask -- code-health` builds the production top-level module graph
with `syn` and finds SCCs with Tarjan. Edges include `crate::`/`super::` paths
in use items, expressions, types and macro bodies; root re-exports resolve to
owners. `#[cfg(test)]` items and `*_tests.rs` are excluded.

- **Gate:** no module may join the largest component. Original `ffd52010`
  ceiling: 189 modules (`receipts/CoherenceRatchet/`, outside Git). Its 188,451
  production lines are a printed trend, never a failure: ordinary features add
  lines, so a line ceiling would merely be raised by ritual.
- **Ceilings fall by default:** `ceiling can drop to N` requires lowering the
  constant in the same change. Raising one requires an explicit constant edit
  and reason comment in `crates/xtask/src/code_health/ratchet.rs`.
- **Hints:** highest-connectivity modules and cheapest cut edges; on `ffd52010`,
  cutting `config -> block_log` frees four modules.
- **Limit:** only named paths count, not dependencies resolved through methods
  without a module path.
- **Dependencies:** `syn`/`proc-macro2` were already in Cargo.lock. They replace
  the deleted text counters (missed macros, counted comments, unresolved modules)
  with the exact count/graph needed for a failing gate.

The same gate ratchets four anti-pattern counts; see
[agent tooling](../AGENT_TOOLING.md#code-health-ratchets).
