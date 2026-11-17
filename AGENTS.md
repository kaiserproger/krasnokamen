# Agent Guide — Krasnokamen

Krasnokamen is a Rust Minecraft Java server serving every release from 26.1.2
through 26.3 (775–777), with canonical 26.3 and an adjacent protocol chain.
Content import installs all version caches; bumps add steps and retain old clients.
The current release candidate is 1.0.0-rc.5 "Pauline" ([status](docs/STATUS.md)).
The binary is `mc-edge`'s
`krasnokamen` over the native core `mc-core`; v1 is retired. `CLAUDE.md` is a
symlink to this guide. The owner is `kaiserproger <kaisergrobe@gmail.com>`;
never change Git configuration.

## Words used here

- **Owner**: kaiserproger. Decides scope, priorities, commits and publication.
- **Owner host**: the shared agent machine (since 2026-11-10 the Aeza dedicated
  server muddy-lime; the owner's laptop is only his terminal), where many agents share one machine.
  You are on it when `/run/media/kaiserroman/games/solaris-work/solaris/tools/dev/mem-run.sh`
  exists. Rules marked *owner host* apply only there; elsewhere run commands directly.
- **Main**: the agent the owner talks to. It plans, delegates and integrates.
  Other agents are workers, reviewers or breakers.
- **`<work>`**: the owner host's work root `/run/media/kaiserroman/games/solaris-work`
  (sibling checkouts, worktrees `wt/`, build outputs `targets/`, `receipts/`).
- **Receipt**: the exact command, tree and log path behind a claim.

## Which rule wins

1. The owner's latest explicit instruction. `Stop`, `стоп`, `забей` or `поправка` replace the current task at once;
   inspect your running processes and worktrees safely before switching.
2. The [Agent Constitution](docs/AGENT_CONSTITUTION.md); it overrides task prompts.
3. This file. Its working rules restate the [owner contract](docs/development/owner-contract.md) (verbatim quotes).
4. The page that owns the area (table below), then the [development playbooks](docs/development/README.md).
5. Prompts, skills, notes and memory outside the repository.

Code, tests, config and runtime evidence beat every document: fix the document.

## Working rules

**Current mode (owner, 2026-11-15):** the 21 November announcement is a finite
stabilization, not an expanding `/goal`. Its
[acceptance and stop condition](docs/DEFINITION_OF_DONE.md#finite-announcement-acceptance)
and [operating mode](docs/development/agent-fleet.md#announcement-stabilization-mode)
(four to five executing children in total; reviewers and breakers outside the cap)
override the general queue and fleet defaults below.

1. Deliver what was asked, end to end for the real case, as simply as possible:
   the 20% that gives 80%. No speculative abstractions, flags, hardening, rare
   edges or unrelated churn; a rare blocker gets a clear error, not a subsystem.
2. A clear request needs no reconfirmation. Ask the owner only about a material
   choice that code, docs or history cannot settle. Do more than was asked only
   after you explain exactly why it is needed and the owner agrees.
3. Finish the current task before a newer request unless the owner says to
   switch now. `Retry` or `продолжай` means revalidate the state, then continue.
4. Report at meaningful checkpoints, not per command: at most three substantive
   updates (non-obvious outcome, blocker, closeout). A progress report covers
   only that interval's commits, capabilities, gates and time sinks.
5. Check your own work. Independent read-only review happens at three points:
   a ledger promotion batch of five or more rows; a change to persistence,
   protocol, custody or the battle core before Main commits it (and the design
   of a client threading or entity-tick change before implementation and its code
   before merging), under the [portable critical-review policy](docs/development/review-committee-and-breaker.md#independent-reviewer)
   (permanent owner decision, 2026-11-15); two release reviewers before a tag.
   Docs-only and test-only slices get no reviewer. The owner contract authorizes
   these reviewers; fixing their findings needs no second review.
6. An autonomous `/goal` covers the approved objective, not a slice; a context
   handoff neither pauses nor completes it. For this announcement the owner
   explicitly authorized the finite acceptance and stop condition linked above.
   Other goals continue until verified or externally blocked with nothing
   actionable. Slices need disjoint interfaces, not duplicated full gates.
7. Priorities: ordinary vanilla-client gameplay and multiplayer; the production
   plugin API and adapters; measured optimization, ECS, ownership and
   autoscaling; rare edges. A lower item moves up only when it blocks play,
   saves or plugins.
8. Main checks the owner's open issues hourly and before a commit (workers do
   not): `gh issue list -R kaiserproger/krasnokamen --author kaiserproger --state open`.
   During announcement stabilization, classify new findings under the
   [finite queue admission rule](docs/DEFINITION_OF_DONE.md#finite-announcement-acceptance):
   only its safety, ordinary advertised-path or frozen-criterion dependencies
   enter the blocking queue; record everything else as visible backlog.
   Outside that mode, issues are the queue outside a `/goal` and join its queue
   inside one.
9. Preserve the complete ordered queue, including blocked and deferred items.
   Only an explicit owner change or a concrete recorded dependency may change
   order; blockers pause an owned item, not silently delete or demote it.
   Side requests append. Before dispatch after compaction/resume, reconcile
   the full queue and actual TODO view under the
   [handoff rules](docs/development/agent-fleet.md#handoff-across-context-loss-or-to-another-person).

## Where to look

| Task | Start with | Also |
|---|---|---|
| build, run, configure, back up a server | [README](README.md#build-from-source), [operating guide](docs/OPERATING.md) | [CONTRIBUTING](CONTRIBUTING.md) prerequisites, [migration](docs/MIGRATION.md) |
| change or add gameplay | [adding a feature](docs/development/adding-a-feature.md) | [mutations](docs/architecture/mutations.md), [state ownership](docs/architecture/state-ownership.md), [compatibility](docs/COMPATIBILITY.md) |
| any change in an area being replaced | [open transitions](docs/STATUS.md#open-transitions) | [the rule](docs/development/adding-a-feature.md#open-transitions) |
| packets and protocol | [protocol changes](docs/development/protocol-changes.md) | ADRs [0002](docs/decisions/0002-vanilla-protocol-metadata-as-reference.md), [0015](docs/decisions/0015-multiversion-protocol-translation.md) |
| saves, WAL, checkpoints, recovery | [persistence](docs/architecture/persistence.md) | [residency](docs/architecture/residency.md), [ADR 0013](docs/decisions/0013-core-model-v2.md) |
| settlements | [settlements architecture](docs/architecture/settlements.md) | [owner decisions](docs/settlements/OWNER_DECISIONS.md), [player guide](docs/SETTLEMENTS_GUIDE.md) |
| plugins and SDK | [plugins](docs/PLUGINS.md) | `../solaris-default-plugins` |
| client adapters | [Krasnokamen Loader](docs/KRASNOKAMEN_LOADER.md) | `../solaris-loader` |
| native client, launcher, singleplayer | `../solaris-client` (its own README and AGENTS) | [client release](docs/development/release-and-tags.md#client-release-and-the-singleplayer-content-bundle), [Windows and macOS QA](docs/development/windows-and-macos-client-qa.md) |
| tests, harness profiles | [agent tooling](docs/AGENT_TOOLING.md) | [definition of done](docs/DEFINITION_OF_DONE.md) |
| real-client gate | [real-client gates](docs/development/real-client-gates.md) | [break-it QA protocol](docs/QA_AGENT_PROTOCOL.md) |
| file, fix or close a bug | [bug ledger](docs/bugs/README.md) | |
| performance | [profiling](docs/development/performance-windows-and-profiling.md) | [profiles](docs/performance/PROFILES.md), [catalog load](docs/development/catalog-load.md) |
| release or tag | [releases and tags](docs/development/release-and-tags.md) | [release notes](docs/releases/README.md) |
| test server | [test server operations](docs/development/test-server-ops.md) | |
| new Minecraft version | [version bump](docs/development/minecraft-version-bump.md) | |
| run, brief or review agents | [running agents](docs/development/agent-fleet.md) | [reviewer, breaker, committee](docs/development/review-committee-and-breaker.md) |
| architecture decision | [ADR index](docs/decisions/README.md) | [architecture overview](docs/ARCHITECTURE.md) |

Everything else: the [documentation index](docs/README.md).

Layout: `crates/mc-core/` authoritative gameplay, native tick, snapshots,
intents and merge, WAL and checkpoints; `crates/mc-edge/` CLI, config, sockets,
auth, console, dashboard and connection presentation. Offline Anvil interchange,
census and probes belong to `crates/mc-world-conversion/`; pregeneration,
prune/compact and revision upgrades belong to `crates/mc-world-tools/`. Both
depend on the core, never the reverse. Other `crates/` own protocol, data, world,
worldgen, physics, plugins, Loader and tooling.
`crates/workspace-features/` unifies third-party features for the server build and
core tests without expanding standalone lower crates; it has no runtime API and
must never depend on a Krasnokamen crate.
`sdk/rust/` is an independent guest SDK workspace; `tools/harness/` is canonical validation;
`tools/dev/` holds [owner-host scripts](tools/dev/README.md). `example.toml` is
the normal config; `playable.toml` the harness survival config.

On the owner host an ignored `.analysis/MEMORY.md` may hold a resume cursor. It
is optional, absent in a fresh checkout, and loses to code and tracked docs;
`.analysis/` archives and `.ai-bridge/` are not startup context.

## Build and validation

From the repository root (debug by default):

```sh
cargo build -p mc-edge --bin krasnokamen
cargo run -p mc-edge --bin krasnokamen -- content import --version 26.3 --download
cargo run -p mc-edge --bin krasnokamen -- --config example.toml
cargo build --release --locked -p mc-edge --bin krasnokamen   # real-client gates, deploys, releases
python3 -m tools.harness list
```

| Level | Commands | When |
|---|---|---|
| L0 | `python3 -m tools.harness run test -p <crate> --lib <filter>` for the affected scope, plus diff/syntax checks of what you touched | while editing |
| L1 | L0 plus `python3 -m tools.harness run fmt` and `python3 -m tools.harness run code-health` | finishing a slice |
| L2 | `python3 -m tools.harness run correctness` (regressions, fmt, code-health, strict workspace Clippy, all nextest workspace/all-target tests) | before a code commit, at major phase close, for a release |

- `python3 -m tools.harness run regressions` runs every registered bug check; L2
  already includes it, so run it alone only after an integration that skips L2.
  Markdown-only changes need `python3 tools/doclinks.py`, not Cargo or harness gates.
- *Owner host:* wrap heavy Cargo, harness and Gradle commands in Main's runner by
  absolute path: `<work>/solaris/tools/dev/mem-run.sh -m 6G <command>`
  (`-m 12G` for full `test`/`correctness`; at most 12G each), never a worktree's
  or sibling repo's copy. There are no memory slots: a compile command waits for
  one of three host-wide build tokens (a nested compile reuses its parent's),
  every other command starts at once. One agent holds at most one token: it
  starts its next compile command only after the previous one finished (owner,
  2026-11-18: one agent with two parallel runs held two thirds of the build
  capacity). The priority slice named by Main keeps the normal lane; every other
  agent compiles with `env KRASNOKAMEN_BUILD_LANE=background` (tokens 1-2 only,
  and a freed token goes to a waiting normal-lane compile first). The heavy-watchdog pauses plain cargo
  builds under real memory pressure and resumes them, never tests, servers or
  harness runs. The runner points
  `RUSTC_WRAPPER` at `tools/dev/sccache.sh` (shared sccache; empty when its daemon is
  down), so never pass `env RUSTC_WRAPPER=` or plain `RUSTC_WRAPPER=sccache`; an old
  `env RUSTC_WRAPPER=` still works but is uncached. The runner refuses release
  builds (`--release`, `--profile release*`) unless `tools/dev/pin-release`, the
  harness/ledger tooling or Main's override asks (owner, 2026-11-15): one release
  binary per integrated tree, reused only for the same tree and build inputs; a
  negative control gets its own reverted tree and its own binary. The runner makes
  Cargo incremental for debug/test builds and normal-lane pin builds, never in
  `-p`/`-x` or `--profile` builds: a binary for a performance window comes from
  `pin-release --no-incremental` (or `env CARGO_INCREMENTAL=0`), and bench,
  coreperf, the perf harness profiles and the battle runner refuse incremental pins. `-p` is serialized
  timing/profiling, never builds; `-x` is a quiet measurement window (pauses
  builds, waits for running tests and servers) that opens only inside
  `tools/dev/measurement-windows`. Both lanes serialize
  themselves: no GO is needed. Compare only matching lanes on a frozen tree
  named in the brief. Replace shared
  runner scripts atomically, never in place; cancel the parent job, not only its
  children. Details: [heavy commands](docs/development/heavy-commands-and-memory.md).
- A validation result is identified by command, tree fingerprint, environment and
  scope: never repeat an unchanged green check. After a failure, rerun only the
  affected gate after a relevant fix. Bug and perf work first records the exact
  command or seed repro in `.analysis/codex-logs/<slug>/`, then fixes and reruns
  it. A fix's regression test goes red when only the fix hunks are reverted
  (negative control); seeing it fail first is recorded, not required. A bug row
  becomes `fixed` only through `ledger promote` ([bug ledger](docs/bugs/README.md)).
- Harness runs own their artifacts and server/client/Xvfb lifecycle. Read
  `.analysis/validation/<UTC>-<profile>-<unique>/result.json` (`status`,
  `commands`, `exits`, `logs`, `details`, `error`). `prepared`, `--check` and
  `--prepare` are not a pass; unknown, unlaunchable or failing commands and
  timeouts fail closed. `code-health` is an architecture tripwire, not behavior,
  parity or performance proof; structural checks never compare Rust statement text or order.
  The AST-counted anti-pattern ratchets and the module-cycle ratchet are allowed: they count
  syn nodes against recorded ceilings ([agent tooling](docs/AGENT_TOOLING.md#code-health-ratchets)).
- Bug status follows the [bug regression contract](docs/bugs/README.md): a unit
  test cannot close a known client failure.
- Actual-client surfaces need actual-client gates (graphical Minecraft under
  Xvfb, exact scenario and evidence) and a separate breaker under the
  [break-it QA protocol](docs/QA_AGENT_PROTOCOL.md): an attack-matrix pass with
  reproduced, severity-ranked findings before a bug row becomes `fixed`, a
  surface is called accepted or a release is tagged. Pushes to `main` follow the
  commit-often rule (owner, 2026-11-07) and wait for neither.
  A happy-path gate alone is not acceptance. New or changed gates need a
  negative control, no pre-existing pass, every new branch on a real client, and
  at least 3 runs when variable. Say whether owner testing (PrismLauncher 26.1.2)
  or client MCP runs were owner-run, agent-run or not run. Graphical
  prerequisites include Linux inotify/pidfd/Xvfb, MCP events and client
  credentials; TCP or unit tests never substitute.
- Owner field testing comes before fixed-seed tuning, then varied-seed
  exploration with seeds and repros recorded. Terrain-blocked stays blocked; a
  failed owner or manual behavior stays blocked until the same or a stricter
  real-client gate passes. Use only `draft`, `stabilization` and `release-ready`
  per the [definition of done](docs/DEFINITION_OF_DONE.md); default to draft and
  never call skipped or manual-pending gates green. Broad gates prove only their scope.

## Execution and subagents

- Navigate with bounded search and reads (grep, read, LSP); bound paths and
  output. Verbose output goes to ignored `.analysis/codex-logs/`; report
  status, failures and receipt paths.
- Long work is event-driven: launch once, do independent work, consume the
  completion once. Main (the top-level session) may end its turn while its own
  background job runs; the job's result arrives as the next message. A
  subagent never ends its turn while a background job it started is running:
  an ended subagent is disposed and its jobs die with it (Perf1M's pins,
  CutTick's L0 and CutEdgePlan's re-check, 2026-11-17; owner, 2026-11-18). With
  nothing else to do it calls `wait` on that job. Never poll process, status or
  agent lists. Never add
  wall-clock sleeps in production, tests, harnesses or tools: wait for the exact
  message, packet, notification or process/world/simulation state (push, not pull).
- Main has standing authorization to spawn within the
  [current operating mode](docs/development/agent-fleet.md#announcement-stabilization-mode).
  Outside announcement stabilization, the general limit is 15 active agents
  plus 2 reviewers (owner, 2026-11-14), with 3 concurrent builders.
  Workers spawn only when their brief allows it.
  Larger fleets need explicit owner permission.
- **Generalists, not an agent per task (owner, 2026-11-16).** Main runs a small
  roster of long-lived generalists, one per lane with disjoint write sets; new
  work joins an existing lane's queue. A new agent starts only for a new lane
  or when a generalist's context is exhausted, and it takes over that lane's
  queue and handoff. While a build, test, gate or trial runs, a generalist works
  on its next queued task and consumes the completion when it arrives (no
  polling, no sleep); an empty queue is reported to Main as free capacity.
  Reviewers and breakers stay separate, read-only and, where possible, from a
  different vendor than the author. Main picks each lane's model by task and
  budget and records the choice and reason in PLAN
  ([roster](docs/development/agent-fleet.md#generalist-roster)).
  Give disjoint responsibilities and write sets, pass each task's base commit,
  paths and evidence contract (no conversation forks), never delegate the
  immediate blocker and then idle, and close generalists whose lane is empty.
  Exploit, fuzz, dupe or attack work goes only to workers whose role
  description warns of no provider filter for it.
- Delegated validation and QA use the harness lifecycle and receipts, not
  hand-rolled tmux/PID/log plumbing. Reviewers neither edit nor delegate.
- Each agent sends Main at most three messages per task (a blocker or a decision
  only Main or the owner can make). Each finished task ends with one compact
  YAML result (a generalist sends it as a message and continues its queue; its
  LAST message hands over the remaining queue):
  implementers `status: done|partial|blocked`, reviewers and breakers
  `verdict: pass|changes|blocked`, plus `changed_files`, `validation` and
  optional `report_path`. A gap is work, not an ending (owner, 2026-11-07):
  close every admitted gap within the bounded slice's reach; `partial` or `gaps` list only
  what needs something outside the agent's reach (owner action, hardware,
  credentials), each with its exact blocker. Main records every reported gap;
  the current mode's queue-admission rule decides blocker versus backlog.
  Ask nothing the brief answers. Acks to peers go through
  messages, never the final text. Long detail goes in a file; deduplicate
  notifications by agent id and revision. Brief and result templates:
  [running agents](docs/development/agent-fleet.md#briefs).
- In autonomous goal mode, do not load generic brainstorming, TDD, debugging or
  verification skills unless asked; mandatory tool skills still apply. If a
  higher-priority runtime requires them, obey and report the conflict.

## Project invariants

- Mojang bytes never enter Git. `.analysis/` and all of `data/vanilla/` are local
  and ignored. Packet IDs and layouts come from local `wire-probe`/`javap`
  ([protocol changes](docs/development/protocol-changes.md)), never memory;
  protocol and Anvil layouts need exact evidence. Gameplay and worldgen need
  minecraft.wiki-level vanilla similarity, not bit-exact RNG, AI cadence, Mth or
  terrain. Performance and correctness (no crashes, dupes or corrupt saves)
  outrank mob AI parity.
- Core v2 persists only the native store plus the tick WAL: no dual-write or
  Anvil fallback; corrupt or unsupported state fails closed. Gameplay authority
  stays in the native tick and intent merge; the edge owns presentation and
  connection state. Root orchestration routes work, not domain policy;
  substantial tests go in focused sibling `*_tests.rs` modules.
- New features are modular (owner, 2026-11-16): each lives in its own module,
  or its own crate when it needs no `mc-core` internals, with its state,
  systems and sibling `*_tests.rs` inside, a narrow explicit interface and one
  registration point owned by its host (for review and parallel lanes; it does
  not break a dependency cycle). It must not grow the number of top-level
  modules in `mc-core`'s largest cycle (code-health ratchet ceiling): a feature
  is a submodule of its domain host or a new top-level module that nothing in
  the cycle refers to, and a submodule's references count as its host's, so
  they can still pull modules in; check the count on the integrated tree.
  Existing code moves along the cycle-cut plan of [ADR 0021](docs/decisions/0021-build-time-and-crate-split.md)
  with byte-exact moves; before the 21 November announcement only once the current
  release tasks are closed, otherwise after (owner, 2026-11-17;
  [how-to](docs/development/adding-a-feature.md#new-features-are-modules)).
- Krasnokamen has no released compatibility surface of its own: migrate callers and
  delete superseded APIs, schemas, authorities, adapters, flags and fallbacks.
  Compatibility targets are the vanilla protocol and Anvil, not old Krasnokamen
  storage. Changes to architecture, authority, threading, waiting, persistence
  order or module policy update the owning ADR; a desired migration is not runtime truth.
- Check an area's row in [open transitions](docs/STATUS.md#open-transitions)
  before changing it: move it forward or add nothing to its old side; start no
  new transition in an area with one open; finishing one deletes the old side.
- Debug builds by default; `--release` only for a required performance, release
  or real-client gate. Real-client gates run a release server (owner,
  2026-10-27): pass the release binary as `KRASNOKAMEN_SERVER_BINARY` (on the owner
  host the `tools/dev/pin-release` pin of that tree);
  a debug server drops to about 10 TPS while a joining player's chunks generate.
- A measured narrow optimization hack documents its bottleneck, boundary,
  correctness fence, effect and removal path. Never restore operator
  worker-thread percentages; derive capacity once and let measured autoscaling
  move bounded budgets.

## Git, local state and sibling repositories

- Main commits often and pushes `main` (owner, 2026-11-07): small Conventional
  Commits per integrated slice, not one giant commit every two days; a commit
  that is not yet green may land on `main`. Main also pushes the sibling
  repositories' `main` after each merged slice (owner, 2026-11-15); a broken
  push gets a follow-up fix. Workers commit only on their own
  worktree branch, never on `main` and never pushed; Main merges or applies
  their work. Tags, releases and publication still need the
  owner's explicit authorization, and every release tag first gets two read-only
  release reviewers ([release reviewers](docs/development/review-committee-and-breaker.md#release-reviewers-owner-2026-11-07)).
  Preserve hooks and signing. Commit `Cargo.lock` only for a concrete dependency
  reason.
- Preserve unrelated dirty files: never reset, clean or stage them. Inspect
  unique changes before removing an inactive worktree; discard only reproducible
  build and client artifacts. Before a reboot stop processes and agents and
  write a handoff (on the owner host into the ignored `.analysis/MEMORY.md`,
  elsewhere in your result to the owner; contents:
  [running agents](docs/development/agent-fleet.md#handoff-across-context-loss-or-to-another-person)),
  then revalidate after restart.
- Worker results record the base tree, diff SHA-256, changed files, validation
  and one next action; use path-limited status and diffs. Never stage
  `.analysis/`, `data/vanilla/`, `.serena/`, `.opencode/`, `x-ui-pro/`,
  `YOLO_MODE.md`, `log.log`, `opencode.json` overlays or other ignored agent or
  tool artifacts unless asked.
- This repo owns the core Rust runtime and the canonical harness.
  `../solaris-loader` owns Java/client scenarios (`KRASNOKAMEN_LOADER_ROOT` may
  override); never duplicate a client-mod source tree here.
  `../solaris-default-plugins` owns the first-party Wasmtime packages
  (`krasnokamen:plugin@0.9.0`), deployed via `[plugins].directory`; there is no
  compiled-in package list or `plugins.bundled` path. `../solaris-client` owns
  the native client and launcher, including singleplayer. Core production builds
  need no sibling; integration gates need the sibling they exercise. Each sibling
  owns its README, AGENTS and Git metadata; creating one locally never
  authorizes staging, commits, remotes, pushes or publication.
- *Owner host:* worktrees are created with `tools/dev/wt-new` under `<work>/wt/`
  and ended with `tools/dev/wt-done`; gate binaries come from
  `tools/dev/pin-release`. Heavy outputs go under
  `<work>/targets/`, nothing new on the root disk. Keep at least 250 GiB free on
  the games disk. Free reproducible build caches first; preserve run data,
  including crash copies and WAL, unless the owner explicitly permits deletion
  ([disk rules](docs/development/heavy-commands-and-memory.md#disk)). Use
  absolute paths in worktree commands (a relative path once corrupted the main
  checkout); never assume the cwd.
