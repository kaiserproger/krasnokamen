# Agent Guide — Solaris

Solaris is a Rust Minecraft Java server for 26.3; 26.1.2 clients can still join
through a retained legacy content cache. The current release candidate is
1.0.0-rc.5 "Pauline" ([status](docs/STATUS.md)). The binary is `mc-edge`'s
`solaris-v2` over the native core `mc-core`; v1 is retired. `CLAUDE.md` is a
symlink to this guide. The owner is `kaiserproger <kaisergrobe@gmail.com>`;
never change Git configuration.

## Words used here

- **Owner**: kaiserproger. Decides scope, priorities, commits and publication.
- **Owner host**: the owner's workstation, where many agents share one machine.
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
   protocol or custody before Main commits it; two release reviewers before a
   tag. Docs-only and test-only slices get no reviewer. The owner contract
   authorizes these reviewers; fixing their findings needs no second review.
6. An autonomous `/goal` is the whole objective and queue: continue until it is
   verified or blocked with nothing actionable. A context handoff neither pauses
   nor completes it. Slices need disjoint interfaces, not separate full gates,
   snapshots, reviews, sessions or commits. Finite mode needs explicit permission.
7. Priorities: ordinary vanilla-client gameplay and multiplayer; the production
   plugin API and adapters; measured optimization, ECS, ownership and
   autoscaling; rare edges. A lower item moves up only when it blocks play,
   saves or plugins.
8. Main checks the owner's open issues hourly and before a commit (workers do
   not): `gh issue list -R kaiserproger/solaris --author kaiserproger --state open`.
   Outside a `/goal` they are the work queue; inside one, add new issues to the
   goal's queue.

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
| client adapters | [Solaris Loader](docs/SOLARIS_LOADER.md) | `../solaris-loader` |
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
auth, console, dashboard and connection presentation; other `crates/` protocol,
data, world, worldgen, physics, plugins, Loader and tooling. `sdk/rust/` is an
independent guest SDK workspace; `tools/harness/` is canonical validation;
`tools/dev/` holds [owner-host scripts](tools/dev/README.md). `example.toml` is
the normal config; `playable.toml` the harness survival config.

On the owner host an ignored `.analysis/MEMORY.md` may hold a resume cursor. It
is optional, absent in a fresh checkout, and loses to code and tracked docs;
`.analysis/` archives and `.ai-bridge/` are not startup context.

## Build and validation

From the repository root (debug by default):

```sh
cargo build -p mc-edge --bin solaris-v2
cargo run -p mc-edge --bin solaris-v2 -- content import --version 26.3 --download
cargo run -p mc-edge --bin solaris-v2 -- --config example.toml
cargo build --release --locked -p mc-edge --bin solaris-v2   # real-client gates, deploys, releases
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
  absolute path: `<work>/solaris/tools/dev/mem-run.sh -m 6G env RUSTC_WRAPPER= <command>`
  (`-m 12G` for full `test`/`correctness`; at most 12G each), never a worktree's
  or sibling repo's copy. There are no slots or queue: every command starts at
  once; the heavy-watchdog pauses plain cargo builds under real memory pressure
  and resumes them, never tests, servers or harness runs. `RUSTC_WRAPPER=` keeps
  the global sccache from compiling outside the cap. `-p` is serialized
  timing/profiling, never builds; `-x` is a quiet measurement window (pauses
  builds, waits for running tests and servers). Both lanes serialize
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
  completion once. Never poll process, status or agent lists. Never add
  wall-clock sleeps in production, tests, harnesses or tools: wait for the exact
  message, packet, notification or process/world/simulation state (push, not pull).
- Main has the owner's standing authorization to spawn subagents within the
  fleet limits: at most 10 live agents and 3 concurrent builders, no child agent
  for a part under 30 minutes. Workers spawn only when their brief allows it.
  Larger fleets need explicit owner permission. Give disjoint responsibilities
  and write sets, pass a bounded task, base commit, paths and evidence contract
  (no conversation forks), never delegate the immediate blocker and then idle,
  and close finished agents. Exploit, fuzz, dupe or attack work goes only to
  workers whose role description warns of no provider filter for it.
- Delegated validation and QA use the harness lifecycle and receipts, not
  hand-rolled tmux/PID/log plumbing. Reviewers neither edit nor delegate.
- Each agent sends Main at most three messages (a blocker or a decision only
  Main or the owner can make). Its LAST message is one compact YAML result:
  implementers `status: done|partial|blocked`, reviewers and breakers
  `verdict: pass|changes|blocked`, plus `changed_files`, `validation` and
  optional `report_path`. A gap is work, not an ending (owner, 2026-11-07):
  close every gap within reach before returning; `partial` or `gaps` list only
  what needs something outside the agent's reach (owner action, hardware,
  credentials), each with its exact blocker, and Main queues every reported gap
  for a fix. Ask nothing the brief answers. Acks to peers go through
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
- Solaris has no released compatibility surface of its own: migrate callers and
  delete superseded APIs, schemas, authorities, adapters, flags and fallbacks.
  Compatibility targets are the vanilla protocol and Anvil, not old Solaris
  storage. Changes to architecture, authority, threading, waiting, persistence
  order or module policy update the owning ADR; a desired migration is not runtime truth.
- Check an area's row in [open transitions](docs/STATUS.md#open-transitions)
  before changing it: move it forward or add nothing to its old side; start no
  new transition in an area with one open; finishing one deletes the old side.
- Debug builds by default; `--release` only for a required performance, release
  or real-client gate. Real-client gates run a release server (owner,
  2026-10-27): pass the `--release --locked` binary as `SOLARIS_SERVER_BINARY`;
  a debug server drops to about 10 TPS while a joining player's chunks generate.
- A measured narrow optimization hack documents its bottleneck, boundary,
  correctness fence, effect and removal path. Never restore operator
  worker-thread percentages; derive capacity once and let measured autoscaling
  move bounded budgets.

## Git, local state and sibling repositories

- Main commits often and pushes `main` (owner, 2026-11-07): small Conventional
  Commits per integrated slice, not one giant commit every two days; a commit
  that is not yet green may land on `main`. Workers commit only on their own
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
  `../solaris-loader` owns Java/client scenarios (`SOLARIS_LOADER_ROOT` may
  override); never duplicate a client-mod source tree here.
  `../solaris-default-plugins` owns the first-party Wasmtime packages
  (`solaris:plugin@0.8.0`), deployed via `[plugins].directory`; there is no
  compiled-in package list or `plugins.bundled` path. `../solaris-client` owns
  the native client and launcher, including singleplayer. Core production builds
  need no sibling; integration gates need the sibling they exercise. Each sibling
  owns its README, AGENTS and Git metadata; creating one locally never
  authorizes staging, commits, remotes, pushes or publication.
- *Owner host:* worktrees are created with `tools/dev/wt-new` under `<work>/wt/`
  and ended with `tools/dev/wt-done`; gate binaries come from
  `tools/dev/pin-release`. Heavy outputs go under
  `<work>/targets/`, nothing new on the root disk. Keep 80-100 GB free on the
  games disk: a finished run keeps only its reports, logs and `result.json`
  ([disk rules](docs/development/heavy-commands-and-memory.md#disk)). Use
  absolute paths in worktree commands (a relative path once corrupted the main
  checkout); never assume the cwd.
