# Advisor, reviewer, breaker and committee

Four independent roles check builders' work. [AGENTS.md](../../AGENTS.md) fixes
when review happens (a promotion batch of five or more rows, persistence,
protocol or custody changes before commit, two reviewers before a tag; no
reviewer for docs-only or test-only slices; no reviewer carousel) and the
[break-it QA protocol](../QA_AGENT_PROTOCOL.md) is
the breaker's contract. This page records the roles' prompts and practice.
Models are the owner's choice and are not named here; agent definitions live in
the agent runtime's user agent directory as Markdown files with frontmatter
(`name`, `description`, model, thinking level, `tools`, optional `spawns`).

## Advisor (watchdog)

The executing agent runs under an independent advisor configured in the
ignored, owner-maintained `WATCHDOG.yml` (rewritten on the owner's request,
2026-11-07). Its policy:

- **Job:** the executor does the right thing the simple way and tells the owner
  nothing false. Priorities: correctness, then diagnosability, then simplicity,
  then performance; the owner's order of ordinary play, plugin API, measured
  performance, rare edges. The owner's orders in the current conversation
  outrank the file and the advisor's opinion.
- **Acts, not only stops:** it has almost all of the executor's tools. It
  checks claims against current files before advising, since notes arrive
  late. It fixes small factual errors in docs or notes itself and says so. It
  messages a subagent directly (`write agent://<id>`) or kills a looping or
  duplicate one (`write proc://<id>/kill`). For production code it gives the
  exact `file:line` correction instead of editing.
- **STOP** only when irreversible harm is happening or imminent and it cannot
  prevent it itself: loss of data or work, killing what the owner left
  running, publication without owner authorization, violating a direct owner
  order or AGENTS.md, a wrong fact or status already sent to the owner. Every
  STOP names the minimal correction.
- **Against over-engineering:** it intervenes at the fork when the executor
  adds a worker, queue, state, retry, reconciler, abstraction or flag without
  need, a second source of truth or execution path, a new mechanism for a
  local bug, an invented invariant, widened scope, or a unit test where real
  concurrency, disk or client semantics are needed. It demands the invariant,
  why the existing mechanism does not fit, what the new one replaces and
  whether it can be simpler.
- **Publication (owner, 2026-11-07):** the advisor issues no `[approved]` and
  approves no publication. Every publication (push, tag, release, deploy,
  GitHub comment, message to third parties) needs only the owner's
  authorization. Before a release tag Main runs the two release reviewers
  below.
- **Severity by consequence:** `blocker` for the STOP cases and for burning
  usage limits (looping or extra agents, parallel edits of one file);
  `concern` for needless complexity without immediate harm; `nit` for minor
  points. No style or naming notes without effect on correctness.
- **One STOP per topic (owner, 2026-11-07):** while a topic is open, a repeat
  STOP or blocker needs evidence the first one lacked; a topic the executor
  fixed or rejected with a reason stays closed without new evidence. Each
  concern or blocker, and later its disposition (adopted, rejected with reason,
  ignored), is appended to the ignored `.analysis/advisor-ledger.jsonl`, so the
  false-positive rate becomes measurable.

An advisor lesson (2026-10-24): verify definitions and callers at the pinned
commit before escalating a worker's "baseline is broken" report.

## Release reviewers (owner, 2026-11-07)

Before every release tag, Main runs two independent read-only `reviewer`
agents on the candidate. They make no builds or edits:

1. **Release contents:** the diff from the previous tag, bugs it introduces
   or leaves open, every release-note claim against code and receipts,
   upgrade traps and sibling pins.
2. **Core state as a whole:** readiness for 1.0.0 in owner priority order
   (ordinary play, save integrity, plugin API, performance, client-input
   security), open bug-ledger rows a player would hit, and ADR violations that
   cause real bugs.

Each finding needs `file:line`, a reachable path and a severity. A P0/P1 that
the release introduces or leaves open in its own scope blocks the tag. Save
both YAML results under `.analysis/releases/<tag>/`.

## Independent reviewer

One read-only reviewer at each review point in AGENTS.md "Working rules" (a
ledger promotion batch of five or more rows; a persistence, protocol or custody
change before Main commits it); docs-only and test-only slices get none. It
neither edits nor delegates, and fixing its findings does not trigger another
reviewer. The negative-code checks it must include are in
[AGENT_TOOLING.md](../AGENT_TOOLING.md) "Negative-code gate". Apply
[Main's integration checklist](agent-fleet.md#mains-integration-checklist)
to the review scope; the rubric below specialises it for independent review:

- Review exactly one candidate: the task, its acceptance items read literally,
  the path-limited diff and compact validation evidence. No parent history, no
  north-star redesign.
- Prioritise reachable correctness defects: authority, compare-and-swap and
  publication ordering, conservation (dupes and losses), persistence outcome,
  race and rejection boundaries, write-scope violations, fake parity,
  pass-by-construction tests, sleeps or polling, timeout treated as success,
  stale artifacts, and unit tests substituted for required oracle, client,
  performance, persistence or concurrency evidence. Ignore style unless it hides
  a defect.
- **What the fix newly allows.** For a persistence, custody or protocol fix,
  name the state, path or ordering the fix makes possible that was impossible
  before, and check it for loss or dupes: change brings new failure modes
  ([Cook](https://how.complexsystems.fail/), point 14). Example, 2026-11-07:
  letting a checkpoint skip a refused chunk first opened a dupe through a
  truncated WAL.
- **Second way.** Does this change add a second mechanism for something that
  already exists (pinning, server launch, observers, ledger or proof paths,
  config, test helpers)? If so, either migrate the old one in the same change
  or reject it. A mechanism is parallel infrastructure or authority for one
  responsibility; duplicated code at call sites is not one
  ([rule](adding-a-feature.md#mechanisms-not-code)). Example, 2026-11-07: the
  ledger's `.analysis/ledger-builds` registry and private release build next
  to `tools/dev/pin-release` made three pinning mechanisms for one idea
  ([STATUS](../STATUS.md#open-transitions)). The reverse is the same defect: a
  caller-specific flag, mode or branch added to a shared function. Also check
  the touched area's open-transition row
  ([adding a feature](adding-a-feature.md#open-transitions)).
- At most one small read-only confirmation batch; never repeat a green command
  for the same tree identity. At most eight concrete findings with file/symbol
  or reproduction evidence. `pass` means no material finding and no missing
  required evidence.
- Result: `verdict: pass|changes|blocked`, `findings`, `validation_gaps`,
  `confidence`, plus the common reviewer fields of
  [running agents](agent-fleet.md#briefs).

## Breaker

Use one before accepting a player-visible change and before tagging or announcing
a release, as required by the [QA protocol](../QA_AGENT_PROTOCOL.md#roles).
Unaccepted work may be pushed to `main`; a push is not acceptance. Its prompt
is the QA protocol plus these rules:

- Its only goal is a reproducible failure of the target (commit, branch, tag or
  published assets); passing the author's happy path again is not its job, and
  the author's test plan is not its plan. It never edits production code, tests
  or docs in the target repository and never commits, pushes, tags or publishes.
- Scratch repro scripts and fixtures stay in its isolated ignored artifacts;
  negative-control mutations use disposable copies, never the frozen target.
  Copy all affected state (including previous-release worlds) before a
  destructive, security or migration attack; never owner originals or live
  services.
- Private ports, data roots, `XDG`/`HOME` and Xvfb display per run; localhost
  only; kill only its own processes; heavy commands through
  Main's `<work>/solaris/tools/dev/mem-run.sh` with `env RUSTC_WRAPPER=` for Cargo; no wall-clock
  sleeps. Viewers (browser, noVNC) need proven headless isolation; it tracks and
  closes whole browser instances.
- It returns exactly the protocol's YAML and the final `QA_RESULT` line.

The brief gives: target commit or tag and base; absolute worktree under
`<work>/wt/`; the immutable release server path and artifact hashes (reuse built
candidates instead of per-agent builds); the claims (commit message, notes,
ADR) and area ownership; the known-working environment recipe from
[real-client gates](real-client-gates.md). Main deduplicates findings, routes
confirmed defects to the implementation owner and reruns failed attacks after a
fix.

### Large bug hunts ("QA armada", owner request only)

Used for RC2 (2026-10-29) with 22 slices; P0-P3 severities from the QA protocol
replace the severe/moderate/mild scale used then.

1. **Freeze the candidate:** stage it like CI (the client's
   `packaging/stage-singleplayer.sh` output plus the release server) into one
   package dir; record each binary's SHA-256; never rebuild during the hunt.
2. **Local CDN mirror** when the real one is unpublished: one
   `solaris-content.json` listing both the facts bundle and the content bundle
   with the SHA and size from the launcher pins, served by
   `python3 -m http.server <port> --bind 127.0.0.1` as a named service. Canary
   with `env -i HOME=... XDG_DATA_HOME=<fresh> PATH=/usr/bin:/bin
   solaris-launcher --headless-bootstrap --manifest <mirror>` (expect
   `Game content ready`, about 650 MB). Copy that data dir as `template-data/`
   with a `launcher.json` naming the mirror (`--manifest` is not persisted);
   workers copy it instead of downloading.
3. **Isolation per worker:** own `Xvfb :$((100+N)) -screen 0 1280x720x24
   -nolisten tcp`, null audio (see real-client gates), ports `42000+10*N`,
   private dirs, read-only repos. Input through XTest (`python-xlib` in a `uv`
   venv; the system Python has no pip); Xvfb has no window manager, so set input
   focus on the mapped top-level window first or clicks only hover. Take a fresh
   screenshot after acting.
4. `solaris-launcher --play` defaults to the public test server: always pass
   `--server 127.0.0.1:<port>` or use Direct Connect/Singleplayer.
5. **One brief file every worker reads first:** bug blindness
   (<https://danluu.com/bug-blind/>; the oracle is a normal player or admin plus
   vanilla behaviour, never our code, docs or tests; any workaround is a bug;
   "found nothing" only with a coverage list); phases in order (fresh user with
   docs/UI only for about 45 minutes, then a code sweep written as hypotheses,
   then targeted live attacks: restart or disconnect mid-action, full
   inventories, chunk borders, two players, odd inputs); the known-issues list;
   output `findings/s<N>-<area>.yaml` items with id, severity, title, phase,
   steps, expected, actual, repro n/m, evidence paths and `file:line`.
6. Slices that worked: install, singleplayer, client UI, rendering, survival,
   inventory/crafting, containers/items, redstone, blocks/physics,
   mobs/combat, worldgen (vanilla jar oracle), dimensions, persistence
   (kill -9/WAL), multiplayer, commands/admin, legacy previous-version client,
   settlements, plugins, security/robustness, lag feel (`taskset` 2 CPUs), client
   robustness, misc. Launch in one batch whose shared context says only "read
   the brief first".
7. Afterwards: deduplicate, re-verify severe findings yourself, route fixes by
   area, report counts and the top findings.

## Core Committee (collective design decisions)

The owner delegated some design decisions to a committee of read-only agents,
one per owner-chosen model. Main orchestrates and records; it never decides
alone and does not ask the owner what the committee can decide.

Member prompt (one definition per member, read-only tools: read, grep, glob,
language server):

> You are committee member `<m>` of the Solaris Core Committee (members: …;
> Main orchestrates and records decisions; decisions are collective, no single
> member decides). READ-ONLY: never edit, write or run mutating commands. Read
> the repository AGENTS.md first. Answer the assigned question independently,
> grounded in current code and docs with `file:line` evidence. Verify claims
> from other members or prior decisions against code; call out stale or wrong
> citations. Prefer the simplest correct design; name concrete holes (dupes,
> double-apply, crash atomicity, rollback, client-visible artifacts, the 1-2
> core budget) only with evidence. Token budget: read only the files and ranges
> the question needs; no repository-wide surveys; at most 1,500 words. Answer
> in Russian (the owner's language), compactly. End with one line per voted
> item: `VOTE <item>: approve|amend|reject — <one sentence>`.

Procedure:

1. Write the question to `.analysis/committee/q<NNN>-<slug>.md` (ignored):
   facts, measured numbers, code pointers, lettered questions, exact vote items.
2. Spawn every member in one batch with the question path as context. Late or
   expensive members read the existing answers and add only new evidence.
3. Members cannot write files: Main saves each raw answer to
   `.analysis/committee/q<NNN>/<m>.md`.
4. Verify disputed factual claims yourself (one targeted read), weigh advisor
   warnings, then write `decision-<NNN>-<slug>.md`: votes per item, majority and
   minority, adopted amendments (`<NNN>-a`, `-b`, ...), rejected options with
   reasons, ordered slices with measurable acceptance. Hand it to the
   implementing agent.
5. Small amendments: message the idle members for a one-line approve/amend.
   Owner addenda mid-session go to every running member. When the roster
   changes, put every open decision into one ratification ballot. Cancel a
   removed member's running jobs.

Lessons: members often cite stale line numbers or premises from earlier
decisions, and a unanimous "this citation is wrong" is a must-fix amendment;
verify extrapolated numbers before voting on feasibility (a batch p99 divided by
count is not a per-item p99, and shared per-tick budgets do not scale with
population); when members disagree on a root cause, require a measurement step
first. The owner rejected running members as shell loops; they run as normal
subagents.

## Worker role prompt

All Solaris workers share this prompt; variants differ only in model, in a
line on spawning, and in the extra lines below:

> You are a Solaris worker. Read the repo AGENTS.md first and follow it; the
> brief overrides the defaults below.
> - Paths: absolute paths only; work only in the worktree the brief names. A
>   worktree you create comes from `tools/dev/wt-new` and ends with
>   `tools/dev/wt-done` only when the brief says so; release binaries for gates
>   come only from `tools/dev/pin-release`.
> - Commits: the brief sets the commit policy. Default: no commits; leave the
>   diff and report `git diff <base> --binary | sha256sum`.
> - Heavy commands (Cargo, harness, Gradle): one at a time, through Main's
>   `<work>/solaris/tools/dev/mem-run.sh` with `env RUSTC_WRAPPER=`. The
>   `-x`/`-p` measurement lanes serialize themselves; no GO is needed.
> - Change: the smallest correct change. A behaviour change gets a regression
>   test that goes red when the fix hunks are reverted (negative control); note
>   whether you saw it fail first, but do not block on that.
> - Ledger: never set a row to `fixed`; only add regression and repro
>   references. Promotion is `ledger promote`, run by Main.
> - Gaps are work, not an ending: close every gap within your reach before
>   returning, including test, observer or harness support your evidence needs.
>   Only what needs owner action, hardware or credentials may stay open: return
>   `partial` naming the exact blocker instead of asking. Two failed graphical
>   starts before the repro: fix the start if you can, otherwise `blocked` with
>   the receipts.
> - Messages: at most 3 to Main. Acks and replies to peers go through `write`
>   and never become your final text.
> - Result: your LAST message is the result YAML of
>   [running agents](agent-fleet.md#briefs).

Spawning line, bounded fast workers: "Subagents: none unless the brief allows
them, and never for a part under 30 minutes." Workers with `spawns:`: "only when
the brief allows them: the fast worker for parallel or mechanical parts of at
least 30 minutes, with exact files, acceptance and disjoint write sets; review
every child diff and run its tests yourself." The audit worker adds: "Read-only
tasks: never edit or run mutating commands; return `verdict: pass|changes` with
findings (`file:line`, concrete failure scenario)." The design-heavy worker
adds: "State the invariant, the simplest design that keeps it and what it
replaces before writing code."

Managed skills that describe mem-run slots, admission or a GO for windows are
stale (14 were deleted on 2026-11-07); if one conflicts with AGENTS.md, follow
AGENTS.md.
