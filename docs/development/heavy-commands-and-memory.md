# Heavy commands, memory and disk on the shared machine

The rules (which commands go through the runner, which `-m` cap to use,
the sccache wrapper, when `-p`/`-x` are allowed) live in
[AGENTS.md](../../AGENTS.md) "Build and validation" and the harness's own
cgroup bounds in [AGENT_TOOLING.md](../AGENT_TOOLING.md) "Resource bounds".
This page explains how the runner works, how to diagnose it and what went
wrong before.

The runner's requirements and fresh-machine setup (systemd user scope, cgroup
v2, `/tmp/solaris-mem-slots`, the last four CPUs for the `-p` lane, Landlock for the
instrumentation guard) are in [`tools/dev/README.md`](../../tools/dev/README.md).
Other scripts there: `scene-slot.sh` (graphical gate slots, see
[real-client gates](real-client-gates.md#3-environment-block)),
`instrumentation-guard.py`, `test-server/` ([test server operations](test-server-ops.md))
and `winqa/` ([Windows QA](windows-and-macos-client-qa.md)).

## The machine

The owner host is a shared build machine, not necessarily the owner's terminal
or laptop ([AGENTS.md](../../AGENTS.md)). Derive current CPU topology, host
memory and parent/slice limits from the runner and cgroups, not an old hardware
snapshot. The agent host contains its subagents outside the heavy-job slice.
Historical laptop incident, 2026-10-02: about 20 agents plus builds drove that
process to 15 GiB RSS; an OOM killed all agents, while worktrees survived.

## How `tools/dev/mem-run.sh` works (current, since 2026-10-28)

`tools/dev/mem-run.sh [-x] [-p] [-m SIZE] <command...>` (flags in this order):

- **No memory slots.** Commands enter their own transient systemd user scope;
  compiles first wait for a build token, and measurement/scene locks can delay
  execution. [`mem-run.sh`](../../tools/dev/mem-run.sh) derives the heavy slice's
  `MemoryMax` from host RAM minus 16 GiB and, when set, the parent `MemoryHigh`
  minus 8 GiB, taking the lower bound. It clears slice `MemoryHigh` and any CPU
  quota not explicitly pinned by the owner. Default command cap is `-m 8G`;
  requests above 12 GiB without `-x` are lowered to 12 GiB. These are runner defaults,
  not a promise about every machine's effective parent budget.
- **Build tokens** (owner, 2026-11-11): at most three compile commands run
  host-wide; the fleet's "3 concurrent builders" is enforced here, not by
  convention. Compile is decided once from argv, after leading
  `env`/`nice`/`ionice`/`timeout` words: `cargo` with any subcommand except
  `fmt`, `fetch`, `tree`, `metadata`, `clean`, `update` and similar, `gradle`
  and `gradlew`, `pin-release`, harness `run build|clippy|code-health|
  correctness|test|regressions|regressions-client|java|standard-pack` and
  harness `ledger probe|promote`. Running built binaries, gates and servers
  takes no token and starts while all three are held; a build hidden inside
  `sh -c` is not seen. A compile re-execs the runner under
  `tools/dev/slot-lock.py` (the same helper as `scene-slot.sh`), which blocks
  on all three locks `/tmp/solaris-mem-slots/tokens/token-{1,2,3}.lock` at once
  and takes whichever frees first: no polling, no CPU while waiting, and the
  runner prints `mem-run: build tokens 1,2,3 are in use; waiting`. With
  `KRASNOKAMEN_BUILD_LANE=background` in the runner's environment a compile waits
  only for tokens 1-2, so token 3 stays free for priority work (owner,
  2026-11-12: the battle video gets resources first), and a freed token goes to
  a waiting normal-lane compile first (owner, 2026-11-18): every normal-lane
  waiter holds a shared lock on `tokens/priority.gate`, and a background waiter
  that wins a token while the gate is held gives it back and queues again once
  every waiting priority compile has won. Main exports the background lane for every agent outside the priority
  task. The lock
  stays at fd 9 of the runner and its scope guardian, never the command, so
  the token returns once the scope is empty: on any exit, kill or OOM. The
  `mem-run: scope` line shows `token=<n>`, `token=<n>(reused)` or
  `token=none`. **Nesting:** the runner exports `KRASNOKAMEN_BUILD_TOKEN` and
  `KRASNOKAMEN_BUILD_TOKEN_PID` (its own pid); a compile whose ancestor process is
  that runner reuses its token, so a harness run or pin-release that runs
  `mem-run.sh ... cargo` again inside never waits for a second token (three
  outer holders would deadlock). The token is taken before the `-x` lock, so
  queued compiles never delay a `-x` window; a `-p` compile holds its token
  while it queues for the perf lane. Holders, their command and tree, and the
  waiters: `tools/dev/fleet-status`. Tests:
  `python3 tools/dev/test_build_tokens.py`.
- **Killed jobs:** an independent guardian kills the job's scope and holds its locks (slot locks and the build token) until the scope is empty.
- **Lock order: scene slot, then `-x`** (2026-11-14). Gates hold a scene slot while they wait for
  the `-x` lock (shared), so a `-x` window that then waits for a scene slot deadlocks every
  build and gate (a window did so for about 50 minutes). Take the slot outside the window:
  `scene-slot.sh mem-run.sh -x <command>`. Inside a window the runner exports
  `KRASNOKAMEN_EXCLUSIVE_WINDOW`; `scene-slot.sh` then runs a command at once when an ancestor
  already holds a slot (`KRASNOKAMEN_SCENE_SLOT_PID`) and otherwise refuses with exit 75. Call
  `scene-slot.sh`, never `slot-lock.py` with the scene lock names directly. Tests:
  `python3 tools/dev/test_scene_slot.py`.
- **Release builds are pins** (owner, 2026-11-15). One release binary belongs to one
  integrated commit: `cargo build|test|run --release` and `--profile release*` are
  refused by the runner, before it takes a slot or a build token, unless
  `tools/dev/pin-release` asks for it (it sets `KRASNOKAMEN_PIN_RELEASE`), the caller
  is the harness/ledger tooling (real-client gates and ledger promote's own candidate
  and control trees, which pin their own tree), or Main overrides one run with
  `KRASNOKAMEN_RELEASE_BUILD_ALLOW=1`. The refusal names `tools/dev/pin-release`:
  pin the commit once and pass the printed `KRASNOKAMEN_SERVER_BINARY` /
  `KRASNOKAMEN_CLIENT_BINARY` / `KRASNOKAMEN_BOTS_BINARY` to the gate or measurement.
  `pin-release` reuses the pin of the same tree and the same build parameters
  (source inputs, repo, kind, target, incremental flag and the codegen environment
  cargo records) instead of rebuilding, and gives a different tree — a ledger control,
  a branch — its own pin, never the shared one. Debug builds, clippy and every
  non-release command are unchanged. Tests:
  `python3 tools/dev/test_mem_run_release_policy.py`.
- **`-x` runs only inside a scheduled measurement window** (owner, 2026-11-15). A `-x`
  window freezes build-only commands and waits for every running test and server, so it
  is a measurement, not a convenience flag. The schedule is
  `tools/dev/measurement-windows`: local recurring `HH:MM-HH:MM` and one-off
  `YYYY-MM-DDTHH:MM/YYYY-MM-DDTHH:MM` intervals. One-offs may cross midnight
  and expire; they do not open the same hours on other dates. The recurring
  baseline is `01:00-02:30`, before the normally03:00 nightly fuzz (parked
  during Stage3); dated allocations follow the existing announcement plan.
  Outside every window the runner refuses with exit75 and names the next
  dated start; an exhausted one-off schedule refuses too. Main may override
  an explicitly authorized run with `KRASNOKAMEN_WINDOW_OVERRIDE=1`, never
  silently to bypass a changed allocation. The check precedes scene-slot
  and `-x` locks, so a refusal never holds anything. Tests:
  `python3 tools/dev/test_measurement_window.py`.
- **Scope properties.** `MemoryMax=<cap>`, `MemorySwapMax=1G`,
  `CPUQuota=<physical cores x 100>%`, `CPUWeight=50`, `nice 19`. These are the
  harness's own scope properties, so harness runs keep their canonical CPU
  budget. Preemptible builds (below) get `CPUWeight=10`, so under contention
  tests and servers win the CPU five to one and builds only slow down. A
  command started through `scene-slot.sh` (it exports `KRASNOKAMEN_SCENE_SLOT`)
  gets `CPUWeight=500`: with dozens of test scopes at 50, a gate's native
  client fell to 6-8 TPS and missed its tick waits (2026-11-11). The runner exports `KRASNOKAMEN_HARNESS_CPU_SCOPE=1`; without it the
  harness re-runs itself in its own scope outside the slice and escapes the cap.
- **`tools/dev/heavy-watchdog.py`** (user unit `solaris-heavy-watchdog`,
  started by the runner) is the only thing that pauses work. On real memory
  shortage (slice PSI trigger or the slice hitting `MemoryMax`) it freezes the
  youngest *preemptible* command with the cgroup v2 freezer (paused, not
  killed). Preemptible is decided once from argv: plain `cargo
  build/check/clippy/doc/fetch` and `cargo test/bench --no-run`, also behind a
  leading `env NAME=value …` (pin-release, per-tree `CARGO_TARGET_DIR`). Tests, harness
  runs, servers and clients are never frozen, so their timeouts stay honest;
  the oldest running command is never frozen. Room is the smaller of
  `MemAvailable` and the slice cap minus its unreclaimable memory (tmpfs, dirty
  and writeback pages count; clean page cache that builds leave in the slice is
  dropped by the kernel at the cap and does not), each minus a 4 GiB reserve. A
  PSI or `MemoryMax` event freezes only while room is negative. When commands
  exit it thaws frozen ones oldest first while their expected growth fits the
  room. A watchdog restart while builds run keeps the shared jobserver as it is.
  Tests: `python3 tools/dev/test_heavy_watchdog.py`.
- **Expected peaks are measured.** Every successful command appends its scope's
  `memory.peak` to `/tmp/solaris-mem-slots/peaks` under an argv signature
  (paths and leading `env` words removed). The next run expects 1.25 x the
  largest of its last five peaks (unknown: 4 GiB), never above its cap.
- **Shared jobserver.** The watchdog owns a make-style jobserver FIFO
  (`/tmp/solaris-mem-slots/jobserver`, one token per logical CPU) passed to
  cargo via `CARGO_MAKEFLAGS`, so all builds together run about `nproc`
  compilers instead of `nproc` each.
- **Pooled target dirs.** A command that sets `CARGO_TARGET_DIR` keeps it.
  Otherwise it gets a free shared dir `<work>/targets/slot-a..slot-z`
  (`KRASNOKAMEN_SLOT_TARGETS` overrides the root): the slot its source tree used
  last, else any existing free slot, else a new one if `df` reports at least
  `KRASNOKAMEN_SLOT_MIN_FREE_GB` (default 250 GiB, the free-space floor below; rounded up) free. This check does
  not reserve space or limit growth of existing slots. Each running command
  holds its own slot (15-40 GB once built): on 2026-11-07 agents running
  several commands in parallel created 12 slots and filled the disk, so run one
  heavy command at a time per agent. On a tree switch every tracked file of the
  new tree is touched so its workspace crates rebuild (see the 2026-11-04 entry
  in [lessons](lessons.md)). The tree is the git toplevel of the cwd when the
  runner starts, so one runner command builds one tree: a script that `cd`s into
  a second worktree inside the same command reuses the first tree's slot without
  the switch, and Cargo (same unit hash for a workspace member in any checkout)
  runs the other tree's binary when that binary is newer than this tree's
  sources (a false test result, 2026-11-14). Run one runner command per tree, or
  give each tree its own target directory under `<work>/targets/`.
- **Incremental:** use the single [argv/build-policy description below](#oom-and-killed-commands);
  normal-lane release pins are not automatically nonincremental. The historical
  cold `mc-core --tests` comparison was 26m48s→19m07s, not a general speed guarantee.
- **Lanes.** `-p` is the perf lane: one at a time (perf lock), pinned to the
  last four CPUs with `CPUWeight=10000`, nice 0, never frozen; for A/B benches and
  profiles, never for builds. Compare `-p` numbers only with `-p` numbers.
  `-x` is a quiet measurement window: the watchdog freezes every preemptible
  command, the runner waits for the rest to finish and holds new commands at
  start until it ends. `-x` is about quiet, not memory. A command started inside
  an already running scope (a ledger promote's server build and gates) is not
  held and never frozen: the window waits for its ancestor, so holding it would
  deadlock both.
- **Exit75** is a refusal, not an executed gate: read the named cause (for example
  no cgroup/watchdog, no scheduled `-x` grant or no usable target directory).
  Build-token, scene-slot, exclusive-window and perf-lane admission have distinct
  owners; waiting for one is not evidence of a dead process. Consume its
  completion event; do not poll or bypass it with another lane.
- The runner prints one `mem-run: scope ...` line (props, lane, target, expected
  peak, signature). Copy it into receipts.
- **One Cargo environment per tree.** Run Cargo only through the runner and keep
  the same environment for every build on the shared host. A plain `cargo build`
  after a runner build of the same tree rebuilt the workspace for 84 s (the runner
  sets `CARGO_INCREMENTAL`), and any `CARGO_PROFILE_*` override is a separate
  profile with its own artifacts. The dev/test profiles already use line-table
  debuginfo, so no per-command debuginfo override is needed.

## Diagnosing "are they stuck?"

Answer with counts from one pass over `/proc` and the cgroup tree; never poll.

1. **Waiting before start.** `tools/dev/fleet-status` lists the build-token
   and scene-slot holders and waiters. Otherwise: `bash` processes whose argv
   contains `mem-run.sh` and that are not the inner `bash -c` wrapper. A child
   `flock` means a lock wait; legitimate ones are `exclusive.lock` while a `-x`
   window runs and the perf-lane lock for queued `-p` commands; a
   `slot-lock.py` process is a compile waiting for a token. Never kill a
   queued `-p` run.
2. **Running and frozen.** Slice cgroup:
   `systemctl --user show solaris-heavy.slice -p ControlGroup --value`. Per
   `*.scope`: `cgroup.freeze` (1 = requested), `cgroup.events` (`frozen 1` =
   confirmed), `memory.current`, comms in `cgroup.procs`. Reasons:
   `journalctl --user -u solaris-heavy-watchdog -o cat -n 20`.
3. **CPU cap.** Slice `cpu.max` is `max` unless the owner pinned a quota in
   `/tmp/solaris-mem-slots/cpu-quota` (one line such as `600%`; the runner
   applies it on every call and clears any other slice quota). Check
   `nr_throttled` vs `nr_periods` in `cpu.stat`. Runtime drop-ins live under
   `/run/user/<uid>/systemd/user.control/solaris-heavy.slice.d/`. An owner-pinned
   cap is not permission to remove it; change it only under owner authorization.
   Host `/proc/pressure/cpu` near 80% while `vmstat` shows >40% idle means a cap
   or pinning, not load.
4. **Pinning.** `Cpus_allowed_list` of cargo/rustc; widen live with
   `os.sched_setaffinity` on every `/proc/<pid>/task/*`.
5. **Memory.** `MemAvailable`, slice `memory.current`, `memory.events`, swap.
6. **Cargo's own lock.** A cargo with `wchan=locks_lock_inode_wait` holds an fd
   on `<target>/.cargo-build-lock`; match its inode in `/proc/locks` (the `->`
   line is the waiter). Two commands share one `CARGO_TARGET_DIR`; it resolves
   when the holder finishes and is not a runner queue.
7. **Agent liveness.** An agent whose last call is a long wait is idle on a
   build, not dead. See [agent fleet](agent-fleet.md).

Fixes: kill stuck waiters by PID only (never by name pattern), including their
`flock` children, then tell the affected agents to rerun anything that exited
before printing its `mem-run: scope` line. To fix a wrong freeze: stop the
watchdog unit, write 0 to every frozen scope's `cgroup.freeze`, swap the fixed
code, start the unit (agents' runner calls may restart it in between).

## OOM and killed commands

- A signal or exit143/137 alone identifies neither OOM nor owner cancellation.
  Correlate the exact child/scope with `memory.events`/`memory.peak` and the user
  journal; kernel OOM records identify host-level victims. Keep unobserved causes
  unknown and retain the failing receipt. Lower compile concurrency before raising
  a proven-insufficient cap; rerun only the affected gate after that correction.
- Host-level: `journalctl -k --since -3h | grep -E 'Killed process|oom-kill:'`
  names the victim (`omp` = agent host; `rustc`/test binary = a scope).
- Prove a cap really applies with a `RUSTC_WRAPPER` script that appends
  `/proc/self/cgroup` to a file; a bash smoke test proves nothing.
- Never set `MemoryHigh` on the slice: reclaim pressure is what systemd-oomd
  kills on, and it picks the largest units (terminal, agent host).
- sccache: the runner exports `RUSTC_WRAPPER=tools/dev/sccache.sh` and
  `SCCACHE_CLIENT_SIDE=1` when the user unit `solaris-sccache.service` (app.slice,
  `client_side_mode`: the daemon only fronts the cache, every compile runs in the
  caller's scope) is active and `~/.cargo/bin/sccache` exists, else
  `RUSTC_WRAPPER=` (uncached). Never stop, restart or reconfigure that unit, never
  start an sccache inside a scope (it would stay populated by that scope), and
  compare `sccache --show-stats` before and after instead of zeroing it. Measured
  2026-11-11, one run: a cold debug `krasnokamen` in another tree against a warm
  cache took 4542 -> 3316 G instructions (`perf stat`, -27%; 213/271 Rust hits plus
  284 C hits), and populating an empty cache cost +6%. The same for a cold release
  `krasnokamen`: 5482 -> 4707 G instructions (-14%), populating +1%. Each is one
  measurement under those conditions, not a general speedup. Expected to help on cold or
  private target dirs, after slot rotation and after toolchain changes; a warm slot
  gains nothing. Plain
  `RUSTC_WRAPPER=sccache` is wrong here: sccache hashes every `CARGO_*` variable
  into Rust keys and the runner's per-slot `CARGO_TARGET_DIR` made every key
  differ (0 hits across slots), it refuses `CARGO_INCREMENTAL=1` (which the runner
  sets for debug builds), and a wrapper not named `sccache` leaves C/C++ build
  scripts (aws-lc-sys, zstd-sys) uncached. `sccache.sh` unsets those two variables
  (cargo passes their effect to rustc as arguments) and keeps the stem `sccache`.
- Incremental (`CARGO_INCREMENTAL`) is set by the runner from the argv: 1 for
  debug/test builds and for `--release` builds in the normal lane, 0 in `-p`/`-x`
  lanes and for any `--profile` build. One re-pin measurement (2026-11-11) used
  less CPU with incremental, but host load was not recorded for the full-build
  runs, so only the direction is plausible, not the size. It needs about 2.4 GB
  more per slot and uses more codegen units, so its binary can run slower: take
  performance binaries from `pin-release --no-incremental`, never an ad-hoc
  release build. Its manifest records the effective build environment.
- Agent hosts and language servers may sit outside the heavy slice: include
  parent/host accounting. Do not blanket-kill shared analysis servers, Gradle or
  clients to reclaim RAM; stop only verified owned/authorized processes under the
  [fleet recovery rules](agent-fleet.md#liveness-and-recovery).
- Agent tool calls default to a 300 s deadline: run long commands with
  `async: true` plus `timeout: 0`/`3600`, never by changing harness timeouts.

## Changing the runner or watchdog

- Replace scripts atomically: write a copy, `bash -n`, then `mv` over the
  original. Running bash callers read the file incrementally; an in-place `cp`
  made waiters execute spliced text (`syntax error near '('`,
  `ty: command not found`).
- Test lock or admission changes on a copy with a private lock dir, FIFO gates
  and `/proc/locks` waiter counts filtered by the lock files' inodes; cover
  ordering, `-x` exclusivity and `kill -9` of a waiter mid-wait. Run old and
  new to show FAIL then PASS.
- Read scope files with `2>/dev/null || continue`: scopes vanish between glob
  and read. Treat a scope as gone only when `cgroup.events` says
  `populated 0` (empty `cgroup.procs` misses child cgroups).

## When the owner says the machine is choking

Owner-approved response (2026-10-24), adjusted for the current runner: renice
running rustc/cargo/java/qemu to 19, start new heavy commands with a lower
`KRASNOKAMEN_MEM_RUN_CPU_QUOTA` (e.g. `400`), and tell agents to use
`-m 6G nice -n 19 cargo ... -j 2`, `--test-threads 2`,
focused `-p` crates only. For a machine-wide cap, write the owner's quota to
`/tmp/solaris-mem-slots/cpu-quota` (e.g. `600%`, replaced atomically); the runner
clears any slice `CPUQuota` not set through that file (a forgotten one held every
heavy command to 2 CPUs from 2026-10-26 to 2026-10-28).
Find rustc processes outside the slice (`/proc/<pid>/cgroup`) and trace their
cwd to the agent.

## Disk

- Nothing new on the root disk (owner, 2026-10-23): worktrees in `<work>/wt/`,
  target dirs in `<work>/targets/`, sccache on the games disk. `/tmp` is on the
  root disk. `git worktree move` fails across devices; use rsync.
- `/dev/shm` has a per-user quota: `df` free space is not usable headroom.
- **Keep at least 250 GiB free on the games disk** (owner, 2026-11-13; raised
  from 80-100 GB set 2026-11-07; it is a DRAM-less Samsung 980 whose fast write
  cache shrinks as it fills: at 92% full writes took over a second each). Below
  the floor, `slot-gc` evicts idle slots in least-recently-used order (below).
  The disk filled to 100% several times: 12 parallel
  target slots (2026-11-07), per-run plugin `.wasm` copies in scenario runs (7
  of 11 GB on 2026-10-28), 650 MB of client content copied into every
  scene-shot receipt (now symlinked), nested `sdk/rust/target` in rsync'd
  worktrees, 66 GB of stale debug targets.
- **Slot cleanup and rotation.** Below `KRASNOKAMEN_SLOT_MIN_FREE_GB` + 10 GiB free
  (260 GiB by default), the runner (except `-x`/`-p`) invokes `slot-gc --delete`;
  failures are logged and do not stop the command. Each pass removes attributable
  dead artifacts and superseded finalized incremental sessions. If space remains
  below the threshold, idle slots are rotated least recently used first: their
  incremental dirs, then the rest except `tmp/` if needed. A slot is skipped
  while its pool lock or its use lock is held (every runner command on the slot
  holds the use lock, an explicit `CARGO_TARGET_DIR` included), or while a live
  process has it as `CARGO_TARGET_DIR`; run cargo through the runner, because a
  bare cargo started after that scan is not protected. Rotation can
  discard live-tree caches; slots ignore the in-use registry. See
  [`tools/dev/README.md`](../../tools/dev/README.md#slot-gc---delete--v-slot).
- **Order when space is short** (owner, 2026-11-16). Free only reproducible
  build output, in this order, and check `df` after each step. Standing owner
  permission (2026-11-17): below the floor, build caches may be cleared whole,
  all three steps, without asking.
  1. Incremental caches of idle slots:
     `tools/dev/slot-gc --delete --incremental-only --min-free-gb <target GiB>`
     (dry run without `--delete` first). Busy slots are skipped as above.
  2. Named `targets/<Agent>` dirs of merged or finished slices with zero users
     (rule under *Target dirs* below). A named dir that also holds inputs (a
     tool checkout, benchmark worlds, pack jars) loses only its cargo target.
  3. Old build outputs of idle slots: full rotation
     (`slot-gc --delete --min-free-gb <target GiB>`).

  Run data is never on this list: crash copies, WAL archives, worlds and
  client state of a run stay unless the owner explicitly approves deleting
  them, and a failing run or negative control stays whole even then. Hashing
  a WAL does not replace it. A `slot-gc` pass above floor + 10 GiB runs only the
  safe classes, which delete nothing a live tree can still use, so it reports
  0 GiB even with hundreds of GiB of idle incremental caches: use step 1.

Cleanup while agents run:

- **Worktrees:** use `wt-done <Name>` (verified `final.patch`, then removal). It
  refuses a worktree another owner marked in the in-use registry
  (`tools/dev/in-use`; `wt-gc` shows the marks), so mark a client/loader checkout
  or a receipt binary you run with `in-use add` and drop it with `in-use remove`.
  By hand, only if its agent finished, it has no tracked changes (a stray `.pyc`
  aside) and its HEAD is in a branch (`git branch --contains`). Check
  handoffs/configs for absolute paths into it first (e.g. an ALSA pkg-config
  shim, a `KRASNOKAMEN_LOADER_ROOT`). Move its `.analysis/` to
  `<work>/receipts/<wt>-analysis` and publish outputs to
  `<work>/release-artifacts/` (call them "built", never "published"), then
  `git worktree remove --force` and `git worktree prune`.
- **Target dirs:** candidates only with zero users across `/proc` (cwd, exe,
  fds, `CARGO_TARGET_DIR=` in environ); re-check right before `rm`. Delete a
  pooled `slot-<l>` only under its lock:
  `flock -n /tmp/solaris-mem-slots/<l> rm -rf <work>/targets/slot-<l>`. Keep
  dirs named in active tasks and dirs holding binaries other agents run.
- **Receipts and scenarios** (owner, 2026-11-07, narrowed 2026-11-16): a
  finished run keeps its reports, logs and `result.json` (text reports such as
  `.md`, `.yaml`, `.json`, `.txt`, plus `.log` files, delivered `.patch` files
  and screenshots). Only provably reproducible inputs may be deleted: copied
  client data roots, binaries whose pin or build is kept elsewhere, jars and
  `.wasm` copies. Crash copies, WAL archives, worlds, `journal/`, recorded
  profiles (`perf.data`) and archives are run data: keep them unless the owner
  explicitly approves, and never delete them from a failing run. Keep the full
  run while its work is active or a named next task needs it.
- Don't touch the owner's main checkouts and their `target/`, sccache, or other
  projects' caches in `/tmp`.

## Superseded runner designs (do not follow old skills)

| Date | Design | Why it changed |
|---|---|---|
| 2026-10-02 | 2 flock slots, 12G slice, 6G per command, admission exit 75, `-x` = both slots | waiters stranded behind a long job; cancelled waiters leaked a slot |
| 2026-10-03/04 | 3 slots x 5G in a 16G slice, CPUs 2/3 reserved for single-core perf windows | admission refused everything while one build ran |
| 2026-10-16 | 25G slice, 3 slots x 8G, `-p` on CPUs 8-11, `-x` acceptance only with Main GO | 20+ agents waited 10+ minutes in admission; admission summed caps of every populated scope, not real usage |
| 2026-10-28 | no slots or queue; watchdog freezer; shared jobserver; measured peaks | current |
| 2026-11-04 | pooled target dirs with per-tree marker and touch on switch | current; fixes false greens |
| 2026-11-11 | three build tokens for compile commands (`slot-lock.py`), reused by nested compiles | current; the 3-builder limit was convention only and the fleet still ran more |

Skills or prompts that mention slots, "3 x 5G", "-m 5G", exit 75 as a memory
refusal, or CPU 2/3 windows are obsolete; AGENTS.md and this page win.
