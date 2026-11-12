# Solaris docs archive

Superseded designs, old release-era notes, finished research and one-off prompts/articles
that no longer describe the current system. Each entry: original path, date/era,
what it said, why superseded, which doc replaced it. Full text lives in Git history.

Current runtime: `solaris-v2` (`mc-edge` + `mc-core`, native tick/intent-merge/WAL)
per [ADR 0013](../decisions/0013-core-model-v2.md) and [ADR 0014](../decisions/0014-v1-core-retirement.md).

## docs/articles/show-hn-solaris.md (2026-10, pre-cutover draft)

Said: Show-HN draft claiming the v1 regional-owner architecture ticks 100k mobs
in 50 ms (p50 41.9/p95 48.9 ms, release build, 50 headless protocol clients);
title options, post body, numbers, first-comment technical details, FAQ.
Superseded: v1 region ownership deleted in ADR 0014 Phase 5; headline is not a
v2 performance claim and current docs must not cite it as one.
Replaced by: `docs/performance/PROFILES.md` (measured v2 profiles, 2026-11-13)
and `docs/ARCHITECTURE.md`.

## docs/articles/region-owned-simulation.md (2026, v1 era)

Said: publication draft of the retired v1 single-writer-region model (8x8-chunk
regions, one lease/lane owner, EntityStore per lane, prepare/commit/finalize
cross-region transactions, 100k-entity benchmark pointer).
Superseded: same as above; region leases/owners no longer exist.
Replaced by: `docs/ARCHITECTURE.md`, ADR 0013.

## docs/architecture/pilot-warehouse-binding.md (2026-09-26, base 9889036, CP-REFACTOR-030)

Said: executed-slice record of a pure-move extraction (`settlement.rs` 4040
lines -> `warehouse.rs` 526 lines), visibility decisions, native test command
(`cargo test -p mc-net ...`), scenario catalog names.
Superseded: self-marked historical; v1 `crates/mc-net/...` paths and commands
deleted in Phase 5, not native-runtime navigation.
Replaced by: `docs/architecture/mutations.md` (ownership) and ADR 0013.
Note: `docs/architecture/mutations.md` still links the filename; link is being
removed with this archive move.

## docs/performance/CORE_PROFILE_MATRIX.md (v1 campaign, frozen through 2026-10-15)

Said: v1 M91/T07 performance contract — `mc_net::AutoscalePolicy::for_profile`
defaults, balanced/low/high/VD32 envelopes, 20-client VD8/VD32 and scale
receipts, `tools/profile-entity-scale.sh`-era commands, seed-42 dry-anchor
fixtures (rev 24/25), owner `sarvar` numbers.
Superseded: ADR 0014 Phase 5 deleted the profiled runtime/commands; autoscale
TOML and regional-owner defaults are not v2 settings. Historical results do not
transfer to the native runtime.
Replaced by: `docs/performance/PROFILES.md` + `catalog-load` contract in
`docs/AGENT_TOOLING.md`.

## docs/performance/ENTITY_SCALE_PROFILE.md (v1 benchmark)

Said: 40k hostile entities / 60 headless-TCP-client benchmark (16 regions 4x4
grid, 2500 entities each, 200 warm-up + 1200 measured ticks, VD2), run script
and report fields.
Superseded: runner and regional-owner lane model deleted in Phase 5.
Replaced by: `docs/performance/PROFILES.md` (`native-100` / `villager-heavy` rows).

## docs/research/ASTRA_RND_2026-10-22.md (2026-10-22, parked ideas)

Said: committee-astra read-only review. Part 1: four ranked near-term
experiments (Exp-CkptSections section-granular checkpoint capture;
Exp-VillagerBox compact journal baselines; Exp-SleepShadow event-driven
sleeping set; Exp-DeltaFanout delta-driven chunk fanout), all [INFERENCE]
until measured, with reject-list (Folia/multi-node, ECS/GPU rewrite, THP,
binary-WAL-first, Minestom comparisons). Part 2: seven long-horizon
"revolution" directions (semantic compiler, multiscale world, temporal DB,
multi-node, heterogeneous engine, semantic protocol, learned accelerators).
Status in-file: parked, not a plan; owner keeps for after the queue.
Superseded-as-current-doc: nothing in it is measured evidence or an approved
task; keeping a 35KB parked note in `docs/` implied currency it disclaims.
Replaced by: nothing yet — experiments go to Main when queued; long-horizon
list stays here only.

## docs/releases/v0.0.1-alpha.1.md (public alpha, pre-v2)

Said: first public alpha for 26.1.2 — login/chunks/movement/inventory/crafting/
containers/persistence/respawn/combat/mobs/trading/gossip; server-owned **Lua**
plugins; Fabric/NeoForge extension path; Linux x86_64+AArch64 archives;
alpha limits; `install.sh` + `solaris --check` install flow.
Superseded: Lua frontend retired (current: Wasmtime `solaris:plugin@0.8.0`);
`--check`/`solaris` binary names retired (current: `solaris-v2`,
`content verify`).
Replaced by: `docs/releases/v0.0.8.md` + `docs/PLUGINS.md` + `docs/OPERATING.md`.

## docs/releases/v0.0.2-alpha.1.md (alpha 2, v1 era)

Said: broader survival coverage, regional-owner natural spawning ("periodic
regional runtime"), server-owned **Luau** plugin API foundation, `mc-net`
chunk-stream/spawn semantics, deterministic `mc-net` test hardening.
Superseded: regional runtime, Luau frontend and `mc-net` topology deleted in
Phase 5.
Replaced by: `docs/releases/v0.0.8.md`, ADRs 0013/0014.

## docs/releases/v0.0.5.md (preliminary, draft)

Said: movement/damage/respawn fixes, animals/aquatic populations, mining/memory,
hosting/logging/profiling changelog; upgrade notes; validation boundary.
Superseded: v1-era receipt; superseded by v0.0.8 core replacement.
Replaced by: `docs/releases/v0.0.8.md`.

## docs/releases/v0.0.6.md (preliminary, draft)

Said: light/fluid fixes, worldgen foundations (generation-only), test-repair
sweep, source simplification/plugin authoring; villages explicitly deferred;
test counts citing `mc-entity`/`mc-server`/`mc-net` suites.
Superseded: v1 crate layout deleted; villages since landed differently.
Replaced by: `docs/releases/v0.0.8.md`.

## docs/releases/v0.0.7.md (preliminary, draft, last v1 release)

Said: mob interaction, containers, first-startup content fixes; graphical
real-client acceptance not recorded.
Superseded: final v1 release before the 0013/0014 core replacement.
Replaced by: `docs/releases/v0.0.8.md`.

## docs/VALIDATION_LEDGER.md historical slices (2026-06-13 .. 2026-07-11, v1)

Archived 2026-10-24: 91 per-slice sections (lines ~43-1441, ~290KB) plus 2
Oct-11 pre-gate slices (fixture surface selection; W1 projectile/respawn roots),
summarized below. Kept in place: header/runtime-era notice, status vocabulary,
M77 preflight, `## Frozen M100 Denominator`, non-goals appendix, M78-M100
targets, 80% blockers, M95 audit (tooling input to `coverage-audit`), and the
2026-10-11 Parity Wave 1 integrated native gate (latest L2 leg).
All archived slices carry the ledger's own runtime-era warning: v1 trees,
deleted `mc-server`/`mc-net` commands and quality labels apply to their
recorded trees, not the current server; no row auto-revalidates on cutover.
Full text in Git history (`git log -- docs/VALIDATION_LEDGER.md`).

- Post-M95 local automated validation update: generated gitignored
  `block_light.json`, reran fmt/clippy/test gates; stabilization label, no
  real-client run, no row promoted to `ready`.
- 2026-06-13 static review: static-only, no cargo/client/profiler/soak; froze
  46-row denominator at 0.00% conservative under the oracle/client rule.
- 2026-06-14 stabilization gate: added `xtask code-health` tripwire; focused
  chunk-stream (mc-net 15), storage/Anvil (mc-world 7), item persistence,
  offline-admission suites; workspace 944 passed; no oracle/client/perf.
- 2026-06-22 probes (baked-light/warm-cache; disk-backed startup; existing-world
  missing light; slow-reader pressure; dirty-flush lock narrowing): 289-chunk
  VD8 stream down to ~327 ms warmed, `light_compute_ms=0`, rebake persisted,
  startup dirty-checkpoint path; synthetic harness only.
- Chest/quick-craft slices (left-drag, right-drag, shared live-update,
  stale-click races, furnace races, malformed/invalid-click fail-closed,
  world-border/out-of-reach resyncs): owner-ordered CAS semantics, protocol
  harness coverage; no client/visual proof.
- Script/plugin manifest slices (command capability, plugin contract, migration
  metadata, client-bridge contract, in-flight rescan caps, queue fairness):
  bounded manifest validation; closed Solaris-native event set, not
  Bukkit/Paper matrix.
- 2026-06-22 real-client agent runs (join/rejoin/chunks/movement, random-tick
  perf, inventory/chest, block/fluid, signs/beds/campfires,
  entities/combat/death/respawn, M40/M41 routes, save/restart, shared chest,
  screenshot/scenario/schema/quality validations, blocked-only + loopback +
  exact-gate slices): prepared-manifest agent runs on the v1 tree, retained as
  provenance; superseded-findings separation established, not v2 acceptance.
- 2026-06-23/07-04/07-05 slices (shared chest/furnace live-update + stale-click
  protocol races; campfire in-flight disk/restart/zero-client/timer/lit-state;
  generated-world chunk-stream lock pressure; operator `--check` warning
  surface for bind/auth/world-dir/sidecar/registry/light/tags/recipes/loot;
  login compression boundary + malformed rejection; falling-block
  landing/occupied-landing; play admission disconnect; save-all stop path):
  v1 `mc-server --check` CLI and `mc-net`/`mc-server` test IDs; commands
  deleted in Phase 4, cited here as receipts only.
- 2026-07-04/05 real-client runner validation slices (screenshots, scenario IDs,
  PNG checks, driver PNG checks, blocked-only scenarios, loopback, exact gate,
  observations schema, quality label): runner-contract scaffolding for
  `tools/run-real-client-regression.sh`; `--prepare` only, no full `--check`.
- 2026-07-10/11 replay + prompt slices (tick percentile windows, core replay
  schema, manifest-driven Solaris/vanilla-oracle/real-client replays, 20-client
  metrics workload, Prompt-02 transaction/precondition/container/item/session/
  cursor/death/restart checks, Prompt-03 single-writer entity boundary): v1
  `SimulationOwner`/regional-owner evidence and replay schema; authority model
  superseded by ADR 0013 intent-merge.
- 2026-10-11 generated survival fixture surface selection: raw-TCP fixture
  picked a cave floor; surface-selection follow-up (kept in archive, not in
  place — superseded by the Oct-11 integrated gate row it fed).
- 2026-10-11 W1 projectile/respawn failure roots: pre-gate failure analysis
  feeding the kept Parity Wave 1 gate section.

## docs/agents/PRO_CORE_AUDIT_PROMPT.md + PRO_CORE_AUDIT_RUNBOOK.md (one-off audit tooling)

Kept in place (referenced by `tools/prepare-pro-core-audit.sh`, `Justfile`
`pro-audit-context`, `docs/README.md`). Note: audit prompt names
`mc-simulation` (deleted crate) in dimension 2 and model names
(`Pi GPT-5.6 Sol/Luna`, `spawn_pi_detached`) that postdate the current
harness vocabulary; treat the prompt as a frozen one-off instrument, not a
current crate/topology reference. If the audit is rerun, refresh it first.

## docs/settlements/OWNER_DECISIONS.md (CP-058/059/068 ruleset, 2026-10)

Kept in place (owner's verbatim war/supply/mount/charge numbers; no code or doc
consumer cites it yet — flagged, not archived: deleting owner ruleset values
would destroy the only copy of the decision). Cross-check note: §CP068-R2
points at `../solaris-default-plugins/docs/settlements/OWNER_DECISIONS.md`
(a sibling repo outside this checkout); verify there on use.
