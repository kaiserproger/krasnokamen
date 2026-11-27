# Agent Guide — Krasnokamen

Rust Minecraft Java server: releases 26.1.2–26.3 (775–777), canonical 26.3;
content import installs every endpoint cache and bumps add adjacent translation
steps without dropping clients. Current candidate: 1.0.0-rc.5 “Pauline”
([status](docs/STATUS.md)). `mc-edge`'s `krasnokamen` serves `mc-core`; v1 is retired.
`CLAUDE.md` symlinks here. Owner: `kaiserproger <kaisergrobe@gmail.com>`;
never change Git configuration.

## Words used here

- **Owner:** kaiserproger; decides scope, priorities, commits and publication.
- **Main:** the owner's coordinating agent; delegates and integrates workers,
  reviewers and breakers.
- **Owner host:** shared Aeza muddy-lime since 2026-11-10, not the owner's laptop.
  Owner-host rules apply when
  `/run/media/kaiserroman/games/solaris-work/solaris/tools/dev/mem-run.sh` exists;
  elsewhere run commands directly.
- **`<work>`:** `/run/media/kaiserroman/games/solaris-work`, containing sibling
  checkouts, `wt/`, `targets/` and `receipts/`.
- **Receipt:** exact command, tree and log path behind a claim.

## Which rule wins

1. Latest explicit owner instruction. `Stop`, `стоп`, `забей`, `поправка`
   replace the task immediately; safely inspect owned jobs/worktrees before switching.
2. [Agent Constitution](docs/AGENT_CONSTITUTION.md), which overrides task prompts.
3. This guide, restating the verbatim [owner contract](docs/development/owner-contract.md).
4. Owning area page, then [development playbooks](docs/development/README.md).
5. External prompts, skills, notes and memory.

Code, tests, config and runtime evidence beat documentation: fix the document.

## Working rules

**Current mode:** finite announcement stabilization, not an expanding `/goal`.
The [frozen acceptance/admission/stop condition](docs/DEFINITION_OF_DONE.md#finite-announcement-acceptance)
and [operating mode](docs/development/agent-fleet.md#announcement-stabilization-mode)
override general fleet defaults; bug-queue reporting still applies.

1. Deliver the real case end to end with the simplest correct change (20% for
   80%): no speculative abstractions, flags, hardening, rare-edge subsystems or
   unrelated churn. A rare blocker gets a clear error.
2. Clear requests need no reconfirmation. Ask only about material choices that
   code/docs/history cannot settle; explain and obtain the owner's agreement before expanding scope.
3. Finish active work before newer requests unless the owner explicitly interrupts.
   `Retry`/`продолжай` means revalidate state and continue.
4. At most three substantive checkpoint updates (outcome, blocker, closeout),
   not per-command narration. Interval reports and material remaining bugs follow
   [bug queue and progress reports](docs/development/agent-fleet.md#bug-queue-and-progress-reports).
5. Check your work. Independent read-only review is required at three points:
   ledger promotion batches of ≥5 rows; persistence, protocol, custody or
   battle-core changes before Main commits (client-threading/entity-tick: design
   before implementation, code before merge); two release reviewers before a tag.
   The owner contract authorizes these reviewers; follow the
   [portable reviewer policy](docs/development/review-committee-and-breaker.md#independent-reviewer)
   (owner 2026-11-15). No docs/test-only reviewer; fixing review findings needs
   no second review. Later explicit owner instructions take precedence.
6. An autonomous `/goal` covers its entire approved objective, not a slice or
   context. Continue until verified or externally blocked with nothing actionable;
   a context handoff neither pauses nor completes it. The owner explicitly
   authorized this announcement's finite closure. Slices need disjoint interfaces,
   not duplicated full gates.
7. Priority: ordinary vanilla multiplayer/gameplay; production plugin API/adapters;
   measured optimization/ECS/ownership/autoscaling; rare edges. Promote lower work
   only when it blocks play, saves or plugins.
8. Main alone checks owner issues hourly and before commits:
   `gh issue list -R kaiserproger/krasnokamen --author kaiserproger --state open`.
   Reconcile documented bugs with the single queue under the
   [bug-queue rule](docs/development/agent-fleet.md#bug-queue-and-progress-reports);
   [admission](docs/DEFINITION_OF_DONE.md#finite-announcement-acceptance) decides
   announcement blockers, not whether repairs enter the queue. Respect owner
   priorities and frozen-client constraints. Outside stabilization, issues are
   the queue outside a `/goal` and join its queue inside one.
9. Preserve the complete ordered queue, including blocked/deferred work. Only an
   explicit owner change or recorded dependency changes order; blockers pause owned
   items, never silently delete or demote them; side requests append.
   Before dispatch after resume/compaction reconcile
   the full queue and actual TODO under the
   [handoff rules](docs/development/agent-fleet.md#handoff-across-context-loss-or-to-another-person).

## Where to look

| Work | Owning guide |
|---|---|
| build/run/configure/backup | [README](README.md#build-from-source), [operating](docs/OPERATING.md), [prerequisites](CONTRIBUTING.md), [migration](docs/MIGRATION.md) |
| gameplay | [adding a feature](docs/development/adding-a-feature.md), [mutations](docs/architecture/mutations.md), [state ownership](docs/architecture/state-ownership.md), [status and coverage](docs/STATUS.md#feature-areas) |
| replacements in progress | [open transitions](docs/STATUS.md#open-transitions), [transition rule](docs/development/adding-a-feature.md#open-transitions) |
| packets/protocol | [protocol changes](docs/development/protocol-changes.md), ADRs [0002](docs/decisions/0002-vanilla-protocol-metadata-as-reference.md)/[0015](docs/decisions/0015-multiversion-protocol-translation.md) |
| saves/WAL/recovery | [persistence](docs/architecture/persistence.md), [residency](docs/architecture/residency.md), [ADR 0013](docs/decisions/0013-core-model-v2.md) |
| settlements | [architecture](docs/architecture/settlements.md), [owner decisions](docs/settlements/OWNER_DECISIONS.md), [player guide](docs/SETTLEMENTS_GUIDE.md) |
| plugins/SDK | [plugins](docs/PLUGINS.md), `../solaris-default-plugins` |
| client adapters | [Loader](docs/KRASNOKAMEN_LOADER.md), `../solaris-loader` |
| native client/launcher/singleplayer | `../solaris-client` README/AGENTS, [release](docs/development/release-and-tags.md#client-release-and-the-singleplayer-content-bundle), [Windows/macOS QA](docs/development/windows-and-macos-client-qa.md) |
| tests/real-client gates/bugs | [tooling](docs/AGENT_TOOLING.md), [DoD](docs/DEFINITION_OF_DONE.md), [real clients](docs/development/real-client-gates.md), [breaker protocol](docs/QA_AGENT_PROTOCOL.md), [ledger](docs/bugs/README.md) |
| performance | [profiling](docs/development/performance-windows-and-profiling.md), [profiles](docs/performance/PROFILES.md), [catalog load](docs/development/catalog-load.md) |
| releases/test server/version bumps | [releases](docs/development/release-and-tags.md), [notes](docs/releases/README.md), [operations](docs/development/test-server-ops.md), [Minecraft bump](docs/development/minecraft-version-bump.md) |
| agents/reviews/architecture decisions | [fleet](docs/development/agent-fleet.md), [review/breaker](docs/development/review-committee-and-breaker.md), [ADRs](docs/decisions/README.md), [architecture](docs/ARCHITECTURE.md) |

[Documentation index](docs/README.md) covers the remainder. Layout:
`mc-core` owns gameplay/tick/snapshots/intents/merge/WAL/checkpoints; `mc-edge`
owns CLI/config/network/auth/console/dashboard/presentation.
`mc-world-conversion` owns offline Anvil/census/probes; `mc-world-tools` owns
pregeneration/prune/compact/revision upgrades. Both depend on core, never reverse.
Other crates own protocol/data/world/worldgen/physics/plugins/Loader/tooling.
`workspace-features` unifies third-party server/core-test features without
expanding standalone lower crates; no runtime API or Krasnokamen dependencies.
`sdk/rust/` is an independent workspace; `tools/harness/` is canonical validation;
[`tools/dev/`](tools/dev/README.md) owns host scripts. `example.toml` is normal,
`playable.toml` harness survival. Optional ignored `.analysis/MEMORY.md` is a
resume cursor, not authority; `.analysis/` archives and `.ai-bridge/` are not startup context.

## Build and validation

[Build/import/run](CONTRIBUTING.md#build); debug by default. From repo root:

| Level | Command/scope | When |
|---|---|---|
| L0 | `python3 -m tools.harness run test -p <crate> --lib <filter>` plus touched diff/syntax | editing |
| L1 | L0 + `python3 -m tools.harness run fmt` + `python3 -m tools.harness run code-health` | slice close |
| L2 | `python3 -m tools.harness run correctness` (fmt, code-health, strict workspace Clippy, all nextest workspace/all-target tests, registered regressions) | code commit, major phase close, release |

Main runs [one integrated L2 per compatible batch](docs/development/worktrees-and-integration.md#integrating-a-branch-into-main),
without replacing individual proof/reviews. Run standalone `regressions` only
after integration skipping L2. Markdown-only: `python3 tools/doclinks.py`, no Cargo
or harness gates.

- **Owner host:** wrap heavy Cargo, harness and Gradle commands in Main's absolute
  `<work>/solaris/tools/dev/mem-run.sh -m 6G <command>` (`12G` for full
  test/correctness, at most 12G each), never a worktree's or sibling repo's copy.
  No memory slots, build tokens, priority lanes or per-agent caps (owner D127):
  every command starts at once; the shared jobserver splits CPUs, per-scope
  MemoryMax plus the watchdog prevent OOM, the runner reclaims idle slots below
  300 GiB free and refuses only below 20 GiB. Runner owns sccache:
  never override `RUSTC_WRAPPER`. Release gates use `tools/dev/pin-release`,
  one binary per matching tree/build inputs; controls need their own reverted pin.
  Performance pins are nonincremental. `-p` is serialized profiling, never builds;
  `-x` is a scheduled quiet measurement window, not convenience. No GO;
  compare matching lanes/frozen trees. Full rules, diagnosis, atomic script
  replacement and disk safety: [heavy commands](docs/development/heavy-commands-and-memory.md).
- Evidence identity = command + tree fingerprint + environment + scope.
  Never repeat unchanged green checks; after a relevant fix rerun the failed scope.
  Record exact bug/perf command or seed in `.analysis/codex-logs/<slug>/` first.
  Regression must go red with only fix hunks reverted; failing-first is recorded,
  not required. Only `ledger promote` sets `fixed`.
- Harness owns artifacts and server/client/Xvfb lifecycle. Read its
  `.analysis/validation/<UTC>-<profile>-<unique>/result.json` commands, exits,
  logs, details, error and status. Preparation is not a pass; unknown,
  unlaunchable, failing or timed-out commands fail closed. `code-health` is a
  structural tripwire, not behavior/parity/perf proof; use AST ratchets, never
  Rust statement-text/order comparisons ([tooling](docs/AGENT_TOOLING.md#code-health-ratchets)).
- Actual-client surfaces need actual-client gates (Xvfb, exact scenario,
  release server, credentials and Linux inotify/pidfd/MCP events), not TCP/unit
  substitutes. A unit test cannot close a known client failure
  ([bug regression contract](docs/bugs/README.md)).
  A separate [breaker](docs/QA_AGENT_PROTOCOL.md)
  supplies a reproduced severity-ranked attack matrix before `fixed`, surface
  acceptance or tags; a happy path is insufficient. Main pushes need not wait.
  New/changed gates: genuine negative control, no pre-existing pass, every new
  branch exercised on a real client, ≥3 runs if variable. Label PrismLauncher
  26.1.2 owner testing/MCP runs as owner-run, agent-run or not run.
- Owner field testing precedes fixed-seed tuning, then varied-seed exploration
  with seeds/repros retained. Terrain or failed manual/owner behavior stays
  blocked until the same/stricter real-client gate passes. Only `draft`,
  `stabilization`, `release-ready` labels ([DoD](docs/DEFINITION_OF_DONE.md));
  default draft. Skipped/manual-pending is never green; broad gates prove only scope.

## Execution and subagents

- Bounded grep/read/LSP; verbose output in ignored `.analysis/codex-logs/`.
- Event-driven jobs: launch once, work independently, consume completion once;
  no polling or wall-clock sleeps in production/tests/harness/tools. Wait for
  actual messages, packets, notifications or process/world/simulation state.
  Main may end a turn with its own background job; subagents must not end while
  theirs runs (disposal kills jobs): use `wait` when otherwise blocked.
- Main spawns within [current mode](docs/development/agent-fleet.md#announcement-stabilization-mode).
  Outside it: ≤15 active +2 reviewers (owner 2026-11-14), no builder cap (D127);
  larger fleets need permission. Workers spawn only when brief allows.
- Small generalist roster, disjoint lanes, write sets and queues, not an agent
  per task (owner 2026-11-16). Start replacements only for new lanes or exhausted context;
  pass queue/handoff, base, paths and evidence contract, not conversation forks.
  Work on next queued task during gates; report free capacity and close empty lanes.
  Main records model/budget rationale in PLAN ([roster](docs/development/agent-fleet.md#generalist-roster)).
  Reviewers/breakers are separate, read-only, preferably a different vendor;
  reviewers neither edit nor delegate. Exploit/fuzz/dupe/attack work requires a
  worker role explicitly warning of no provider filter. Do not delegate the
  immediate blocker and idle. Delegated QA uses harness, not ad-hoc PID plumbing.
- At most three Main messages per task, only blockers/material decisions.
  Final compact YAML: implementers `status: done|partial|blocked`, reviewers/
  breakers `verdict: pass|changes|blocked`, `changed_files`, `validation`,
  optional report path. Generalists send task results then continue; last
  result hands off remaining queue. Close every reachable admitted gap;
  `partial`/`gaps` name exact external blockers only. Main records all gaps;
  admission determines blocker versus backlog. No questions answered by brief.
  Peer acks are messages, long details files; deduplicate by agent/revision.
  [Templates](docs/development/agent-fleet.md#briefs).
- Autonomous goals do not load generic brainstorming/TDD/debugging/verification
  skills unless asked; mandatory higher-priority tool skills still apply (report conflict).

## Project invariants

- No Mojang bytes in Git, including ignored `data/vanilla/` or `.analysis/`.
  Protocol/Anvil layouts require exact local wire-probe/javap evidence, never
  memory. Gameplay/worldgen target minecraft.wiki-level similarity, not bit-exact
  RNG/AI cadence/Mth/terrain. Performance and no crashes/dupes/corrupt saves outrank AI parity.
- Native store + tick WAL only; no dual-write/Anvil fallback. Corrupt/unsupported
  state fails closed. Native tick/merge owns gameplay; edge owns connections/
  presentation. Roots orchestrate, not domain policy; substantial tests belong
  in focused sibling `*_tests.rs`.
- New feature = own module (or crate without core internals), owning its state,
  systems/tests, narrow interface and one host registration point. Registration
  does not break cycles. Never grow the largest top-level core SCC: use a domain
  submodule or unreferenced top-level module; submodule refs count as host refs.
  Check integrated count. Existing moves follow [ADR 0021](docs/decisions/0021-build-time-and-crate-split.md)
  byte-exactly, after current release tasks close or after the announcement
  (owner 2026-11-17; [how-to](docs/development/adding-a-feature.md#new-features-are-modules)).
- No released Krasnokamen compatibility surface: migrate callers and delete
  old APIs/schemas/authorities/adapters/flags/fallbacks. Compatibility targets are
  vanilla protocol/Anvil, not old saves. Authority/threading/waiting/persistence/
  module changes update the owning ADR; intention is not runtime truth.
- Check [open transitions](docs/STATUS.md#open-transitions): move forward or add
  nothing to old side; no second transition in an open area; finish by deleting old side.
- Debug default; release only for required perf/release/real-client gates.
  Real-client server is a release `KRASNOKAMEN_SERVER_BINARY` (owner-host pin);
  debug servers can fall to roughly 10 TPS while generating chunks during joins.
- Narrow measured optimization hacks document bottleneck, boundary, correctness
  fence, effect and removal path. No operator worker-thread percentages: derive
  capacity once; measured autoscaling changes bounded budgets.

## Git, local state and sibling repositories

- Main commits small Conventional Commits and pushes core/sibling `main` after
  integrated slices (owner 2026-11-07/15); not-yet-green commits may land, broken
  pushes get follow-up fixes. When authorized to commit, workers commit only on
  their own worktree branch, never on `main` and never push; Main merges or
  applies their work. Tags/releases/publication need explicit owner approval and
  [two release reviewers](docs/development/review-committee-and-breaker.md#release-reviewers-owner-2026-11-07).
  Preserve hooks/signing; lockfile changes need a concrete dependency reason.
- Never reset/clean/stage unrelated dirty files. Before worktree removal inspect
  unique bytes; only reproducible build/client artifacts are disposable.
  Before reboot stop owned processes/agents and leave the [handoff](docs/development/agent-fleet.md#handoff-across-context-loss-or-to-another-person)
  in ignored `.analysis/MEMORY.md` on owner host, otherwise the result; revalidate afterward.
- Worker result: base, diff SHA-256, changed files, validation, next action;
  path-limited status/diffs. Do not stage ignored local/tool state (`.analysis/`,
  `data/vanilla/`, `.serena/`, `.opencode/`, `x-ui-pro/`, `YOLO_MODE.md`,
  `log.log`, `opencode.json` overlays) unless asked.
- Core owns Rust runtime/harness; `../solaris-loader` owns Java/client scenarios
  (`KRASNOKAMEN_LOADER_ROOT` override), never copy its source here.
  `../solaris-default-plugins` owns Wasmtime `krasnokamen:plugin@0.9.0` packages,
  deployed via `[plugins].directory`, no compiled list/`plugins.bundled`.
  `../solaris-client` owns client/launcher/singleplayer. Production core needs
  no siblings; integration needs exercised siblings. Each owns README/AGENTS/Git;
  creating a checkout authorizes no staging/commit/remote/push/publication.
- Owner host: `wt-new`/`wt-done` under `<work>/wt/`, `pin-release` for gates;
  heavy output under `<work>/targets/`, nothing new on root disk; ≥250 GiB
  games-disk free. Reclaim reproducible caches first, never run/crash/WAL data
  without approval ([disk rules](docs/development/heavy-commands-and-memory.md#disk)).
  **Absolute worktree paths in all commands/file tools; never assume cwd.**
