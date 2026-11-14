# Catalog load (`catalog-load`)

Open-loop protocol loads against the ordinary **v2 release server**, driven by a
separate `player-scale-bots` process. The profile builds no binaries, runs no
kernel-only benchmark and impersonates no rendered client. It is the current
public-protocol workload family; the retired `entity-scale` and
`living-world-scale` results are history, and debug-build numbers are never
optimized performance evidence.

`python3 -m tools.harness list` shows the profile; `python3 -m tools.harness
explain` lists the scenario catalog. Resource admission and exclusive windows:
[heavy commands](heavy-commands-and-memory.md) and
[performance windows](performance-windows-and-profiling.md).

## Where the scenarios are defined

- `tools/harness/catalog_load_scenarios.json` maps each stable scenario `id` to
  its fixture and behaviours, wall-time windows, required work,
  oracle/invariants, capability blockers and threshold provenance.
- `tools/harness/catalog_controls.py` holds the control recipes;
  `tools/harness/catalog_mixes.py` the MIX recipes and validators.
- "Runnable" means implemented, not an executed PASS.

## Run one scenario

```sh
python3 -m tools.harness run catalog-load -- \
  --scenario JOIN-01 \
  --server-bin /absolute/prebuilt/krasnokamen \
  --bots-bin /absolute/prebuilt/player-scale-bots \
  --clients 10 --smoke --out /absolute/new/catalog-load-smoke
```

Use a new output directory; repeat `--scenario` for explicit selections. The
binaries are explicit and prebuilt: no implicit builds.

Defaults: 10 clients, seed 42, view distance 4, readiness deadline 30 s and
**20/10/20 wall seconds** for steady/burst/recovery (`--steady-seconds`,
`--burst-seconds`, `--recovery-seconds`, `--readiness-seconds`,
`--view-distance`; `--ramp-clients 1 10 25 50` selects strictly increasing
client rungs). `--worldgen-gpu` sets the server's `[server].worldgen_gpu`; the
receipt records it.

## What a run measures

- Gameplay starts at a shared ready barrier and a monotonic epoch; the
  steady/burst/recovery boundaries use the bot's shared schedule epoch, and tick
  selection keeps only ticks starting in `[start, end)`.
- Recovery offers zero gameplay: connected readers plus mandatory protocol
  liveness only, never repair commands.
- Measured demand never waits on responses. Menu and button bursts use
  predeclared fixed leads and spacings; missing authoritative pose or reach
  rejects the offer without retry, refill or extended windows. Reports after the
  wall schedule ends acquire no new work credit; an interrupted rung keeps its
  precise failure classification.
- Receipts separate offered, transport-admitted, skipped/dropped, actually
  observed server admission/completion, and client-observed work: writing a
  packet is not completion.
- Outcomes: `UNSUPPORTED_MECHANIC`, `INFRA_BLOCKED`, `SETUP_FAILED`,
  `CORRECTNESS_FAIL`, `PERF_FAIL`, `PASS`. Absent required oracle or telemetry
  cannot pass.
- Explicit operator defaults (not catalog SLOs): sustained TPS >= 19.5, tick
  p99 <= 50 ms, maximum tick <= 250 ms, RSS <= 3072 MiB, no correctness
  failures. Overrides: `--min-tps`, `--sustained-seconds`, `--max-tick-p99-ms`,
  `--max-tick-max-ms`, `--max-rss-mb`; effective values are receipted.
- Smoke windows total at most 120 seconds and are non-performance evidence:
  even `PASS` with `smoke_only: true` is not a capacity or tail-latency result.

## JOIN-01 and `join-ready-assisted`

The `JOIN-01` public-command path uses one declared operator setup client to
create a bounded safe platform (65-by-65, five-column grass sample verified at
feet y=64 against actual section-palette cells; advertised heightmaps are not the
oracle), set spawn and gamerules, then issue `save-all flush`. The received
`Saved the game` fence precedes clean stop, hash and restart of that same
isolated world. Exact `JOIN-01` measured clients have no operator permissions
and no creative/item grants. `join-ready-assisted` retains the older bounded
preparation, creative landing and dirt grant; its `PASS` cannot close the exact
catalog case.

## Control and compound scopes

`python3 -m tools.harness list` plus the JSON mapping state current capability.

| ID | Scope |
|---|---|
| `dense-hostile` | 1024 authored hostile UUIDs, real survival/damage/respawn, native-owner proof |
| `spread-passive` | Four 15-by-15-chunk arenas, 256 passives each, wandering plus 1024-UUID native ownership |
| `explore` | Assisted walking, decoded chunks, server snapshot-derived `SetCenterChunk` transitions (diagnostic; not exact CHUNK-01 closure) |
| `built-base` | 64 furnaces, 128 hoppers, 64 loose stacks, 512 physical buttons; real transforms, atomic native stock |
| `fluids` | 128 naturally clocked capped sources; finite plug releases, fresh topology/level cells |
| `village` | 64 actual farmers, legal native Brain, unique declared bed/job claims (not a day-cycle oracle) |
| `combat` | 128 hostiles, 96 finite primed TNT plus 96 chain targets; real blasts, health, removal |
| `projectiles` | 128 skeletons plus 128 finite initial arrows; continued UUID/shooter/victim/damage/removal proof |
| `farms` | All 11440 natural crops and substrates; legal fresh ages, positive measured growth |
| `plugin-heavy` | Current-WIT towns + economy `--plugins-dir`, at least two bots; concurrent town/claim and exact balance queries, idempotent payment, ordinary chest custody, per-package journal commits |
| `checkpoint-restart` | Full mixed base/128 passives/64 villagers/11440 crops, actual save ACK, same-world native restart |
| `long-session` | Finite full base/256 passives, minimum 600 s non-smoke windows, independent resource/queue/work/native-stock proof |
| `MIX-01` … `MIX-21`, `mixed-survival-long` | Separate MIX recipes/validators (tiny for `--smoke`, catalog scale otherwise); a control never silently replaces a compound crisis |

`plugin-heavy` requires `--plugins-dir /absolute/current-WIT/deployment` with
real `krasnokamen-towns` and `krasnokamen-economy` packages; only those packages are
copied intact into the isolated preparation directory. Keep `RUST_LOG=info` (or
at least `plugin.runtime_ready=info,plugin.command_result=info`): missing state
readback, host evidence or ordinary tick/resource telemetry fails closed.

## `spain-industrial`: assisted chain diagnostic

`--scenario spain-industrial --spain-fixture /absolute/fixture.json` runs an
assisted chain diagnostic in one live world, admitting bot cohorts of 2, then 2,
then 4. The last cohort attempts two concurrent actor chains with distinct
pursuit targets and independently attributed collectors.

- The fixture identifies the staged world, unverified waypoint inputs,
  source/destination containers and two observed villagers, a certified
  building, a workstation and the matching embedded item registry. Provisioned
  stock and workstation are not earned extraction.
- Each completed cycle requires independent received arrival, conserved
  chest/player images, matching item pickup, and native carry and production
  receipts. Visible villager carry baselines are recorded before the drop; the
  actual item collector must have one, show the exact carry increase and perform
  the production job. A pursuit target's last received pose is not fresh
  authoritative state; actor arrival, actual pickup and carry remain separate
  evidence.
- Completed custody and subsequent job/carry stage evidence is retained in
  `bread-<stage>-<actor>-evidence.json` even when production validation fails.
- After all four chains complete, the diagnostic records each collector's native
  carry, cleanly restarts the same world, then requires identical completed-job
  identities/receipts and collector carry slots/revision/hash across the restart.
- `ASSISTED_CHAIN_ONLY` is deliberately not a Spain-industrial or capacity
  PASS; runs emit no performance claim. The private-world inotify guard aborts
  below 60 GiB on observed world writes; unrelated writers can consume disk
  between those events.

## Related tools

- `python3 -m tools.coreperf.capacity_ladder`: the fresh-world player-capacity
  ladder, see [BENCHMARKS](../performance/BENCHMARKS.md#player-capacity-ladder).
- `python3 -m tools.coreperf.compare` (`--mobs`, `--mode abab|v2-only`) is a
  separate fixture comparison, not catalog closure evidence.
- The Constantinople city fixture is [its own page](constantinople-fixture.md).
