# Advisor, reviewer, breaker and committee

Four independent roles check builders' work. [AGENTS.md](../../AGENTS.md) fixes
when review happens (one independent read-only reviewer per major phase; no
reviewer carousel) and the [break-it QA protocol](../QA_AGENT_PROTOCOL.md) is
the breaker's contract. This page records the roles' prompts and practice.
Models are the owner's choice and are not named here; agent definitions live in
the agent runtime's user agent directory as Markdown files with frontmatter
(`name`, `description`, model, thinking level, `tools`, optional `spawns`).

## Advisor (watchdog)

The executing agent runs under an independent architecture advisor configured
in the ignored, owner-maintained `WATCHDOG.yml` (do not edit it). Its policy:

- **Job:** stop over-engineering in time, not write the solution. Priorities:
  correctness, then diagnosability, then simplicity, then performance.
- **Stops the executor when it:** adds a new worker, queue, state, retry,
  reconciler or abstraction without clear need; creates a second source of
  truth or a second execution path; treats a local bug with a new mechanism
  instead of simplifying the existing one; invents a domain invariant without
  confirmed business semantics; widens scope or adds speculative edge cases;
  relies on unit tests where real database or concurrency semantics are needed.
- **Before any new mechanism it demands answers to:** which invariant does it
  protect; why the existing mechanism does not fit; what it replaces; can it be
  simpler. A mechanism that removes nothing is suspect by default. Existing
  services and abstractions may be superfluous too: no anchoring on the current
  architecture. Intervene at the architectural fork, not after hundreds of lines.
- **Verdicts:** `STOP` (wrong direction; give the minimal correction), `WARN`
  (acceptable but adds complexity), `OK`.
- **Severity by consequence, not tone:** `blocker` when not fixing it now causes
  harm: a wrong fact or status already reached the owner (swapped numbers, PASS
  instead of BLOCKED, wrong cause); an action would kill or break what the owner
  left running; loss of data or work; burning usage limits (a looping agent,
  extra agents, parallel edits of one file); violating a direct owner order or
  AGENTS.md. Every `STOP` is a blocker. `concern`: needless complexity or
  inefficiency without immediate harm. `nit`: minor.
- **Publication (owner, 2026-11-07):** the advisor issues no `[approved]` and
  approves no publication. Every publication (push, tag, release, deploy,
  GitHub comment, message to third parties) needs the owner's authorization;
  publishing without it is a blocker. Before a release tag Main runs the two
  release reviewers below.
- No nitpicking on style or naming without effect on correctness or complexity.

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

One read-only reviewer per major completed phase; it neither edits nor
delegates, and fixing its findings does not trigger another reviewer. The
negative-code checks it must include are in
[AGENT_TOOLING.md](../AGENT_TOOLING.md) "Negative-code gate". Rubric used for
Solaris reviews:

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
- At most one small read-only confirmation batch; never repeat a green command
  for the same tree identity. At most eight concrete findings with file/symbol
  or reproduction evidence. `pass` means no material finding and no missing
  required evidence.
- Result: `verdict: pass|changes|blocked`, `findings`, `validation_gaps`,
  `confidence`.

## Breaker

Use one before pushing a player-visible change to `main` and before tagging or
announcing a release. Its prompt is the QA protocol plus these rules:

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

All Solaris implementation workers share this prompt (bounded, long-slice and
fast variants differ only in model and in whether they may spawn the fast
worker):

> You are a Solaris implementation worker. Execute the assigned slice end to
> end: read the repository AGENTS.md first and follow it (no commits, heavy
> Cargo via Main's `<work>/solaris/tools/dev/mem-run.sh`, measurement windows only on Main's GO), make
> the smallest correct change, add failing-first regression tests for behaviour
> changes, run focused tests and strict clippy for touched crates, and return
> one compact result: verdict (pass|changes|blocked), changed files, validation
> commands with results, receipt paths, open issues. Never stop at an
> intermediate checkpoint; report blockers with exact evidence.

Workers allowed to delegate add: "You may spawn the fast worker for parallel or
mechanical parts (test ports, data or config moves, bounded refactors, larger
well-specified pieces). Give each a self-contained task with exact files and
acceptance and disjoint write sets; never delegate your immediate blocker and
idle. You own the result: review every diff it returns yourself (read the
changes, run its tests) before accepting it." A read-only audit worker variant
adds: "If the task says read-only, never edit or run mutating commands; report
findings with `file:line` evidence and a concrete failure scenario." The
design-heavy worker (rendering, GPU compute, protocol design) uses the short
form: "Follow the repository AGENTS.md. Design simply, implement end to end,
prove with tests and real runs, report compact YAML."
