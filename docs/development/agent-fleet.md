# Running agents

[AGENTS.md](../../AGENTS.md) "Execution and subagents" says when delegation is
allowed and what a result looks like; the
[Agent Constitution](../AGENT_CONSTITUTION.md) binds every agent. This page
records how fleets were run on the owner's machine and what went wrong.

## Size and roles

### Announcement stabilization mode

Owner-directed cutover, 2026-11-15, for the finite 21 November announcement;
cap replaced by the owner on 2026-11-18 (D25/D26, ~00:10 host): **at most six
live executors on this host, at most three holding build tokens and at most
three editing or reading**, announcement rows and the off-screen settlement
simulation goal together (fixers, implementers, readers, gate and heavy-run
owners). Every other piece of work is a queued TODO item, not a live agent:
Main starts the next item when a slot frees (pull, not push), pre-freeze items
first. More throughput comes from the second server, not from agents waiting
for this host's tokens. Backlog streams (the `Tail*` items) wait for the second
server. One standing review committee (one Opus and one Sol reviewer) serves
every slice in batches by area; no per-task reviewer spawns. Persistence and D7
changes still need both members' verdicts; other slices may take one member's.
An agent that needs a build token runs one blocking command that waits for the
token and then runs (the `mem-run` runner already does); never `timeout N true`,
sleep or status-polling loops. Executors run only L0/L1 for their own crates
(filtered tests, `fmt`, `code-health`); no `correctness`, full L2 or
workspace-wide Clippy in a worktree. Main runs L2 once per merge wave on `main`
(owner, 2026-11-18). Main's hourly report gives live executors, token
holders, token waiters and their waiting time, CPU use and Main's RSS; the cap
moves only on that data. The builder-token, disk-floor and `mem-run` limits are
unchanged. This mode overrides the general fleet defaults/history below, until
the [finite acceptance and stop condition](../DEFINITION_OF_DONE.md#finite-announcement-acceptance)
is reached or the owner changes it.

Main uses the single ranked critical path in `receipts/Stage3/PLAN.md`:
finish/integrate/accept a returned vertical slice or its named dependency
before opening another front. Findings enter this announcement's blocking
queue only through the DoD admission rule; other findings remain visible
backlog. Do not start a helper, committee or broad QA campaign for backlog.

Checkpoint and park excess owners at safe immutable commit/patch boundaries;
preserve ownership and partial/negative proof, and never kill an in-flight
proof merely to reduce the counter. Waiting background commands consume no
LLM turns, but cannot hide concurrently writing owners. Already allocated
laptop W2 is a separate measurement job, not another host implementation
slot. Existing build, memory and measurement safeguards remain.

Workers return bounded immutable changes with affected L0/L1; Main batches
integration and owns final integrated L2. Reuse the same clean pin for the
same tree/inputs; do not repeat unchanged greens or sibling full suites.
Checkpoints and completions are event-driven, without polling, empty wait
loops, acknowledgement chains or repeated large transcript reads. Report
newly integrated/accepted outcomes, remaining blocking rows and impediments,
not agent count as progress.

No new paid top-ups, saved resets, accounts or provider/credential changes
without explicit owner authorization. Main's process/session/model stay
as-is: this spending brake is not a restart or a claimed dollar-budget cap.

- **Keep the fleet small.** On 2026-10-24 about 26 parallel agents burned 30%
  of the owner's weekly usage on two accounts in one hour, and the owner
  collapsed the fleet. General limits since 2026-11-14 (owner), **outside the
  current announcement mode**, are at most 15 active agents plus 2 reviewers
  (agents only waiting for builds/tests do not count), and 3 concurrent
  builders enforced by the runner's
  [build tokens](heavy-commands-and-memory.md#how-toolsdevmem-runsh-works-current-since-2026-10-28);
  no child agent for a part under 30 minutes; reserve the
  review and breaker budget before spawning builders. Long sessions re-read
  their own growing context every turn (owner, 2026-11-15: BvItemCache ran
  16.9 h and 1080 turns), so a generalist hands its lane over when its context
  is nearly exhausted rather than running on. Larger
  fleets (the 22-slice QA hunt of 2026-10-29) run only on explicit owner request.
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
- **Models** are chosen by Main per lane, by task and budget
  ([generalist roster](#generalist-roster)). Briefs link the standing reviewer
  requirements in AGENTS.md rather than restating them; a specifically named
  reviewer cannot be replaced or an older review relabelled without owner
  authorization and actual source coverage. Reserved provider limits remain binding.
- Deploys to the test server are batched until the owner says deploy.
- **New process rules replace or become checks.** A rule written after an
  incident must replace an existing rule or become a tool check (main-guard,
  in-use registry, preflight), not add prose: post-incident remedies mostly
  add coupling and complexity, and every rule is one more thing a worker must
  hold ([Cook](https://how.complexsystems.fail/), point 15). Delete the rule it
  replaces in the same change. A check fails only on what it names: one that
  fails ordinary correct changes is fixed or deleted, never routinely raised
  or skipped, because a check everyone learns to override defends nothing.

### Generalist roster

Owner rule, 2026-11-16 (`receipts/Stage3/OWNER-RULE-GENERALISTS-20261116.md`):
no agent per task. Main runs a few long-lived generalists, one per lane, each
with a queue of tasks and a write set disjoint from the other lanes. New work
joins the matching lane's queue. A new agent starts only for a new lane or when
a generalist's context runs out; the new one takes over that lane's queue and
the previous one's handoff.

- **Never idle on a wait.** A generalist starts its build, test, gate or trial
  in the background, works on the next queued task, and handles the completion
  when it arrives. With nothing else queued it tells Main it has free capacity
  instead of waiting.
- **One result per task.** Each finished task gets its YAML result (template
  below) as a message to Main; the agent keeps going. Its last message hands
  over whatever is left in its queue. Claims are per task, as below.
- **Reviewers and breakers are not generalists.** They are separate, read-only,
  and from a different vendor than the author where possible.
- **Model by task and budget (owner D46, 2026-11-19).** Main chooses each
  lane's model and records the choice and reason in `receipts/Stage3/PLAN.md`.
  Implementation defaults to Terra or Sol: Sol for long multi-step slices,
  Terra for bounded slices, gates, QA and harness. Opus only designs
  persistence, protocol or item custody, or takes a task the OpenAI filter
  refuses; the owner wants Opus cut even below that, so in doubt pick Terra or
  Sol. Fable runs only when the owner explicitly asks for an architecture
  design. Reviews run on Opus or Sol from a vendor other than the author's
  (D14). Muse or DeepSeek take mechanical bulk. Edit-heavy slices go to Terra,
  Sol or Astra: about 10% of their edits failed against 24–27% for Sonnet and
  Mistral (owner, 2026-11-17).
- **New context after each finished task.** After sending a task's result, a
  generalist writes its remaining queue and state to its lane's handoff file
  (`receipts/<lane>/HANDOFF.md`) and starts a new context before the next
  task; it does not carry 150–260k tokens of a finished task into every turn.

### Fleet speed rules

Owner, 2026-11-17, from the transcript audit `receipts/FleetSpeedAudit/REPORT.md`
(about 60% of active agent time was spent waiting on the agent's own gates,
tests and builds; 55 h on Main, IRC or peers; 14% of edits failed).

- **Short work root.** `/var/tmp/w` is a symlink to `<work>`; briefs and
  commands may use it (`/var/tmp/w/solaris/tools/dev/mem-run.sh` is Main's
  runner). If it is missing, recreate it with
  `ln -sfn /run/media/kaiserroman/games/solaris-work /var/tmp/w`.
- **Gates.** Main builds one pin per merged `main` commit and every gate binds
  to it. One gate lane runs the real-client gates and their repeats; a builder
  runs its own change's red/green proof once and hands the exact repeat
  command to Main instead of rerunning unchanged gates.
- **Hand-in.** A finished branch merges cleanly into current `main`: merge
  `main` into it in the worktree, resolve conflicts there and rerun the
  affected L0 before sending the result. Independent parallel branches cannot
  both fast-forward `main` after the first lands, so Main applies a batch in order on a
  separate integration branch, rechecking each before applying it, runs the
  merged-tree checks there and then moves `main` with one `--ff-only`.
- **Do not block on Main.** After sending a question, continue the queue's
  other work; the answer arrives as a message.
- **Ledger reads.** Never read all of `docs/bugs/ledger.toml`: find the row
  with `rg -n 'id = "<ROW>"' docs/bugs/ledger.toml` and read only its block;
  edit only that block.
- **Edits.** Re-read the exact range right before editing it.

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
# Module     a new feature only: its module or crate path, host module, the one
#            registration point, exported interface; largest-cycle module count
#            from code-health not higher ([new features are
#            modules](adding-a-feature.md#new-features-are-modules))
# Evidence   per row: cargo:<crate>:<test> | harness:<profile> (real client) |
#            review-only
# Commits    none (leave the diff and its SHA-256) | local commits on the branch
# Limits     only what differs from the standing rules: wall budget, whether
#            subagents are allowed. One build token per agent at a time (the
#            next compile starts after the previous one finished);
#            `env KRASNOKAMEN_BUILD_LANE=background` unless Main names this slice
#            the priority or it is battle-video work. The one-token rule and
#            priority gate are laptop proposals of 2026-11-18, pending owner
#            confirmation; battle-video work gets resources first (owner, 2026-11-12).
# Waiting    never end your turn while a background job you started is still
#            running: a subagent that ends its turn is disposed and the job dies
#            with it (owner, 2026-11-18: Perf1M's pin build, CutTick's L0 and
#            CutEdgePlan's re-check died this way). With nothing else to do,
#            call `wait` on that job. Never poll; never run sleep loops in eval
#            or bash.
# Turns      batch independent relevant reads into one turn when there are
#            several; a dependent read may follow (owner, 2026-11-15: LLM time
#            was 22% of 90 agents' day)
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
- **Slices close in hours; nothing claimed waits a day.** Main selects owners
  from the current ranked critical path under the operating mode above, not
  a historical “fill ten agents / battle first” schedule. A brief names its
  bounded result and timebox; at expiry the owner returns immutable work and
  evidence, or `partial` with the exact outside prerequisite. Main handles
  that completion/overrun event by integrating, parking or reassigning; it
  never polls fleet status on a schedule. Reassignment follows the previous
  owner's checkpoint/release, so two owners never write the same task.
  Release the claim when the result arrives. Deferred work stays visible in
  the single queue, not automatically active for this announcement.

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
6. At slice close, update the existing page that owns each changed behaviour or
   material verified lesson; do this before context loss if the lesson is critical,
   not after every command. Keep one owner per fact: a short rule/invariant,
   source or reproducible check, and its limits; link existing coverage instead
   of another summary layer. Distinguish current runtime, design/hypothesis and
   branch-only work. Replace stale normative text and repair incoming links;
   preserve valid historical evidence and verbatim owner decisions as history.
   Every `gaps` entry stays queued with its blocker. Temporary pins/jobs/queue
   state belongs in the ignored handoff cursor, not reusable public instructions;
   never commit secrets, account identities or raw diagnostic/transcript archives.

## Doc coherence check

Run it after any change to AGENTS.md, the Constitution, AGENT_TOOLING,
DEFINITION_OF_DONE, the QA protocol, the bug ledger README, the owner
contract, this page or the heavy-commands page. During the owner-directed
announcement cutover, Main performs this check **inline**, without a new
docs reviewer/helper: read AGENTS and the touched owning pages as a new
worker would; record concrete conflicting statements and their disposition.
Outside this mode, a fresh read-only agent may perform that check.
Fix contradictions in their owning page, not by copying policy elsewhere,
then run `python3 tools/doclinks.py`. Code/persistence/release independence
requirements remain unchanged and their reviewers use the same two slots.
Record each concrete finding and its before/after disposition in the result.

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

Tracked docs supply constraints, not a substitute for Main's complete ordered
work queue. On the owner host load the complete current AND deferred ordered
sections of `receipts/Stage3/PLAN.md`, including blocked rows, and the current
`.analysis/MEMORY.md` index; read historical event logs only when needed.
After every compaction/resume, read the actual tool TODO view and reconcile stable IDs,
order, statuses, blockers and receipt links **before new dispatch**. Restore
missing rows from the canonical plan/history without inventing completion;
an active-pane summary alone is not proof that other rows were lost.

A handoff, written at phase close or before a reboot (not per call), goes into
`.analysis/MEMORY.md` on the owner host and into the result to the owner
elsewhere. It lists for each slice in flight: worktree path, base commit and
branch tip, state (committed, complete but unverified, or partial), receipt
paths, and one next action; a slice building a new feature also names its
module path, host registration point and interface, since new features are
[modules](adding-a-feature.md#new-features-are-modules) and that survives the
handoff. Without commit permission add the tree, the diff
SHA-256 and the changed files. Durable lessons follow the
[slice-close owning-doc rule](#mains-integration-checklist), not a second
tracked knowledge dump.
Older night runs wrote a tracked `docs/NIGHT_RUN_<date>.md`; that is
superseded.

Only an explicit owner instruction or a concrete recorded dependency may
change order. Record the affected ID, old/new position and causal authority;
preserve every other item's relative order. A blocker pauses the owned item
with its rank, partial proof and receipts intact; it does not silently delete
or demote it. Executing an eligible item while another is blocked does not
rewrite their ranks.

Side requests append to the existing queue, never `init`/replace it with only
the new subtask. Keep current and future stable IDs in the plan, Main cursor
and tool TODO. Cross-reference overlap rather than dropping either promise.
Deferred phases remain **blocked**, with no automatic research/benchmark/
implementation dispatch until announcement closure or explicit owner
activation; the finite announcement's stop/owner-decision rule still applies.


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
