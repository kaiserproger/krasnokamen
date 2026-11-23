# Heavy commands, memory and disk on the shared machine

[AGENTS.md](../../AGENTS.md#build-and-validation) sets agent policy;
this page owns runner behavior and diagnosis. Harness-only cgroups:
[resource bounds](../AGENT_TOOLING.md#resource-bounds). Installation:
[`tools/dev/README.md`](../../tools/dev/README.md) (systemd user scopes, cgroup v2,
`/tmp/solaris-mem-slots`, Landlock for instrumentation). Related:
[real-client environment](real-client-gates.md#3-environment-block),
[test server](test-server-ops.md), [Windows QA](windows-and-macos-client-qa.md).

## The machine

Derive CPU topology, RAM and effective parent/slice limits from the running
host, not old laptop recipes. Agent hosts/language servers can live outside the
heavy slice; include them in accounting. The 2026-10-02 laptop OOM killed the
15 GiB agent process and all children, not their worktrees.

## How `tools/dev/mem-run.sh` works (current, since 2026-10-28)

Wrap heavy Cargo, harness and Gradle commands in Main's absolute
`<work>/solaris/tools/dev/mem-run.sh`, never a worktree's or sibling repo's copy.
Use one heavy command per agent and one source tree per invocation:
`mem-run.sh [-x] [-p] [-m SIZE] COMMAND ...` (that flag order).
The one-token-per-agent rule and priority gate below remain laptop proposals
of 2026-11-18, pending owner confirmation; see the
[brief limits](agent-fleet.md#briefs). Pass actual build argv, not a build
hidden inside `sh -c`/`bash -c`: classification is not a security boundary.

| Mechanism | Contract |
|---|---|
| Memory | No memory slots. Per-command default 8G, non-`-x` requests capped at 12G; agent policy normally selects 6G/12G. Slice `MemoryMax` is the lower of host RAM−16 GiB and, if set, parent `MemoryHigh`−8 GiB. Slice `MemoryHigh` is cleared. Effective parent limits still apply. |
| Scope | `MemorySwapMax=1G`, CPU quota = physical cores ×100%, `CPUWeight=50`, nice 19. Preemptible builds get weight 10; scene-slot commands 500. `KRASNOKAMEN_HARNESS_CPU_SCOPE=1` keeps harness children in this scope rather than escaping into another. |
| Build tokens | Three host-wide tokens; compiles wait without polling via `slot-lock.py` on `tokens/token-{1,2,3}.lock`. Built binaries/gates/servers need none. `fleet-status` shows holders/waiters. |
| Priority | `KRASNOKAMEN_BUILD_LANE=background` may take any token but yields to normal-lane waiters on `tokens/priority.gate`. Main's priority slice and battle takes/render/pins keep normal lane (proposal status above). D27 p.6 authorized the four-token trial; it ended on 2026-11-18 after 15-minute load stayed >20. D27 p.7 removed token-3 reserve; restore it only for a starved live-take window. |
| Token lifetime | Runner fd 9 and independent scope guardian hold it, never the command. Release only when scope is empty, including kill/OOM. `KRASNOKAMEN_BUILD_TOKEN{,_PID}` permits ancestor-verified reuse by nested compiles, avoiding three-holder deadlock. Scope line says `<n>`, `<n>(reused)` or `none`. |
| Jobserver | Watchdog owns `/tmp/solaris-mem-slots/jobserver`, a make-style FIFO with one token per logical CPU, passed by `CARGO_MAKEFLAGS`; concurrent Cargo builds share roughly `nproc` compilers. |
| Peak estimate | Successful scope `memory.peak` goes to `/tmp/solaris-mem-slots/peaks`, keyed by argv without paths/leading env. Next estimate = 1.25× largest of last five, unknown 4 GiB, bounded by cap. |

Compile classification strips leading env/nice/ionice/timeout wrappers:
Cargo except noncompiling commands (`fmt`, `fetch`, `tree`, `metadata`, `clean`,
`update`, etc.), gradle(w), pin-release, harness
`run build|clippy|code-health|correctness|test|regressions|regressions-client|java|standard-pack`
and `ledger probe|promote`. Nested wrappers recognized by the live script remain
authoritative. Token is acquired before `-x`; queued compiles cannot block a
quiet window. A `-p` compile would hold its token while queued: **do not build in `-p`**.

**Locks and lanes**

- Take scene slot **before** exclusive lock: `scene-slot.sh mem-run.sh -x COMMAND`.
  The reverse order deadlocked the host for ~50 minutes (2026-11-14).
  In `KRASNOKAMEN_EXCLUSIVE_WINDOW`, `scene-slot.sh` reuses an ancestor's
  `KRASNOKAMEN_SCENE_SLOT_PID` or refuses exit 75; never acquire scene lock names
  directly through `slot-lock.py`.
- `-p`: serialized perf lock, last four CPUs, weight 10000, nice 0, never frozen;
  compare only matching perf-lane runs.
- `-x`: quiet measurement, not extra-memory convenience. Freeze preemptible
  builds, wait for running tests/servers, hold new work until completion.
  Descendants of already-running scopes continue, or the ancestor wait deadlocks.
  Admission uses `tools/dev/measurement-windows`: recurring local intervals and
  dated one-offs (may cross midnight; never recur after expiration).
  Baseline recurring `01:00-02:30` precedes normally-03:00 nightly fuzz (parked
  during Stage3); the current announcement plan owns dated allocations.
  Outside a grant, refuse 75 and name next start, before acquiring locks.
  Only Main's explicitly authorized `KRASNOKAMEN_WINDOW_OVERRIDE=1` overrides.
- A guardian kills cancelled jobs' scopes and retains locks until empty.
  Cancel the parent job, not just children. On a wrapper signal the scope gets
  `TERM` directly first, then every `cgroup.procs` member; the guardian
  escalates to `cgroup.kill` (`SIGKILL` to the whole scope, the wrapper
  included) after `KRASNOKAMEN_MEMRUN_CANCEL_GRACE` seconds, default 15 — a
  harness still writing `result.json` at that deadline is cut with its scope.
  If the guardian fifo is unwritable the runner says so on stderr and the
  `TERM` pass still applies.
- Exit 75 is admission refusal (named cgroup/watchdog/window/target cause), not
  an executed gate. Token, scene, exclusive and perf waits have different owners;
  waiting is not death. Consume completion, never poll or bypass another lane.

**Release and build identity**

- Ad-hoc `cargo build|test|run --release`/`--profile release*` is refused before
  token/slot admission. `pin-release`, harness/ledger candidate-control tooling,
  or Main's explicit `KRASNOKAMEN_RELEASE_BUILD_ALLOW=1` may request it.
  Reuse the immutable pin only for identical source/repo/kind/target/incremental/
  codegen inputs. Each reverted control or branch gets its own pin. Gates consume
  the printed `KRASNOKAMEN_SERVER_BINARY`, `CLIENT_BINARY` or `BOTS_BINARY`.
- Keep one Cargo environment per tree. Runner incremental policy:
  [below](#oom-and-killed-commands). `CARGO_PROFILE_*` overrides produce distinct
  artifacts; dev/test already have line-table debuginfo. Plain Cargo after a
  runner build changes inputs and can rebuild unnecessarily.
- Explicit `CARGO_TARGET_DIR` is retained; otherwise select
  `<work>/targets/slot-a..slot-z` (`KRASNOKAMEN_SLOT_TARGETS` override):
  that tree's last free slot, another existing free slot, then a new one only
  above `KRASNOKAMEN_SLOT_MIN_FREE_GB` (250 GiB default, rounded up).
  This reserves no space and does not cap existing-slot growth.
- `.krasnokamen-tree` records the starting cwd's Git toplevel; switching trees
  touches tracked files to defeat Cargo mtime/relative-dep-info false freshness.
  Never `cd` to a second worktree inside the command: it can run the first
  tree's newer binary. Use separate runner invocations or distinct targets.
  One heavy command per agent also prevents multiplying 15–40 GB slots.
- Retain the `mem-run: scope` line (properties, lane, target, peak, signature).

**Watchdog**

`solaris-heavy-watchdog.service`, started by the runner, alone freezes builds.
On PSI/MemoryMax pressure, freeze the youngest preemptible command only while
available room is negative; never the oldest running command. Preemptible means
plain cargo build/check/clippy/doc/fetch or test/bench `--no-run`, including
leading env. Tests, harnesses, servers and clients are never pressure-frozen,
so timeouts remain honest.

Room = min(`MemAvailable`, slice cap−unreclaimable memory)−4 GiB; tmpfs/dirty/
writeback count, reclaimable clean cache does not. On exit, thaw oldest-first
while expected growth fits. Restart preserves the shared jobserver.
Relevant regression sources: `tools/dev/test_build_tokens.py`,
`test_scene_slot.py`, `test_mem_run_release_policy.py`,
`test_measurement_window.py`, `test_heavy_watchdog.py`.

## Diagnosing "are they stuck?"

One bounded `/proc`/cgroup observation, never a polling loop:

| State | Evidence |
|---|---|
| Before start | `fleet-status` token/scene holders/waiters; runner's child `flock` for exclusive/perf lock, `slot-lock.py` for build admission. Never kill a legitimately queued `-p` run. |
| Frozen | `systemctl --user show solaris-heavy.slice -p ControlGroup --value`; scope `cgroup.freeze` requests freeze, `cgroup.events` `frozen 1` confirms. Read `memory.current`, `cgroup.procs`; reasons: `journalctl --user -u solaris-heavy-watchdog -o cat -n 20`. |
| CPU throttled | Slice `cpu.max`, `cpu.stat` `nr_throttled/nr_periods`; owner quota comes from `/tmp/solaris-mem-slots/cpu-quota` (e.g. `600%`) and is reapplied each call. Other quotas are cleared. Runtime drop-ins: `/run/user/<uid>/systemd/user.control/solaris-heavy.slice.d/`. Never remove an owner cap without authorization. High CPU pressure with >40% idle suggests quota/pinning, not load. |
| Pinned | Cargo/rustc `Cpus_allowed_list`; authorized live widening must cover every `/proc/<pid>/task/*` via `os.sched_setaffinity`. |
| Memory | Host `MemAvailable`, slice `memory.current/events`, swap and parent limits. |
| Cargo lock | `wchan=locks_lock_inode_wait`, fd for `<target>/.cargo-build-lock`, matching inode in `/proc/locks` (`->` waiter). Shared target contention, not runner admission; resolves when holder exits. |
| Agent | A long wait is not death; [fleet liveness/recovery](agent-fleet.md#liveness-and-recovery). |

For a diagnosed stuck waiter, cancel its parent job/owned PID tree including
flock children, never by broad name. Report commands exiting before their
scope line for rerun. Wrong-freeze repair: stop watchdog, thaw frozen scopes
(`cgroup.freeze=0`), atomically replace fixed code, start watchdog; other runner
calls may restart it during repair.

## OOM and killed commands

- Exit 143/137 or a signal proves neither OOM nor owner cancellation. Correlate
  exact child/scope with `memory.events`, `memory.peak`, user journal and kernel
  records (`journalctl -k --since -3h | grep -E 'Killed process|oom-kill:'`).
  Keep unknown causes unknown and retain failed receipts. Lower compile
  concurrency before increasing a proven-insufficient cap; rerun only affected gate.
- To prove compiler cgroup placement, an authorized diagnostic wrapper records
  `/proc/self/cgroup` from the compiler, not a shell smoke test.
  Never set slice `MemoryHigh`: reclaim pressure can make systemd-oomd kill the
  largest unit (terminal/agent host).
- Runner selects `tools/dev/sccache.sh` and `SCCACHE_CLIENT_SIDE=1` only while
  `solaris-sccache.service` is active and `~/.cargo/bin/sccache` executable;
  otherwise uncached `RUSTC_WRAPPER=`. The app.slice daemon fronts cache only:
  compiles remain in caller scope. Never stop/restart/reconfigure it or start
  sccache in a job scope. Compare stats without zeroing them.
- Do not pass your own `RUSTC_WRAPPER=` or plain `sccache`. The wrapper removes
  `CARGO_TARGET_DIR`/`CARGO_INCREMENTAL` from sccache's environment (their effects
  already reach rustc argv), preventing per-slot key misses and incremental
  refusal. Its `sccache` stem enables C/C++ cc-rs caching too.
  One cold-target/warm-cache measurement (2026-11-11): debug 4542→3316 G
  instructions (213/271 Rust hits, 284 C hits), release 5482→4707 G;
  empty-cache population +6%/+1%. These are conditional samples, not guarantees;
  already-warm target slots gain nothing.
- Runner sets `CARGO_INCREMENTAL=1` for debug/test and normal-lane release,
  `0` for `-p`/`-x` and any `--profile`. Incremental costs ~2.4 GB/slot and may
  slow runtime through extra codegen units. Performance binaries must use
  `pin-release --no-incremental` (manifest records effective environment).
  Historical cold core-test build: 26m48s→19m07s; re-pin CPU direction was
  favorable but full-build host load was unrecorded, so no speedup guarantee.
- Do not blanket-kill shared LSP, Gradle, clients or agent hosts. Stop only
  verified owned/authorized processes under fleet recovery rules.
  Long tool jobs need an explicit async deadline (`timeout: 0`/`3600`), not
  widened harness timeouts to evade a tool's 300 s default.

## Changing the runner or watchdog

Replace scripts atomically (write copy, `bash -n`, rename), never edit a running
shell script in place: incremental reads executed spliced text previously.
Test lock/admission changes on a private copy/lock dir using FIFO gates and
`/proc/locks` inode-filtered waits: ordering, exclusive behavior and killed
waiters must fail on old and pass on new. Scopes can vanish between glob/read;
ignore failed reads, but only `cgroup.events` `populated 0` proves empty
(`cgroup.procs` omits child cgroups).

**Install order when the wrapper's guardian arguments change** (since 2026-11-23): the two are one
protocol, so install the parts a new wrapper *needs* before the wrapper itself, each by atomic mv:

1. `mem-run-guard.py` — the argument list is positional; a wrapper that passes five arguments to a
   four-argument guard makes every launch exit 75 before the scope exists.
2. `reserved_cpus.py` and `reserve-cpus` — a wrapper that reads `verdict`/`clear` refuses (exit 75)
   when the helper is missing, so it must already be beside it.
3. `mem-run.sh` last. It is the only file that decides which of the others it calls, so a rollback is
   `mv` the old wrapper back first.

Never install the wrapper first "because it is the entry point": every command on the host would fail
until the guard copy lands, and mixed versions on the shared host share one lock and registration dir.

## When the owner says the machine is choking

Owner-approved 2026-10-24 response: renice rustc/cargo/java/qemu to 19; new
commands use lower `KRASNOKAMEN_MEM_RUN_CPU_QUOTA` (e.g. `400`), `-m 6G`,
`nice -n 19 cargo ... -j 2`, `--test-threads 2`, focused crates. A machine-wide
owner quota goes atomically into `/tmp/solaris-mem-slots/cpu-quota` (`600%`,
for example), not an untracked slice override the runner clears. Trace out-of-slice
rustc cgroups/cwds to owners.

## Disk

Nothing new on root disk: worktrees `<work>/wt/`, targets `<work>/targets/`,
sccache on games disk; `/tmp` is root disk. Cross-device worktree moves need
rsync, not `git worktree move`. `/dev/shm` per-user quota can be below `df` free.

**Keep ≥250 GiB free** (owner 2026-11-13): DRAM-less Samsung 980 write cache
shrinks near full. Below floor+10 GiB (default 260), ordinary runner calls invoke
`slot-gc --delete`, report errors without failing the command; not in `-p`/`-x`.
It removes attributable dead artifacts/superseded finalized incremental sessions,
then rotates idle slots LRU (incremental first, other output except `tmp/` next).
Pool/use locks and live `CARGO_TARGET_DIR` users protect slots, including explicit
targets; bare Cargo racing the scan is not protected. Slots ignore in-use marks,
so idle live-tree caches can be rotated
([slot-gc details](../../tools/dev/README.md#slot-gc---delete--v-slot)).

Below floor, standing owner approval (2026-11-17) allows **reproducible build
caches only**, in this order; check `df` after each:

1. Idle incremental caches: dry-run, then
   `slot-gc --delete --incremental-only --min-free-gb <target>`.
2. Named finished/merged `targets/<Agent>` dirs with zero users; if they also
   hold inputs/worlds/jars/tool checkouts, remove only Cargo output.
3. Idle-slot full rotation: `slot-gc --delete --min-free-gb <target>`.

Above floor+10, plain GC only removes safe dead classes; zero reclaimed does not
mean no incremental caches exist. Use step 1 rather than deleting run data.

| Object | Retention and removal |
|---|---|
| Worktree | `wt-done` verifies `final.patch` and archives evidence before removal; obey [archive/restoration rules](worktrees-and-integration.md#creating-a-worktree). Mark dependent checkouts/pins using `in-use add/remove`; `wt-gc` displays marks. Manual removal requires finished owner, no tracked changes except stray `.pyc`, HEAD retained in a branch (`git branch --contains`), and no handoff/config path dependencies. First preserve `.analysis/` at `<work>/receipts/<wt>-analysis` and built outputs at `<work>/release-artifacts/` (not “published”); only then `git worktree remove --force <absolute-worktree>` and `git worktree prune`. |
| Target dir | Recheck zero users in `/proc` cwd/exe/fds/environ immediately before removal. Pooled deletion requires its lock (`flock -n /tmp/solaris-mem-slots/<l> rm -rf <work>/targets/slot-<l>`). Preserve active-task dirs and binaries other agents run. |
| Receipts/scenarios | Active or next-task-needed runs stay whole. Finished runs retain reports (`md/yaml/json/txt`), logs, `result.json`, patches and screenshots. Only provably reproducible copied client roots/binaries/jars/wasm may go when their source/pin is retained. |
| Run data | Crash copies, WAL, worlds, journal, client run state, `perf.data` and archives require explicit owner deletion approval; failing runs and negative controls stay whole even then. A WAL hash never replaces its bytes. |
| Other | Never touch owner's main checkouts/targets, sccache, or other projects' `/tmp` caches. |

Past disk fills came from parallel slots, copied wasm/client content and nested
SDK targets. Avoid copying caches into worktrees/receipts; protect unique input
and evidence, not reproducible bulk.

## Superseded runner designs (do not follow old skills)

The October fixed-memory-slot, 3×5G, CPU-2/3 and Main-GO recipes are retired.
Current design is the watchdog/shared jobserver (2026-10-28), tree-marked pooled
targets (11-04) and three nested-reusable compile tokens (11-11; D27's four-token
trial ended 11-18). Exit 75 is a named admission refusal, not the old memory-slot
rule. Follow this page and the live runner, not historical skills.
