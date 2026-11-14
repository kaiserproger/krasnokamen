# Battle staging and capture

The operator command is `battle setup <seed> <field-100|field-1k|field-10k>
[count]`, followed by `battle start`, `battle pause`, `battle status`, or
`battle reset`. The optional count is per side and must be 1–10,000. Presets
are compiled in; scenario files are not read by the tick. Run setup on the
desired flat battlefield: its centre is the command origin (the world spawn
for console). Formation runtime is in memory; after a restart reset the
persisted battle before staging it again.
`battle start` refuses until all live staged entities occupy loaded,
entity-simulated chunks; the staging ticket covers the bounded battlefield.
Battle setup rejects Peaceful: its hostile-mob policy removes even persistent
husks. Switching to Peaceful during a battle therefore ends its armies; ordinary
distance-based natural despawn is disabled. Successful setup sets world rules
`mob_drops`, `mob_griefing` and `spawn_mobs` to false. These changes persist
after reset; restore them explicitly if the world is reused.

The side mix is 50% sword/shield infantry, 20% spearmen, 20% archers and
10% cavalry (integer rounding goes to infantry). Regiments are homogeneous:
archers deploy behind the foot and mounted troops on the flanks, with two-block
cavalry frontage. The units use vanilla husks and equipment.

`run-battle.sh` creates a new run directory and never overwrites an existing
one. `--world` copies a **stopped** native world, including its spawn and
terrain; without it the server generates a fresh world, not necessarily flat.
Pass `--world-seed <seed>` if the copied world's generation seed differs from
the battle seed.
The server pin comes from `tools/dev/pin-release`; the client must contain the
headless recorder, and the pinned server must contain the recorder/telemetry
slice. Both are required for an actual run. For example:

```sh
tools/battle/run-battle.sh --pin /absolute/server-pin --client /absolute/krasnokamen-client \
  --client-data /absolute/client-data --cache /absolute/vanilla-cache \
  --world /absolute/stopped-flat-world --run-dir /absolute/new-run \
  --seed 42 --scenario field-10k --duration 180
```

Add `--dry-run` to print the planned command list without starting processes or
creating the run directory. The runner verifies the server pin checksum, writes
the effective config, commands, provenance and logs, starts the headless
recorder and waits for Play, pose and a tick marker **before** setup. It then
requests the console-only `battle roster` of soldier and horse UUIDs, publishes
the roster atomically, and waits until every UUID is observed before combat.
The recorder gets a 900-second post-join roster deadline for large setups;
its requested recording duration starts only after that roster is observed.
It streams dashboard telemetry and stops the server after the recorder exits.
This does not establish real-time performance or all-dirty residency.

## Checkpoint observation

`checkpoint-report.py TELEMETRY.jsonl SERVER.log --output REPORT.json` joins the
E1 per-tick telemetry with `MC_CORE_PHASE_LOG=1` checkpoint-capture messages.
It reports tick gaps and capture ticks outside the retained telemetry instead of
inventing samples. The tool does not verify population, movement or dirty
counts; report those from independent observations rather than inferring them.
It also summarizes tick-owner `journal_build_us` and `journal_queue_us` where
present; these are not physical WAL byte counts or background fsync latency.

For the paused-versus-running ladder, add `--paused-ticks 1200
--minimum-battle-ticks 1200 --duration 600` to the runner. The recorder's
duration includes the paused interval; increase it if load slows the server.
The run fails closed if its recorder receipt lacks a continuous marker range or
fewer than the requested running ticks. `phase-boundaries.json` records the
paused sample window, server transition ticks and the last recorded marker.
Pass it to the analyzer as `--boundaries RUN/phase-boundaries.json` to get
sample counts, missing ticks and p50/p95/p99/max for each phase. These are
per-run sampled percentiles, not a population guarantee.

For the 20,000-moving-unit experiment, run the staged battle across at least
two periodic checkpoints with `MC_CORE_PHASE_LOG=1`, retaining the full server
log and exporting telemetry before the 16,384-tick ring wraps. Record the pin,
world, seed, CPU quota, allocator, and `battle status` counts for each side
and resident/active units. The runner requests status about every 100 recorded
ticks; the report pairs each checkpoint with the nearest sample and shows its
signed tick gap. Nonzero velocity is a snapshot,
not proof that every unit changed position since the preceding checkpoint; a
true all-dirty claim requires actual movement or dirty-set evidence. The ADR
marks position writes dirty, so measured moving units contribute dirty rows.
Report capture duration, telemetry gaps and ordinary ticks alongside the
50 ms budget, and describe the observed population without calling an
unmeasured all-dirty set proven.
The operator-triggered status command scans retained entities and decodes their
owner tags; this is not per-soldier per-tick staging work, and its cost is
included in the sampled tick.
`MC_CORE_PHASE_LOG` also emits per-tick phase logs, so these timings include
diagnostic logging overhead and are not an uninstrumented TPS measurement.

ADR 0020 records 50.8 ms at 10k and 279.2 ms at 100k on a historical setup;
its 20k cost is not measured. If all-dirty capture violates the budget, evaluate
immutable paged copy-on-write snapshots (ADR 0020 option A) or a consistent
multi-tick snapshot with a retained mutation log. Either requires a separate
persistence change proving a single snapshot fence, WAL-first ordering, bounded
nonblocking handoff, tombstones, crash recovery and memory bounds. Merely
spreading reads of mutable rows across ticks is not a consistent checkpoint.
The `~100 ms` figure in the simulation research is an extrapolation, not a
measurement or a promise on the battle host.
