# Performance windows and profiling

Binding rules: [AGENTS.md](../../AGENTS.md) (measurement windows need Main's GO
on a frozen integrated tree; compare only matching lanes; narrow optimisation
hacks document bottleneck, fence, effect and removal path) and the
[definition of done](../DEFINITION_OF_DONE.md) "Performance And Multithreading
DoD" section. Published workloads and their
commands are in [PROFILES.md](../performance/PROFILES.md) and
[BENCHMARKS.md](../performance/BENCHMARKS.md). This page is the working method.

## Window protocol

1. The measuring agent prepares **one** binary with all instrumentation it needs
   and smoke-tests the profiler capture outside the window on a short unmeasured
   run: absolute output path, exit 0, samples > 0. Cargo integration tests run
   with the crate as cwd; a relative perf output path made perf exit 255.
2. Agent to Main: `READY FOR WINDOW` (binaries built, expected duration).
3. Main tells only the agents running heavy work (not idle agents, not
   `agent://all`) that a window is starting, waits for their acks, then sends
   `GO` to exactly one agent. Number the windows to avoid out-of-order
   confusion.
4. During the window: no builds on any CPU, including the bot CPUs; servers,
   imports and world upgrades are gated too. A rustc on the bot CPUs
   contaminated a MIX replay once. With the current runner, `mem-run.sh -x`
   freezes preemptible builds and waits for running tests and servers (see
   [heavy commands](heavy-commands-and-memory.md)).
5. Run the canonical scene **unshortened**.
6. Agent to Main: `WINDOW DONE` with gate status, p50/p95/p99, stage breakdown,
   CPU time per tick, profile path. Main sends `WINDOW CLOSED` to the same
   agents.

In-process single-agent micro-benches pinned to the perf lane (`mem-run.sh -p`,
CPUs 8-11) may run without a window if `uptime` is recorded; their numbers are
comparable only with other `-p` numbers from the same session.

## Comparing builds

- A/B alternates in one session: A B A B (or B A B A) under identical
  conditions (CPU quota, nice, build profile, concurrent load). Name any
  difference before comparing with an older baseline.
- A profiled run carries extra load and is diagnostic only; compare clean runs
  and state how many clean repeats exist. If the A-vs-A spread is comparable to
  the A-B gap, the verdict is undetermined, not a regression.
- With load above about 4, absolute numbers are diagnostic; ratios count.
- Compare against the previous round's exact frozen binary, never a
  reconstruction.
- Claim gains only from the real workload, never from a micro-bench: they
  disagreed by 3x, and a micro-bench win (views page batching) regressed the
  full bench by 21-36% and was reverted. Count real calls with a temporary
  counter before scaling a micro result.
- Changing the workload (tracking range, view distance) is not a speedup.
  Compare frontier points only with equal useful activity.
- An optimisation with no measurable change in the stage it targets is
  reverted.
- Report CPU time per tick (per-thread `/proc/self/task/*/stat`), not only wall
  time. In the old load scenarios server and bots shared one Tokio runtime, so
  only the tick thread's CPU was attributable; clean server-only CPU needs bots
  in a separate process.
- An unpinned server with 23 threads under a 6-core quota was CFS-throttled in
  856 of 929 periods (2026-10-10); pinned, 0. Check `cpu.stat` throttling before
  reading tick tails.

## Profiling recipes

- **Frame-pointer release build** for attribution:
  `CARGO_TARGET_DIR=<work>/targets/fp RUSTFLAGS="-C force-frame-pointers=yes"
  CARGO_PROFILE_RELEASE_DEBUG=line-tables-only <work>/solaris/tools/dev/mem-run.sh -m 12G env
  RUSTC_WRAPPER= cargo build --release --bin solaris-v2 --bin player-scale-bots`;
  record the SHA-256 of `solaris-v2`.
- **perf:** `perf report -i perf.data --no-children --comm mc-core-tick --sort sym
  --stdio -g none --no-inline` for self time, `--children` for inclusive.
  Inlined frames mislead attribution (a 100-row collect once looked like 9 ms);
  confirm with `--no-inline` and `--symbol-filter=<sym> -g caller` before blaming
  a function. 1% of samples is about tick_ms x tick-thread share / 100.
  `perf --call-graph dwarf` on builds without frame pointers leaves Rust caller
  frames `[unknown]`.
- **Attaching:** `samply record -p <pid> -d <secs> -s -o /abs/profile.json.gz`
  or `samply import perf.data` (Firefox Profiler format). Attaching to your own
  processes needs `kernel.perf_event_paranoid=1` (the owner sets it; ask). gdb
  and perf attach were blocked on some days (`ptrace_scope=1`,
  `perf_event_paranoid=4`).
- **Profiling a harness-run server:** pass a wrapper script as `--server-bin`
  that runs `perf record -F 999 --call-graph dwarf,16384 -o <dir>/perf-$$.data
  -- <frozen-bin> "$@"` only when the arguments contain ` --config `, and plain
  `exec <frozen-bin> "$@"` otherwise (export and converter calls use the same
  binary). Never poll for the PID to attach. DWARF captures are about 2 GB per
  minute: read with `perf report -g none --no-children --sort comm,sym`, then
  delete.
- **Phase timers:** `MC_CORE_PHASE_LOG=1` prints per-phase and per-section
  milliseconds in `server.log`. If the dashboard phases add up to much less than
  the tick total, the time is in untimed sections (spawning, inputs, chunk
  ensure/evict, commit, checkpoint capture).
- **Worldgen:** prefer the test-only stage clocks in
  `crates/mc-worldgen/src/terrain/profile_tests.rs` over call graphs; to split a
  stage add an `end(new_index)` call and widen the totals. Use `--exact`
  (otherwise `vanilla_stages` also matches the other modes). Reference
  2026-10-29 (laptop GPU, `-p` lane): alone 7.80-8.98, shared solids 6.42-7.00,
  GPU density 4.08-4.68 ms/chunk; run-to-run variance 2-5%. Feeding neighbour
  solids to the ore halo halves ore cost; GPU batches of one chunk cost about
  5 ms of fixed overhead, so batch by spatial tiles. Offline `world pregenerate`
  under `-p` is pregeneration throughput, never live serving; live comparisons go
  through a canonical harness profile.

## Optimisation rounds (native-100 pattern, 2026-10)

1. Agents land slices with disjoint file ownership in their own target dirs on
   the games disk (not `/tmp`).
2. Main broadcasts `FREEZE rN` to active agents; agents with uncompiled edits
   restore their last compiled state first. Agents never self-release: only
   Main's `BUILD DONE rN`.
3. Joint gate on the combined tree (strict clippy for touched crates, focused
   tests), check free disk, build the frame-pointer binaries, record the SHA.
4. Run the canonical workload with the fixture cache (prepared world reused;
   saves about 20 minutes) under `-x`, with perf recording.
5. Read the result windows: TPS = ticks over the full window length, not the
   first-to-last capture span; the 10 TPS acceptance is completed ticks per
   window, not p50 under 100 ms (the tick distribution has a heavy tail). An
   `INFRA_BLOCKED` result (for example ENOSPC) is invalid; a gameplay-oracle
   failure at low TPS can be the bot dropping actions while the server lags
   (check the bots report's dropped-action reasons) rather than a server bug.
6. Assign the next round per measured hotspot.

Durable lessons from those rounds: quiescence caches must keep live timers
(burning, fire damage, portal cooldowns) and cache only geometric absence;
per-tick fields must be projected from a tick anchor instead of journaled
(see [lessons](lessons.md)); transparent huge pages gave no gain and were
rejected.

## Load harness and the frozen old core

- `tools/coreperf/compare.py` takes `--mobs`. Cohorts are identified by a
  fixture custom-name marker. The old core kicks setup bots above its ingress
  budget: pace preparation identically for both. `--prepare-only` durability
  gates every comparison; if it fails, stop.
- 100k-mob runs need about 8 GB of cgroup peak including page cache though the
  server's RSS is about 2.3 GiB.
- Treat any non-PASS or missing receipt as failure; existing receipts are
  skipped by the runner. Non-smoke MIX recipes enforce minimum window lengths
  (MIX-18: steady and burst at least 33 s); `--readiness-seconds` applies per
  region.
- Tar a worktree's `.analysis/` before removing it, or its receipts are lost.
