# Running agents

[AGENTS.md](../../AGENTS.md) "Execution and subagents" says when delegation is
allowed and what a result looks like; the
[Agent Constitution](../AGENT_CONSTITUTION.md) binds every agent. This page
records how fleets were run on the owner's machine and what went wrong.

## Size and roles

- **Keep the fleet small.** On 2026-10-24 about 26 parallel agents burned 30%
  of the owner's weekly usage on two accounts in one hour, and the owner
  collapsed the fleet. Limits since 2026-11-07: at most 10 live agents and 3
  concurrent builders (enforced since 2026-11-11 by the runner's
  [build tokens](heavy-commands-and-memory.md#how-toolsdevmem-runsh-works-current-since-2026-10-28));
  no child agent for a part under 30 minutes; reserve the
  review and breaker budget before spawning builders. Prefer one long-lived
  lane per ledger area that batches rows; split a lane when its queue exceeds
  eight rows. Larger fleets (the 22-slice QA hunt of 2026-10-29) run only on
  explicit owner request.
- **Batch related bugs.** Give one fixer a batch of related bugs (same module,
  feature or repro) with one combined gate and proof per batch; route to the
  existing owner of the area instead of spawning an agent per bug. Close agents
  whose area is empty. Before assigning rows, run `ledger probe` on the base and
  skip rows that are already green (5 of 6 blocker agents on 2026-11-03 found
  their rows already fixed).
- **Nested delegation.** A worker definition names the worker types it may
  spawn (`spawns:`), but a worker spawns only when its brief allows it. The
  spawner gives disjoint write sets and reviews every returned diff itself (see
  [review committee and breaker](review-committee-and-breaker.md)).
- **Models** are the owner's choice; repo docs name roles, not vendors or
  models. When the owner says a provider's limits are reserved, do not spawn
  workers on it.
- Deploys to the test server are batched until the owner says deploy.
- **New process rules replace or become checks.** A rule written after an
  incident must replace an existing rule or become a tool check (main-guard,
  in-use registry, preflight), not add prose: post-incident remedies mostly
  add coupling and complexity, and every rule is one more thing a worker must
  hold ([Cook](https://how.complexsystems.fail/), point 15). Delete the rule it
  replaces in the same change. A check fails only on what it names: one that
  fails ordinary correct changes is fixed or deleted, never routinely raised
  or skipped, because a check everyone learns to override defends nothing.

## Briefs

A brief carries only what is specific to the task. Everything standing lives
in the owning docs, which every agent reaches through AGENTS.md: link them,
never restate them. Restated rules go stale (14 managed skills still described
memory slots and a GO after the runner dropped both; deleted 2026-11-07).

```text
# Target     rows or paths (ids only; attach `ledger probe` output for rows)
# Base       <sha>; worktree name
# Claim      task id and timebox; Main runs `task-claim` before launch (below)
# Write set  owned files and dirs; everything else read-only
# Evidence   per row: cargo:<crate>:<test> | harness:<profile> (real client) |
#            review-only
# Commits    none (leave the diff and its SHA-256) | local commits on the branch
# Limits     only what differs from the standing rules: wall budget, whether
#            subagents are allowed
# Deliver    the result template below
```

Standing rules a brief links and never repeats:

- Precedence and conduct: [AGENTS.md](../../AGENTS.md) "Which rule wins", the
  [Agent Constitution](../AGENT_CONSTITUTION.md).
- Heavy commands, runner caps, the `-p`/`-x` lanes (no GO), disk floor:
  AGENTS.md "Build and validation",
  [heavy commands](heavy-commands-and-memory.md#disk).
- Worktrees (`wt-new`, `wt-done`), absolute paths, release binaries
  (`pin-release`, never a per-agent release build), commit policy, owner
  checkouts untouchable: AGENTS.md "Git, local state and sibling
  repositories", [worktrees](worktrees-and-integration.md).
- Evidence, negative control, real-client gates, quality labels: AGENTS.md
  "Build and validation", [real-client gates](real-client-gates.md),
  [definition of done](../DEFINITION_OF_DONE.md); breakers also the
  [QA protocol](../QA_AGENT_PROTOCOL.md).
- Gaps are work (partial only for a blocker outside the agent's reach), the
  three-message limit, subagent limits, exploit/fuzz/dupe/attack routing, the
  result: AGENTS.md "Execution and subagents" and the template below.
- Hygiene (owned here): no `git checkout/restore/stash/reset` or whole-file
  copies over shared sources; delete only your own artifacts by exact path;
  kill only your own PIDs; keep your files compiling.

Result template (the LAST message; acks go through messages):

```yaml
status: done|partial|blocked        # implementers
verdict: pass|changes|blocked       # reviewers and breakers instead of status
base: <sha>
head: <commit>        # or patch: {path, sha256}
changed_files: []     # [] for reviewers and breakers
rows:                 # ledger work only
  - {id, state: fixed-unproven|already-fixed-at-base|not-reproduced|open, check, negative_control: <receipt>|none}
validation: [{cmd, tree, result, receipt}]
plan: <receipt of `python3 -m tools.harness plan`; every gate it lists is in validation or gaps>
gaps: []              # only items outside the agent's reach, each with its blocker
asks: []
noticed: []           # wrong things seen outside the brief, each with evidence
```

Reviewers add `findings`, `validation_gaps` and `confidence`
([reviewer](review-committee-and-breaker.md#independent-reviewer)); breakers
add the [QA protocol](../QA_AGENT_PROTOCOL.md#result-contract) fields and its
final `QA_RESULT` line.

- No conversation history forks.
- **Verify every diagnosis in source before relaying it.** An advisor once
  escalated a worker's "unresolved symbol" report into "the committed
  baseline is broken"; `git grep` at the pinned commit showed the functions
  existed and were public (2026-10-24). Briefs cite `file:line` you read.

## Claims and fleet status

Every task an agent works on is claimed, so no two agents take the same task
and an overrun is visible without reading transcripts (owner, 2026-11-11).

- **Claim at launch.** Main runs
  `<work>/solaris/tools/dev/task-claim <task-id> --agent <name> --timebox <min>`
  for every agent it launches (`wt-new` prints the command; the task id is
  usually the worktree name) and `task-release <task-id>` when the agent's
  final result arrives. The claim is the file
  `<work>/fleet/claims/<task-id>.json` (agent, start, timebox, lease end).
- **A claim is a lease, not a PID.** Main's subagents run inside Main's own
  process, so several logical agents share one PID. A lease is live until it
  is released or until start + timebox + grace (default 15 minutes);
  `task-claim --renew <task-id> --agent <name> [--timebox <min>]` starts a new
  lease from now and works only for the holder. A claim of a live lease is
  refused with the holder's name (exit 1); a released or expired lease is
  taken over, and the takeover is logged with the previous holder in
  `<work>/fleet/claims.log` (append-only JSONL of every claim, refusal,
  renewal, release and takeover).
- **`tools/dev/fleet-status`** prints one screen: unreleased claims with
  agent, start, timebox left or `OVERRUN`, and lease end (`EXPIRED ..., can be
  taken over`); build tokens in use with the holder's command and tree, and
  the commands waiting for one; scene slots the same way; host load, memory
  and running runner commands. Main and the owner read it instead of
  reconstructing state from transcripts. Tests:
  `python3 tools/dev/test_task_claim.py`.
- **Slices close in hours; nothing claimed waits a day** (owner, 2026-11-12).
  The owner runs the full allowed fleet (10 agents, 3 build tokens) and must
  not wait days for one task. Main cuts every task into slices that finish
  within hours and claims each with its timebox. At the hourly check Main
  reads `fleet-status` and handles every `OVERRUN` claim: split it, reassign
  it, or report its exact blocker to the owner. No claimed task goes more than
  24 hours without a result the owner can look at (a receipt, frames, a
  commit or a written blocker). The battle video stays first.

## Messaging

- Message active owners by exact id. **Never broadcast to `agent://all`**:
  it wakes completed and idle agents into acknowledgement loops that burn
  budget, and it revives cancelled ones. The one exception is a fleet-wide
  pause (below).
- Measurement lanes need no announcement: `mem-run.sh -x` serializes windows
  itself (see [performance windows](performance-windows-and-profiling.md)).

## Main's integration checklist

Before accepting a returned slice (procedure:
[worktrees and integration](worktrees-and-integration.md)):

1. The diff stays inside the brief's write set, and `changed_files` matches
   `git diff --stat` of that tree. Name the concrete failure or change cost
   and why this is the smallest sufficient correction; a pattern name or
   "legacy" alone is not a reason to change code.
2. Identify the owner of the rule and the invariant that must survive the
   operation, including dependent debits, credits and rejection. Similar
   statements need not express the same rule; shared authority must not be
   duplicated. Module boundaries follow responsibility and independent reasons
   to change, not file count. See [mechanisms](adding-a-feature.md#mechanisms-not-code)
   and [state ownership](../architecture/state-ownership.md).
3. `validation` names the tree and observes the promised result at its consumer:
   dispatch, chat echo or a mock interaction is not proof that world geometry,
   inventory or pixels changed. Reuse green checks only for an identical
   relevant tree, environment and scope.
   An optimisation claim also needs a comparable full workload, the relevant
   latency/memory budget and preserved behaviour under the
   [profiling rules](performance-windows-and-profiling.md), not just a
   microbenchmark or average TPS.
4. **Second way.** Does this change add a second mechanism for something that
   already exists (pinning, server launch, observers, ledger/proof paths,
   config, test helpers)? If so, either migrate the old one in the same change
   or reject it. A mechanism is parallel infrastructure or authority for one
   responsibility; duplicated code at call sites is not one
   ([rule](adding-a-feature.md#mechanisms-not-code)). Example, 2026-11-07: the
   ledger's `.analysis/ledger-builds` registry and its private release build
   were added next to `tools/dev/pin-release`, which left three pinning
   mechanisms for one idea ([open transitions](../STATUS.md#open-transitions)).
5. The touched area's open-transition row: the change moved it forward or left
   it alone; a transition opened or finished updates STATUS in the same change
   ([rule](adding-a-feature.md#open-transitions)).
6. The doc that owns each changed behaviour is updated; code beats docs.
   Every `gaps` entry is queued with its blocker, never closed by rewording.
   Put task-specific decisions, evidence and next actions in the existing
   handoff cursor so context loss cannot turn a review finding into a forgotten
   report; link the owning rule rather than copying it into another policy.

## Doc coherence check

Run it after any change to AGENTS.md, the Constitution, AGENT_TOOLING,
DEFINITION_OF_DONE, the QA protocol, the bug ledger README, the owner
contract, this page or the heavy-commands page. Main runs it, or hands it to a
read-only agent that edits nothing. A fresh agent with no memory reads only
AGENTS.md and the pages it links, as a newly started worker would, and lists
every contradiction or stale statement with both `file:line` citations. Main
fixes each one in the page that owns the topic (the one "Which rule wins" or
"Where to look" names), never by restating the rule elsewhere, then runs
`python3 tools/doclinks.py`. The result lists each finding with its
before and after lines. First run: 2026-11-07.

## Liveness and recovery

- Check `proc://` once when an agent looks stuck, never on a timer (AGENTS.md
  forbids polling), and the mtime of its session `.jsonl` (find it with
  a glob over the sessions directory; the newest top-level session may belong to
  another project). An agent whose last call is a long wait is idle on a build,
  not dead.
- Some workers stop mid-task ("standing by", "awaiting call") or hit the
  runtime's soft request budget and report "wrap-up"; that is the runtime, not
  the owner. Revive them with `write agent://<id>` and a concrete finish
  condition, or spawn a successor with the handoff file.
- After cancelling agents, list `proc://` once more: same-name orphan instances
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
