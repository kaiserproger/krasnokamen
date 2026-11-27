# Performance windows and profiling

[AGENTS](../../AGENTS.md) and [performance DoD](../DEFINITION_OF_DONE.md) bind
acceptance. [PROFILES](../performance/PROFILES.md) and
[BENCHMARKS](../performance/BENCHMARKS.md) own published workloads/commands.
This page owns measurement method, not a second catalog.

## Window protocol

1. Freeze the brief's exact tree; prepare one instrumented binary and smoke the
   profiler **outside** measurement: absolute output path, exit 0, samples >0.
   Cargo integration tests use crate cwd; relative perf output previously failed 255.
2. Run canonical workload **unshortened** through Main's absolute `mem-run.sh -x`.
   It freezes builds, drains tests/servers and serializes measurement; no GO.
   [Runner lock/lane rules](heavy-commands-and-memory.md#how-toolsdevmem-runsh-works-current-since-2026-10-28)
   apply, including scene-before-exclusive ordering.
3. `tools/dev/measurement-windows` and the existing announcement plan own
   allocations (recurring 01:00–02:30 plus dated grants). Expired grants do not
   recur. Outside grant = refusal, not permission to bypass it. Only Main may
   explicitly authorize override; reassign before next cell, never mid-cell.
4. Report gate status, p50/p95/p99, stage breakdown, CPU time/tick and raw profile path.

Single-agent in-process micro-benches may use `-p` outside a quiet window:
last four CPUs, record `uptime`, compare only same-session `-p` runs. They are
not capacity claims. A rustc on bot CPUs contaminated a MIX replay; full load
measurements therefore require `-x`.

## Comparing builds

- Alternate A B A B (or reverse) in one session with equal build/nice/quota,
  physical/SMT placement, client placement and contention. Baseline is the
  previous round's exact frozen binary, never reconstruction. Name differences.
- Profiled/contaminated runs (ratios included) are diagnostic only. Compare clean
  repeats and record throttling, contention and A-vs-A spread. If that spread
  approaches A-B, result is **undetermined**. Shared-host load alone proves no comparability.
- Equal useful workload: tracking/view distance, simulation/AI/physics/activation,
  mode and actual actor routes. Log successful login and active-cohort movement;
  intentional idle actors need not move. Remote clients are distinct from local,
  not interchangeable capacity points ([benchmark contract](../performance/BENCHMARKS.md)).
- Separate resident, active, admitted/ticked and rendered populations with sample
  interval/frequency. Received entities are not submitted crowd bodies; zero
  submitted bodies cannot prove crowd FPS. Missing required scene/group leaves
  aggregate undefined, never reweighted. Preserve failed primary scenes when adding
  diagnostic closer/repacked fixtures; [evidence classes](real-client-gates.md#evidence-classes) apply.
- Claim gains from the real workload, not micro extrapolation: they disagreed by
  3×; page-batching's micro win regressed full bench 21–36% and was reverted.
  Count real calls before scaling micro results. Revert an optimization with no
  measurable effect in its target stage.
- Name algorithms from implementation, not absent spellings or aspirations:
  [villager motion](../../crates/mc-core/src/villager/brain/motion.rs) reuses
  progressing paths then calls `find_path_ctx` (cached A*, not a flow-field
  result). Serial ownership alone proves neither bottleneck nor scaling win.
- Attribute CPU per tick via `/proc/self/task/*/stat`, not wall time alone.
  Separate server/bot processes for server CPU (old shared Tokio runs only
  attributed tick-thread CPU). Check `cpu.stat` before interpreting tails:
  23 unpinned threads under six-core quota throttled 856/929 periods, versus
  zero pinned (2026-10-10).

## Always-profile server load windows

`server-load-profile` wraps an existing battle/coreperf/catalog workload and
always captures server CPU-clock stacks. Use an immutable nonincremental
frame-pointer release pin, whole command under `-x`; bots/recorder are not
sampled. Wrapper forwards console/shutdown to the real server.

```sh
python3 -m tools.harness run server-load-profile -- \
  --kind coreperf --budget-ms 50 -- \
  python3 -m tools.coreperf.profiles native-100 \
  --server-bin /absolute/server-pin/krasnokamen \
  --bots-bin /absolute/bots-pin/player-scale-bots \
  --build-receipt /absolute/server-pin/manifest.json \
  --fixture-cache /absolute/prepared-cache \
  --server-wrapper '{wrapper}' --out '{out}/workload'
```

Harness expands `{wrapper}` and `{out}`. Battle uses `--take --server-wrapper
'{wrapper}' --run-dir '{out}/workload'`; catalog uses one scenario/client rung,
`--server-wrapper '{wrapper}' --out '{out}/workload'`. Other workload args and
canonical duration stay unchanged.

`profile-result.json` records tick range, mean/p95 active iteration ms, p95
budget, sampled server CPU ms/completed source tick, ranked self/inclusive
functions and hash/comparison/allocation/map-lookup flags. Inclusive functions
and categories **overlap: never sum**. Unsymbolized kernel frames remain
`[unknown]`. `neighbours.txt` captures first/middle/last process snapshots;
idle waiters/frozen builds are not CPU interference. Keep logs, manifests and
raw tick telemetry as provenance.

Failed/prepared/smoke command, incomplete battle, missing middle snapshot,
missing/discontinuous ticks, failed profiler or absent stacks fails closed.
Over-budget complete capture is `budget_exceeded`. Complete windows from failed
workloads retain rankings as `diagnostic_only: true` but stay failed;
incomplete setup is no measurement. Parser/budget controls:
`python3 -m unittest tools.harness.test_server_load_profile`; real-window
control re-analyzes the same capture below its p95 budget. Profile first,
then N/2N/4N on the measured hotspot under equal conditions; report growth
exponents and CPU/wall, not profiled capacity.

## Profiling recipes

- **Maintained window:** `tools/dev/perf-window.sh <pid> [seconds] [outdir]` is the
  script a capture uses (do not re-invent it in `receipts/`). It runs three
  sequential windows of `seconds` each: `perf stat -M PipelineL1 --per-thread`
  (top-down verdict), `perf stat -x,` over the explicit AMD events
  `ls_dmnd_fills_from_sys.all`, `l2_request_g1.all`, `l2_cache_req_stat.ls_rd_blk_c`,
  `cycles`, `instructions`, then `perf record -g --call-graph fp --per-thread`
  with inclusive callers (`perf report -g "graph,0.5,caller,0.5"`), `self.txt` and
  `script.txt` for flame graphs. `cache-references`/`cache-misses` alias to L2 on
  AMD64: never quote them as a last-level number. `l2_misses.all` is not an event
  of the owner host's PMU (`perf stat -e l2_misses.all` refuses it); the L2 miss
  counter there is `l2_cache_req_stat.ls_rd_blk_c`. The call graph needs the
  frame-pointer pin above; the script refuses a capture without caller frames.
  In `-x,` output the enabled/running percentage is field 6 with `--per-thread`
  (field 5 without), so a multiplexed counter is visible instead of quoted as exact.
  Per-thread record buffers come from `RLIMIT_MEMLOCK` (8 MiB in a user session),
  so the script caps them at `PERF_WINDOW_MMAP_PAGES` (default 16); raise both if a
  capture reports lost samples.
- **Pin:** `CARGO_TARGET_DIR=<work>/targets/fp RUSTFLAGS="-C force-frame-pointers=yes"
  CARGO_PROFILE_RELEASE_DEBUG=line-tables-only <work>/solaris/tools/dev/pin-release
  <tree> --no-incremental`. Instrumentation is part of fingerprint; `--bots`
  selects bots. Retain printed binary SHA and manifest; no ad-hoc release build.
- **Attribution:** `perf report -i perf.data --no-children --comm mc-core-tick
  --sort sym --stdio -g none --no-inline`; `--children` is inclusive.
  Confirm apparent hotspots with `--no-inline --symbol-filter=<sym> -g caller`
  (inline attribution once made a 100-row collect look like 9 ms).
  1% ≈ tick_ms × tick-thread share /100. DWARF without frame pointers can leave
  Rust callers unknown.
- **Attach:** `samply record -p <pid> -d <secs> -s -o /abs/profile.json.gz`
  or `samply import perf.data`. Owner must authorize kernel settings;
  own-process attach needs `perf_event_paranoid=1`; ptrace_scope=1/paranoid=4
  previously blocked gdb/perf. Never change security settings silently.
- **Harness wrapper:** for arguments containing ` --config `, run
  `perf record -F 999 --call-graph dwarf,16384 -o <dir>/perf-$$.data --
  <frozen-bin> "$@"`; other calls exec the binary normally (export/converters
  share it). Never poll to attach. DWARF can use ~2 GB/minute; inspect with
  `perf report -g none --no-children --sort comm,sym` and retain under
  [run-data rules](heavy-commands-and-memory.md#disk), not an automatic delete.
- **Phase timers:** `MC_CORE_PHASE_LOG=1` in server.log. Dashboard phase sums
  below total indicate untimed spawning/input/chunk/commit/capture work.
  `tick/phase_laps.rs` has no clock overhead when unset. Overworld logs:
  `input|chunks|entity systems|block|commit detail ms`, `entity phase detail ms`,
  `prepare rows detail ms`; laps sum to their phase. `step_scope` is parallel:
  split with perf, not laps. `post detail us` separates pickup/hostile/projectiles;
  `battle detail us` includes clocks and probe counters (standing, target switches,
  motion/windup facing errors). `battle_slots−stepped_battle_rows` bounds rests,
  not counts them: inactive/dying rows are included.
- **Worldgen:** test-only `terrain/profile_tests.rs` stage clocks; split with a
  new `end(index)` and widened totals. Use `--exact` to avoid matching other modes.
  Laptop GPU/-p sample 2026-10-29: alone 7.80–8.98, shared solids 6.42–7.00,
  GPU density 4.08–4.68 ms/chunk (2–5% variance). Neighbor solids halve ore-halo
  cost; one-chunk GPU batches cost ~5 ms fixed overhead, prefer spatial tiles.
  Offline pregeneration throughput is not live serving; live needs canonical profile.

## Optimisation rounds (native-100 pattern, 2026-10)

Land disjoint slices in own games-disk targets, then use
[one acceptance freeze/pin](worktrees-and-integration.md#freezing-one-acceptance-identity).
Main's `FREEZE rN`/`BUILD DONE rN` controls release; no worker self-release.
Pending/uncompiled work stays isolated, not discarded. Combined tree needs
strict touched-crate Clippy/focused tests, disk check and frame-pointer pin/hash.
Run full cached-fixture workload under `-x` with perf (prepared-world reuse
historically saved ~20 min); next round follows measured hotspots.

TPS = completed ticks/full window, not first-to-last capture span. Historical
10 TPS threshold was not “p50 <100 ms”: tails matter. INFRA_BLOCKED (e.g. ENOSPC)
is invalid; at low TPS inspect bot dropped-action reasons before calling an oracle
failure a server bug.

Durable optimization boundaries:

- Quiescence caches prove geometric absence, never suppress live burn/fire/
  portal timers. [Tick-anchor projection](../PITFALLS.md#code) replaces journaled
  per-tick counters. Transparent huge pages had no measurable gain (2026-10-18).
- Contact work classifies on pool; serial owner receives only live burn,
  feet-cauldron or real-hazard rows. Section palettes are coarse, so pool checks
  exact eye/body/support cells. In roofed native-100, serial contact admission
  fell from ~5300 rows/tick to the 100 players (2026-11-12). Portal uses the same
  palette-clear proof; trader scan uses three villager-kind lists; despawn
  candidates come from `advance_admitted_ages`, not a second cold sweep.
- `native100_input_phase_micro` (`tick_tests::contact_hazards_tests`) is the
  ignored roofed 100-player/104k-mob test. Outside measurement, build with
  `<work>/solaris/tools/dev/pin-release <absolute-tree> --core-tests --no-incremental`;
  run the printed release test executable under `-p` with phase logs. The tool
  uses the ordinary build lane and a 12G cap; do not use ad-hoc `cargo test --release`.
- Commit laps split `merchant_mounts_allay` into merchant/mounts/allay/
  equipment_pickup. 3000 mature items exposed collector selection at 1.2–1.5 ms;
  it now scans `MOTION_ITEM_COLLECTOR`, not cold type rows. Egg clocks run on
  pool via `advance_egg_clocks`/`MOTION_EGG`; zombie unchanged-arms decisions
  move to pool, writes remain serial. `N100_ITEMS` enables the collector in
  `native100_like_entity_phase_micro` (`entity_prephase_tests`).
- Item `PickupDelay=32767`/`Age=-32768` never tick those clocks; unsigned age
  stores `ITEM_AGE_NEVER`, converted at Anvil boundary.

## Load harness and the frozen old core

`tools/coreperf/compare.py --mobs` marks cohorts by fixture custom names.
Pace preparation equally: old-core ingress kicks excessive setup bots.
`--prepare-only` durability must pass first. 100k mobs need ~8 GB cgroup peak
including cache despite ~2.3 GiB server RSS. Non-PASS/missing receipts fail;
runner skips existing receipts. Non-smoke MIX enforces minimum durations
(MIX-18 steady/burst ≥33 s); readiness timeout is per region. Preserve worktree
`.analysis/` with the [verified archive procedure](worktrees-and-integration.md#creating-a-worktree)
before removal.

## In-process native-100-like tick micro

Ignored `mc-core::native100_like_entity_phase_micro` is an internal tick test,
not real-protocol capacity. `N100_MOBS=104000` (multiple of eight),
`N100_VILLAGERS=64` defaults; 100 in-process players, 60 real ticks.
100k/20k PGO training **and every candidate** use `N100_MOBS=100000
N100_VILLAGERS=20000`. Outside measurement, build with Main's absolute
`<work>/solaris/tools/dev/pin-release <absolute-tree> --core-tests --no-incremental`
in the ordinary build lane; run its printed executable with
`--ignored --nocapture native100_like_entity_phase_micro` in `-x -p`.
NATIVE100 samples contain total ms and test-only villager brain/roster µs;
seven `villager_phase_us` counters (refresh/admission/evaluation/social/
acquisition/full/skipped) overlap, never sum. Flat scene lacks buildings/
workstations: no inhabited-settlement coverage claim.

## Related tools

- [Worldgen comparison](../../tools/worldgen_diff/README.md) and
  [retained evidence](../evidence/worldgen-diff/README.md).
- [Player-capacity/weak-hardware/scaling/VPS matrices](../performance/BENCHMARKS.md).
- [Items-heavy profiles](../performance/PROFILES.md#many-items-benchmarks):
  polluted fixture native-100, `items ...`/`edge items:` logs and
  `tools/coreperf/items_table.py`.
