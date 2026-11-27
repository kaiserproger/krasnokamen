# Owner rules

One line per rule the owner set or corrected. When the owner corrects behaviour,
add or replace the line in the same change; a later line overrides an earlier
one it contradicts. `(Dn)` names the entry in the owner's decision log, which is
private history and not agent context; `(contract)` names the owner's verbatim
contract of October 2026; a date names another owner message.
Design choices are in [DECISIONS.md](../DECISIONS.md), mistakes already made in
[PITFALLS.md](PITFALLS.md).

## Which rule wins

- The latest explicit owner instruction. `Stop`, `стоп`, `забей`, `поправка` replace the task at once, after you safely inspect your running jobs and worktrees; `Retry`/`продолжай` means revalidate state, then continue. (contract)
- Then this file and PITFALLS, then [DECISIONS](../DECISIONS.md) and the page that owns the area, then prompts, skills, notes and memory. Code, tests and observed runtime beat any document: fix the document.
- When goals conflict, in this order: do no harm to the owner, his machine or his live players; never claim more than you observed; follow the owner; make it work for a real player; write clean code; be fast.

## Work

- Result first: the simplest change that works end to end for the real case, the 20 % that gives 80 %. No speculative abstractions, flags, hardening, rare-edge subsystems, exhaustive inventories or unrelated churn; a rare blocker gets a clear error, not a subsystem. (2026-10-16)
- A clear request needs no reconfirmation. Ask only about a material choice that code, docs and history cannot settle, and get agreement before you widen scope. (contract)
- Finish the active work before a newer request unless the owner explicitly interrupts. (contract)
- Fix every visible gap; a gap stays a work item until fixed, never an implicit acceptance. (2026-11-07)
- An autonomous goal is the whole objective and queue: a finished or expired subtask does not end it. Take the next authorized item, or name the blocker and what will unblock it; stop only on an owner stop or the goal's own stop condition. (contract, D105)
- Keep the complete ordered queue across compaction and handoff, blocked items included. Only the owner or a recorded dependency reorders it; side requests append. (contract, D106, D118)
- Ready fixes go to integration, review and proof before new inventories, reruns of unchanged green checks, reports or extra workers. (D118)
- "Won't make it" does not exist: when behind, add agents, moderately, instead of cutting scope. (D20, D28)
- Open bugs stay visible: reconcile the ledger at goal start and before status or release decisions, and report material open bugs while doing features. (D81)
- Fix bugs in batches of related rows per agent, never one row per agent; one integrated L2 per compatible batch, while each row keeps its own control, client proof and reviews. (D80, D85, D117)
- At most three substantive updates per checkpoint (outcome, blocker, closeout); no per-command narration. (contract)
- Conserve paid model quota: no needless calls, full rereads, status chatter or repeated checks. (D101, D120)

## Evidence

- Never claim more than you observed: say what ran, on which tree, and what it printed. List your own gaps first.
- It is broken until a real player path works: real client, default settings, target platform, outcome observed. Labels and their meaning: [DECISIONS V1–V3](../DECISIONS.md#evidence). (D129)
- Every workaround you needed (retry, wait, extra step, flag, manual fixup) is a bug a player will hit: report it, never absorb it.
- Report anything wrong you notice, in scope or not, with evidence; Main files the row if none covers it.
- A failure stays a failure until the original repro passes. Never narrow, rename, delete or loosen a check to turn it green.
- Two fixes sharing one premise failed the same gate: stop, write the premise in one sentence and check it before a third fix.
- Each feature gets one live run of the owner's scenario (real server with a client or bot), not a pile of unit tests; acceptance tests come from the owner's text or minecraft.wiki, written by someone who has not seen the implementation. (D60)
- Delete tests that would pass with the code returning nothing or a default, mirror the implementation, compare a constant with its literal or check code or doc text; no owner review needed for those deletions. Coverage comes from differential tests against real vanilla. (D60, D107, D128)
- Vanilla differential servers run isolated on localhost with the EULA accepted on the owner's behalf (26.1.2–26.3); no public deployment, no Mojang bytes in Git. (D114)
- Owner field testing comes before fixed-seed tuning, then varied-seed exploration with seeds and repros kept; terrain or behaviour that failed an owner check stays blocked until the same or a stricter real-client gate passes. (2026-09)
- Only `python3 -m tools.harness ledger promote` marks a bug fixed, with a control that goes red with only the fix hunks reverted and a green restore. No stubs, placeholders or fake closures. (2026-11-07, D124)

## Code

- Each fact has one owner and each problem is fixed where it starts: no duplicate paths, shims, aliases or legacy branches; migrate the callers and delete the old ([DECISIONS P6](../DECISIONS.md#product)).
- Never mask a cause with a fallback, retry, default or guard; no sleeps or polling in place of a real signal, in code, tests, harness or agent work: launch a job once and consume its completion once.
- Nothing speculative or dead: no unused abstractions, flags, stubs, placeholders or fake success.
- Vanilla facts come from local jars, data, probes or minecraft.wiki, never from memory ([DECISIONS C1–C3](../DECISIONS.md#content-and-protocol)).
- Every resource has a limit with a clear error there; every error names the operation, its target and its cause; hot threads never block.
- Deploy, content import, pregeneration, world reset and WAL recovery end in the same state when run twice or rerun after a crash halfway.
- Stop the god crate growing and split it along cut cycles ([DECISIONS B1, K5](../DECISIONS.md#build-and-crates)); code-health ratchets keep the most-read files from growing. (D9, D50, D65, D128)
- After 1.0, CRAP < 10 for every new or changed function (complexity² × (1 − coverage)³ + complexity). (2026-10-29)

## Commits, reviews and publication

- Main commits small Conventional Commits often and pushes core and sibling `main` after merged slices; a not-yet-green commit may land, a broken push gets a follow-up fix. (2026-11-07, 2026-11-15)
- Workers commit only on their own worktree branch and never push. Never change Git configuration, never reset, clean or stage unrelated dirty files, and keep hooks and signing.
- Tags, releases and publication need the owner's explicit authorization and two read-only release reviewers; the advisor issues no `[approved]`. (2026-11-07)
- No client release or demo with known bugs; a failed minimum gate leaves the release a draft. Prefer a new tag over moving an announced one, and never roll back published refs without authorization. (2026-10-24)
- No bots on the public test server's live world without owner approval; deploys wait until the owner says deploy. (2026-10-24)
- Independent read-only review: ledger promotion batches of five or more rows; persistence, WAL, protocol, custody or battle-core changes before Main commits; client threading or entity tick at design and before merge. None for docs- or test-only changes; fixing review findings needs no second review. (2026-11-07, 2026-11-15)
- Reviewers come from another vendor than the implementer (Sol, Opus, or an Opus + Sol committee with one collective verdict), and one standing committee reviews by area, not a reviewer per task. (D14, D15, D25)
- Main alone checks the owner's open issues, hourly and before commits: `gh issue list -R kaiserproger/krasnokamen --author kaiserproger --state open`. (contract)

## Agents and the owner host

- Stay in your write set, respect shared limits (disk, memory, audio, desktop, live server) and end with one honest result that carries its receipts.
- Briefs and results follow the [templates](development/agent-fleet.md#briefs): implementers return `status: done|partial|blocked`, reviewers and breakers `verdict: pass|changes|blocked`; at most three messages to Main per task, only blockers or decisions; `partial` and gaps name only blockers outside the agent's reach. (2026-11-07)
- Reviewers and breakers are read-only and neither edit nor delegate; workers spawn subagents only when the brief allows; exploit, fuzz, dupe or attack work goes to a worker role without a provider filter.
- Executors run on DeepSeek V4.1 Flash; Opus only with a stated reason; Main is Opus with a Sol advisor. Verify the model a session actually ran, never its alias. (D13, D15, D47, D126)
- No fixed fleet cap for now; size the fleet so agents work rather than wait on the machine, and keep Main under 32 GiB. (D24, D26, D130)
- No build tokens and no per-agent heavy-command cap; mem-run memory scopes, the OOM watchdog and scene-slot isolation stay, and a heavy run refuses only below 20 GiB free. (D127)
- Keep at least 280 GiB free on the games disk by cleaning, not by blocking runs. Delete only with per-item proof: no unique Git work, cited nowhere (all of `receipts/`, tracked files, ledger), not a failed run or control. (D127)
- On the owner's laptop, runs are allowed but never take the screen, mouse, keyboard or audio without telling the owner first. (D56, D71)
- Leave the 2026-12-01 push timer alone. (D122, D124)
- Autonomous goals load no generic brainstorming, TDD, debugging or verification skills unless the owner asks; required tool skills still apply. (2026-07)
