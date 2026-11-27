# Advisor, reviewer, breaker and committee

The [owner rules](../RULES.md#commits-reviews-and-publication) own review points/counts; [QA protocol](../QA_AGENT_PROTOCOL.md)
owns breaker acceptance. Normal review points: promotion batches ≥5 rows,
critical persistence/protocol/custody/battle-core changes (including client
threading/entity-tick design and code), and two reviewers before a tag.
No docs/test-only reviewer or reviewer carousel **unless explicitly requested
by the owner**. The current [fleet policy](agent-fleet.md#announcement-stabilization-mode)
sets the number of independent verdicts. [Historical wording](README.md) is retained in Git.

Runtime agent definitions are user-directory Markdown with frontmatter:
`name`, `description`, model, thinking level, `tools`, optional `spawns`.
Record the actual runtime identity, not merely this definition.

## Advisor (watchdog)

Owner-maintained ignored `WATCHDOG.yml` configures the independent advisor
(rewritten by owner request, 2026-11-07). It keeps the executor simple, correct
and truthful; priorities are correctness, diagnosability, simplicity, performance,
within the owner's ordinary-play/plugin/measured-performance/rare-edge order.
Current owner instructions outrank its file or opinion.

| Boundary | Rule |
|---|---|
| Action | Verify current files before advising (messages can arrive late). May correct small factual docs/notes errors and report them, message workers (`write agent://<id>`) or kill looping/duplicate jobs (`write proc://<id>/kill`). Production code: give exact `file:line`, do not edit |
| STOP | Only imminent/ongoing irreversible harm it cannot itself prevent: data/work loss, killing owner-kept jobs, unauthorized publication, direct-order/AGENTS violation, or wrong fact/status already sent to owner. Name minimal correction |
| Complexity | Intervene before an unnecessary worker/queue/state/retry/reconciler/abstraction/flag, second authority/path, local-bug subsystem, invented invariant, widened scope, or unit-only proof of concurrency/disk/client semantics. Ask invariant, why existing mechanism fails, what is replaced, simplest alternative |
| Publication | Owner authorization only; advisor issues no `[approved]` and grants none. Applies to pushes, tags, releases, deploys, GitHub comments and third-party messages; release still needs two reviewers |
| Severity | `blocker` for STOP cases or wasted usage (looping/extra agents, parallel same-file edits); `concern` for needless complexity; `nit` for minor effects. No style/naming notes without correctness consequence |
| Repetition | One STOP per topic (owner 2026-11-07). Repeat only with new evidence; fixed or reasoned-rejected topics stay closed. Append concern/blocker and disposition (adopted/rejected with reason/ignored) to ignored `.analysis/advisor-ledger.jsonl` to measure false positives |

Before escalating “baseline broken”, check definitions **and callers at the
pinned commit** (2026-10-24 lesson).

## Release reviewers (owner, 2026-11-07)

Before every tag, Main requests two independent read-only reviews, no builds/edits:

1. **Contents:** previous-tag diff, introduced/remaining bugs, each release-note
   claim against code/receipts, upgrade traps, sibling pins.
2. **Whole core:** 1.0 readiness in owner order—ordinary play, saves, plugin API,
   performance, client-input security; player-reachable ledger bugs and ADR
   violations that cause real bugs.

Each finding: `file:line`, reachable path, severity. P0/P1 introduced or left open
**in release scope** blocks the tag. Save both YAMLs to `.analysis/releases/<tag>/`.

## Independent reviewer

Read-only independent review is **required at each review point** in the owner rules,
including critical work **before Main commits**, not just before release.
Reviewers neither edit nor delegate; fixing findings does not itself commission
another review. Current fleet policy sets verdict counts.

Owner D14 (2026-11-17), reaffirmed D46 (2026-11-19), chose Sol/Opus from a vendor
different from the author. The [D69 routing and later addenda](agent-fleet.md#model-routing-while-claude-is-out-owner-d69)
supersede those **model assignments** while applicable; they do not waive
independent review. Fable is used only for an explicitly owner-requested
architecture design. A later explicit **owner** instruction overrides the model
assignment for its named task only: D119 (2026-11-23) requires real Fable 5.2
review of documentation consolidation, not a fallback. Do not generalize it.
Record actual agent/model, independence, exact base/candidate/diff and critical-source
coverage. Reuse an old/partial review **only for its covered hunks**, never for a
new combined candidate. Review points/counts, correctness, negative controls,
original-consumer/client evidence, breaker and two release reviews are unchanged.

Apply [negative-code gate](../AGENT_TOOLING.md) and
[Main's integration checklist](agent-fleet.md#mains-integration-checklist):

| Review | Required question / output |
|---|---|
| Scope | One candidate: literal acceptance items, path-limited diff, compact evidence. No parent history or north-star redesign |
| Reachability | Authority, CAS/publication order, conservation, persistence, races/refusals, write scope, fake parity, pass-by-construction tests, sleeps/polling, timeout-as-success, stale artifacts; missing required client/oracle/perf/storage/concurrency evidence. Style only if it hides a defect |
| Newly allowed state | For persistence/custody/protocol, identify the state/path/order newly enabled and check loss/dupes ([Cook](https://how.complexsystems.fail/), point 14). Historical example: checkpoint skipping a refused chunk enabled a truncated-WAL dupe (2026-11-07) |
| Second mechanism | Reject parallel pinning/launch/observer/ledger/proof/config/test infrastructure unless old is migrated/deleted in the same change. Call-site repetition is not a second authority; caller-specific modes in shared functions are also a defect ([mechanisms](adding-a-feature.md#mechanisms-not-code)). Check [open transitions](../STATUS.md#open-transitions); the 2026-11-07 ledger private-build/registry versus pin-release split illustrated this |
| Evidence budget | At most one small read-only confirmation batch; never rerun an unchanged green identity. At most eight concrete findings with file/symbol or repro evidence |
| Verdict | `pass` only with no material finding **and** no missing required evidence; `verdict: pass\|changes\|blocked`, `findings`, `validation_gaps`, `confidence` and [common fields](agent-fleet.md#briefs) |

## Breaker

Use one before accepting player-visible work and before tag/announcement, as
required by the [QA protocol](../QA_AGENT_PROTOCOL.md#roles). Unaccepted work
may be pushed to `main`; a push is not acceptance.
The author’s happy path/test plan is not the breaker’s plan.

- Seek reproducible failures of the exact commit/branch/tag/assets. Never edit
  target production code, tests or docs; no commits/push/tags/publication.
- Scratch repros/fixtures stay in isolated ignored artifacts. Negative controls
  use disposable copies, not frozen target. Before destructive/security/migration
  attacks copy **all affected state**, including previous-release worlds; never
  attack owner originals/live services.
- Each run: private ports, data, `XDG`/`HOME`, Xvfb; loopback only; kill only own
  processes. Cargo uses Main's `<work>/solaris/tools/dev/mem-run.sh`; no sleeps.
  Browser/noVNC viewers require proven headless isolation; track/close whole instances.
- Brief: target+base, absolute `<work>/wt/` path, immutable server and hashes
  (reuse candidate), claims/ADR/area owner, known-working
  [client environment](real-client-gates.md).
- Return exactly protocol YAML and final `QA_RESULT`. Main deduplicates, routes
  confirmed defects to owners and reruns failed attacks after fixes.

### Large bug hunts ("QA armada", owner request only)

RC2 (2026-10-29) used 22 slices. Today use QA P0–P3, not its old
severe/moderate/mild scale.

| Stage | Procedure |
|---|---|
| Freeze | CI-style client `packaging/stage-singleplayer.sh` output plus release server in one package; hash every binary, no rebuild during hunt |
| Unpublished CDN | One loopback `python3 -m http.server <port> --bind 127.0.0.1` named service with `krasnokamen-content.json`, facts/content names, sizes and SHA from launcher pins. Canary: `env -i HOME=... XDG_DATA_HOME=<fresh> PATH=/usr/bin:/bin krasnokamen-launcher --headless-bootstrap --manifest <mirror>`; expected `Game content ready` (~650 MB historically). Copy to `template-data/` with mirror in `launcher.json` (`--manifest` is not persisted); workers copy template |
| Worker isolation | Own `Xvfb :$((100+N)) -screen 0 1280x720x24 -nolisten tcp`, null audio per gate guide, ports `42000+10*N`, private dirs, read-only repos. XTest via `python-xlib` in `uv` venv (host Python lacks pip); Xvfb has no WM: focus mapped top-level window before input, take fresh screenshot after |
| Destination | Launcher `--play` defaults to public test server: explicitly use `--server 127.0.0.1:<port>` or Direct Connect/Singleplayer |
| Shared brief | Read first: [bug blindness](https://danluu.com/bug-blind/), normal player/admin and vanilla oracle (not our code/docs/tests), workarounds are bugs, “nothing found” needs coverage. Ordered phases: fresh-user docs/UI (~45 min in RC2), code hypotheses, then restart/disconnect mid-action, full inventories, borders, two players, odd inputs; include known issues |
| Evidence | `findings/s<N>-<area>.yaml`: id/severity/title/phase/steps/expected/actual/repro n/m/evidence/`file:line` |
| Coverage | Install, singleplayer, UI/rendering, survival, inventory/crafting, containers/items, redstone/physics, mobs/combat, vanilla-jar worldgen, dimensions, persistence kill-9/WAL, multiplayer/admin, previous-version clients, settlements/plugins/security, two-CPU (`taskset`) lag feel, robustness/misc. Batch briefs share “read the brief first” |
| Close | Deduplicate; Main re-verifies severe findings, routes by area, reports counts/top findings |

## Core Committee (collective design decisions)

The owner delegated **some** design decisions to read-only members, one per owner-chosen
model. Main orchestrates/records, never decides alone or asks the owner a
delegated question the committee can resolve. Use normal subagents, **not shell
loops**. Member tools: read/grep/glob/language server.

**Member brief:** identify roster and collective authority; read AGENTS first;
no writes/mutations; answer independently from current code/docs with `file:line`.
Verify other members' claims and stale citations. Prefer simplest correct design;
name evidence-backed dupes/double-apply/crash atomicity/rollback/client artifacts/
1–2-core budget holes. Bounded reads, no repo surveys, ≤1500 words in Russian;
end each item `VOTE <item>: approve|amend|reject — <one sentence>`.

1. Main writes ignored `.analysis/committee/q<NNN>-<slug>.md`: facts,
   measurements, source pointers, lettered questions, exact vote items.
2. Spawn all members in one batch with question path; late/expensive members
   read prior answers and add only new evidence. Main saves raw answers at
   `.analysis/committee/q<NNN>/<m>.md` (members cannot write them).
3. Target-read disputed facts, weigh advisor warnings; `decision-<NNN>-<slug>.md`
   records votes/majority/minority, amendments `<NNN>-a`, `-b`, rejected options
   with reasons and ordered slices with measurable acceptance; send to implementer.
4. Small amendments: one-line approve/amend from idle members. Owner addenda go
   to every running member; roster changes require one ballot ratifying all
   open decisions and cancellation of removed members' jobs.

Unanimously wrong citation is a must-fix amendment. Verify extrapolated numbers:
batch p99/count is not per-item p99, and shared tick budgets do not scale with
population. Root-cause disagreement requires measurement first.

## Worker role prompt

Use [AGENTS](../../AGENTS.md), the assigned brief and
[fixed result templates](agent-fleet.md#briefs), not a second copied constitution.
The brief overrides the following defaults:

- Absolute assigned-worktree paths; `wt-new`, authorized `wt-done`, release gates
  only from `pin-release`. Default **no commits**; report
  `git diff <base> --binary | sha256sum`.
- One heavy command at a time through Main's runner; `-x`/`-p` serialize without
  GO. Stale slot-era skills lose to AGENTS (14 removed on 2026-11-07).
- Smallest correct change; behavioral fix needs fix-only negative control;
  record fail-first if observed, not a gate. Workers add regression/repro
  references, never mark rows fixed; Main uses `ledger promote`.
- Close reachable gaps including observer/harness/test support. Outside blockers
  (owner/hardware/credentials) → `partial` with exact blocker, not a question.
  Two failed graphical starts: repair if possible, otherwise `blocked` with receipts.
- ≤3 messages to Main; peer acknowledgements through `write`, not final result.
  Last message is result YAML. Read-only audits never mutate and use
  `verdict: pass|changes` with concrete `file:line` failure scenarios.
- No subagents without brief authorization; never parts under 30 minutes.
  Spawn-enabled variants delegate disjoint parallel/mechanical slices with exact
  files/acceptance, review every child diff and run its tests yourself.
  Design-heavy variants state invariant, simplest design and replacement before code.
