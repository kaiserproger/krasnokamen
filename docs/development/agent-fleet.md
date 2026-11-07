# Running agents

[AGENTS.md](../../AGENTS.md) "Execution and subagents" says when delegation is
allowed and what a result looks like; the
[Agent Constitution](../AGENT_CONSTITUTION.md) binds every agent. This page
records how fleets were run on the owner's machine and what went wrong.

## Size and roles

- **Keep the fleet small.** On 2026-10-24 about 26 parallel agents burned 30%
  of the owner's weekly usage on two accounts in one hour, and the owner
  collapsed the fleet. Default since then: a few long-lived roles (for example
  one Loader owner, one fixer for server and client, one bug finder/triage
  that owns the bug list and hands out batches). Larger fleets (the 22-slice QA
  hunt of 2026-10-29) run only on explicit owner request.
- **Batch related bugs.** Give one fixer a batch of related bugs (same module,
  feature or repro) with one combined gate and proof per batch; route to the
  existing owner of the area instead of spawning an agent per bug. Close agents
  whose area is empty.
- **Nested delegation.** On 2026-10-24 subagents were forbidden to spawn their
  own subagents. Since 2026-10-28 a worker definition names the worker types
  it may spawn (`spawns:`): the bounded and long-slice workers only the fast
  worker, the design-heavy worker also the long-slice worker. The spawner gives
  disjoint write sets and reviews every returned diff itself (see
  [review committee and breaker](review-committee-and-breaker.md)).
- **Models** are the owner's choice; repo docs name roles, not vendors or
  models. When the owner says a provider's limits are reserved, do not spawn
  workers on it.
- Deploys to the test server are batched until the owner says deploy.

## Briefs

- Bounded task, base commit, absolute worktree path, owned files, evidence
  contract, and the memory rules from [heavy commands](heavy-commands-and-memory.md).
  No conversation history forks.
- Include the Constitution and, for worktree agents, the absolute-path rule.
- **Verify every diagnosis in source before relaying it.** An advisor once
  escalated a worker's "unresolved symbol" report into "the committed
  baseline is broken"; `git grep` at the pinned commit showed the functions
  existed and were public (2026-10-24). Briefs cite `file:line` you read.
- Tell agents: no `git checkout/restore/stash/reset` or whole-file copies over
  shared sources; delete only their own artifacts by exact path; kill only
  their own PIDs; keep their files compiling.

## Messaging

- Message active owners by exact id. **Never broadcast to `agent://all`**:
  it wakes completed and idle agents into acknowledgement loops that burn
  budget, and it revives cancelled ones. The one exception is a fleet-wide
  pause (below).
- Perf/measurement windows are announced only to agents running heavy work
  (see [performance windows](performance-windows-and-profiling.md)).

## Liveness and recovery

- Check `proc://` and the mtime of each agent's session `.jsonl` (find it with
  a glob over the sessions directory; the newest top-level session may belong to
  another project). An agent whose last call is a long wait is idle on a build,
  not dead.
- Some workers stop mid-task ("standing by", "awaiting call") or hit the
  runtime's soft request budget and report "wrap-up"; that is the runtime, not
  the owner. Revive them with `write agent://<id>` and a concrete finish
  condition, or spawn a successor with the handoff file.
- After cancelling agents, list `proc://` again: same-name orphan instances
  survived a kill and ran for 10-17 hours (2026-10-24).
- Compact transcripts are `<session dir>/<Name>.md` (children:
  `<Name>/<Name>.<Child>.md`). The `.jsonl` files are 5-25 MB: read only a
  bounded tail, never the whole file.

### After an OOM killed the agent host

1. `journalctl -k --since -3h | grep -E 'Killed process|oom-kill:'`: was the
   victim the agent host or a build scope?
2. `free -g`; list surviving cargo/rustc/java/Xvfb/server processes.
3. Per worktree: `git -C <wt> diff --shortstat HEAD` and the newest edited
   files. Worktrees survive; agent state does not.
4. Relaunch one replacement per track in a single batch, with new names. Each
   brief: worktree, predecessor transcript path, decisions already made ("do
   not reopen"), ordered next steps, memory rules, and the first step
   "compile-check through the runner and finish half-applied edits; revert
   nothing".

## Handoff across context loss or to another person

Tracked state is enough to continue: [docs/STATUS.md](../STATUS.md) (release,
priorities, known 1.0 gaps), the owner's open issues, the
[bug ledger](../bugs/README.md) and the ADRs. The owner host may also keep an
ignored cursor, `.analysis/MEMORY.md`; it is optional, and code, tests and
tracked docs win over it.

A handoff, written at phase close or before a reboot (not per call), goes into
`.analysis/MEMORY.md` on the owner host and into the result to the owner
elsewhere. It lists for each slice in flight: worktree path, base commit and
branch tip, state (committed, complete but unverified, or partial), receipt
paths, and one next action. Without commit permission add the tree, the diff
SHA-256 and the changed files. Facts that outlive the slice move into the
owning doc page.
Older night runs wrote a tracked `docs/NIGHT_RUN_<date>.md`; that is
superseded.

## Moving repositories while agents run

Used once (2026-10) to move the repos from the root disk to `<work>`:

1. Find the real mount (`lsblk`, `findmnt`); automounts live under
   `/run/media/$USER/<label>`. Never create a missing mountpoint on the system
   disk.
2. Clean first (inactive worktrees, stale large artifacts, orphan processes).
3. Bulk-copy live, excluding build dirs (`--exclude=/target/
   --exclude=/target-*/ --exclude=/build/`); rebuilding is faster than copying
   millions of small files.
4. The one legitimate broadcast: PAUSE (cancel commands, stop own servers and
   clients, no writes, reply `paused <name>`). Collect acks; kill leftovers whose
   cwd is inside the repos.
5. Final rsync pass, then `mv <old> <old>.old-move && ln -s <new> <old>`; check
   `git status` and `git worktree list` through the symlink.
6. RESUME (shells must `cd` again; first build is a full rebuild). Verify
   `git diff HEAD --stat` matches, then delete the `.old-move` copies. Ask the
   owner to add the disk to `/etc/fstab` with `nofail`.
