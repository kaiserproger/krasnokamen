# Heavy commands, memory and disk on the shared machine

The rules (which commands go through the runner, which `-m` cap to use,
`env RUSTC_WRAPPER=`, when `-p`/`-x` are allowed) live in
[AGENTS.md](../../AGENTS.md) "Build and validation" and the harness's own
cgroup bounds in [AGENT_TOOLING.md](../AGENT_TOOLING.md) "Resource bounds".
This page explains how the runner works, how to diagnose it and what went
wrong before.

## The machine

One owner laptop: 6 physical / 12 logical cores, about 30 GB RAM, root disk
plus the games disk `<work>` (see [README](README.md)). The agent host process
holds every subagent; with about 20 agents and parallel cargo builds it once
reached 15 GB RSS and was OOM-killed, which killed every agent at once
(2026-10-02). Worktrees survived; agent state did not. Everything below exists
to keep that from happening again without idling the machine.

## How `tools/dev/mem-run.sh` works (current, since 2026-10-28)

`tools/dev/mem-run.sh [-x] [-p] [-m SIZE] <command...>` (flags in this order):

- **No slots, no admission queue** (owner, 2026-10-28). Every command starts
  at once in its own transient systemd user scope inside
  `solaris-heavy.slice` (slice `MemoryMax=25G`, `MemorySwapMax=2G`,
  no `MemoryHigh`, no `CPUQuota`). Default cap is `-m 8G`; above 12G without
  `-x` the request is lowered to 12G.
- **Scope properties.** `MemoryMax=<cap>`, `MemorySwapMax=1G`,
  `CPUQuota=<physical cores x 100>%`, `CPUWeight=50`, `nice 19`. These are the
  harness's own scope properties, so harness runs keep their canonical CPU
  budget. The runner exports `SOLARIS_HARNESS_CPU_SCOPE=1`; without it the
  harness re-runs itself in its own scope outside the slice and escapes the cap.
- **`tools/dev/heavy-watchdog.py`** (user unit `solaris-heavy-watchdog`,
  started by the runner) is the only thing that pauses work. On real memory
  shortage (slice PSI trigger or the slice hitting `MemoryMax`) it freezes the
  youngest *preemptible* command with the cgroup v2 freezer (paused, not
  killed). Preemptible is decided once from argv: plain `cargo
  build/check/clippy/doc/fetch` and `cargo test/bench --no-run`. Tests, harness
  runs, servers and clients are never frozen, so their timeouts stay honest;
  the oldest running command is never frozen. When commands exit it thaws
  frozen ones oldest first while their expected growth fits `MemAvailable`
  minus a 4 GiB reserve.
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
  (`SOLARIS_SLOT_TARGETS` overrides the root), preferring the slot its source
  tree used last. On a tree switch every tracked file of the new tree is
  touched so its workspace crates rebuild (see the 2026-11-04 entry in
  [lessons](lessons.md)). With all 26 busy the command is refused.
- **Incremental** compilation is forced on for debug/test builds and off for
  `--release`/`--profile`. Measured once: cold `mc-core --tests` 26m48s to 19m07s.
- **Lanes.** `-p` is the perf lane: one at a time (perf lock), pinned to CPUs
  8-11 with `CPUWeight=10000`, nice 0, never frozen; for A/B benches and
  profiles, never for builds. Compare `-p` numbers only with `-p` numbers.
  `-x` is a quiet measurement window: the watchdog freezes every preemptible
  command, the runner waits for the rest to finish and holds new commands at
  start until it ends. `-x` is about quiet, not memory.
- **Exit 75** now means one of: the slice has no cgroup, `-x` without a running
  watchdog, or all 26 target dirs busy. The only other legitimate wait at start is a
  `-p` command queued on the serialized perf lane (`flock $dir/perf`); anything else is a regression.
- The runner prints one `mem-run: scope ...` line (props, lane, target, expected
  peak, signature). Copy it into receipts.

## Diagnosing "are they stuck?"

Answer with counts from one pass over `/proc` and the cgroup tree; never poll.

1. **Waiting before start.** `bash` processes whose argv contains `mem-run.sh`
   and that are not the inner `bash -c` wrapper. A child `flock` means a lock
   wait; legitimate ones are `exclusive.lock` while a `-x` window runs and the
   perf-lane lock for queued `-p` commands. Never kill a queued `-p` run.
2. **Running and frozen.** Slice cgroup:
   `systemctl --user show solaris-heavy.slice -p ControlGroup --value`. Per
   `*.scope`: `cgroup.freeze` (1 = requested), `cgroup.events` (`frozen 1` =
   confirmed), `memory.current`, comms in `cgroup.procs`. Reasons:
   `journalctl --user -u solaris-heavy-watchdog -o cat -n 20`.
3. **CPU cap.** Slice `cpu.max` must be `max`; check `nr_throttled` vs
   `nr_periods` in `cpu.stat`. Runtime drop-ins live under
   `/run/user/<uid>/systemd/user.control/solaris-heavy.slice.d/`; clear a cap
   with `systemctl --user set-property --runtime solaris-heavy.slice CPUQuota=`.
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

- "SIGTERM, no compiler error" or exit 143 is usually an OOM inside the scope:
  `journalctl --user --since -30min | grep -E 'run-.*scope|oom'`. Lower
  `--jobs` before raising the cap; a build whose `memory.peak` equals its cap
  will fail next time.
- Host-level: `journalctl -k --since -3h | grep -E 'Killed process|oom-kill:'`
  names the victim (`omp` = agent host; `rustc`/test binary = a scope).
- Prove a cap really applies with a `RUSTC_WRAPPER` script that appends
  `/proc/self/cgroup` to a file; a bash smoke test proves nothing.
- Never set `MemoryHigh` on the slice: reclaim pressure is what systemd-oomd
  kills on, and it picks the largest units (terminal, agent host).
- The global `~/.cargo` sccache wrapper compiles outside the cap; that is why
  every Cargo command is `env RUSTC_WRAPPER=`. An sccache daemon first started
  inside a scope keeps that scope populated forever; start it outside
  (`systemd-run --user --scope --slice=app.slice sccache --start-server`).
- rust-analyzer (about 5 GB, agents auto-start it) and the agent host (about
  4 GB) sit outside the slice. Killing rust-analyzer, leaked Minecraft clients
  and idle Gradle daemons is safe; codegraph serves the MCP session, keep it.
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
`SOLARIS_MEM_RUN_CPU_QUOTA` (e.g. `400`), and tell agents to use
`-m 6G nice -n 19 env RUSTC_WRAPPER= cargo ... -j 2`, `--test-threads 2`,
focused `-p` crates only. A slice-level `CPUQuota` does not hold: the runner
clears it on every call (a forgotten one held every heavy command to 2 CPUs
from 2026-10-26 to 2026-10-28). Find rustc processes outside the slice
(`/proc/<pid>/cgroup`) and trace their cwd to the agent.

## Disk

- Nothing new on the root disk (owner, 2026-10-23): worktrees in `<work>/wt/`,
  target dirs in `<work>/targets/`, sccache on the games disk. `/tmp` is on the
  root disk. `git worktree move` fails across devices; use rsync.
- `/dev/shm` has a per-user quota: `df` free space is not usable headroom.
- Keep ample free space on the games disk (at least 50 GB for load runs; R&D
  scale runs used a 60 GiB floor). The disk filled to 100% several times:
  per-run plugin `.wasm` copies in scenario runs (7 of 11 GB on 2026-10-28),
  650 MB of client content copied into every scene-shot receipt (now
  symlinked), nested `sdk/rust/target` in rsync'd worktrees, 66 GB of stale
  debug targets.

Cleanup while agents run:

- **Worktrees:** removable only if its agent finished, it has no tracked
  changes (a stray `.pyc` aside) and its HEAD is in a branch
  (`git branch --contains`). Check handoffs/configs for absolute paths into it
  first (e.g. an ALSA pkg-config shim, a `SOLARIS_LOADER_ROOT`). Move its
  `.analysis/` to `<work>/receipts/<wt>-analysis` and publish outputs to
  `<work>/release-artifacts/` (call them "built", never "published"), then
  `git worktree remove --force` and `git worktree prune`.
- **Target dirs:** candidates only with zero users across `/proc` (cwd, exe,
  fds, `CARGO_TARGET_DIR=` in environ); re-check right before `rm`. Delete a
  pooled `slot-<l>` only under its lock:
  `flock -n /tmp/solaris-mem-slots/<l> rm -rf <work>/targets/slot-<l>`. Keep
  dirs named in active tasks and dirs holding binaries other agents run.
- **Receipts:** delete duplicated `.wasm`, `world/`, `pristine-world/`,
  `journal/` and `xdg/` of finished runs; keep logs, worlds you still need and
  JSON receipts. Never delete `result.json`, logs or `perf.data` you cite.
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

Skills or prompts that mention slots, "3 x 5G", "-m 5G", exit 75 as a memory
refusal, or CPU 2/3 windows are obsolete; AGENTS.md and this page win.
