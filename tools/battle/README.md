# Battle staging and capture

The operator command is `battle setup <seed> <field-100|field-1k|field-10k>
[count]`, followed by `battle start`, `battle pause`, `battle status`, or
`battle reset`. The optional count is per side and must be 1–10,000. Presets
are compiled in; scenario files are not read by the tick. Run setup on the
chosen battlefield, flat or natural relief: its centre is the command origin
(the world spawn for console). Formation runtime is in memory; after a restart
reset the persisted battle before staging it again.
Every foot soldier and horse stands on the collision top of its own footprint
as soon as its chunk is resident (before its first physics tick); a slot on a
trunk, a wall or water moves to the nearest dry, clear footprint within 8
blocks. Trees, logs and leaves are never ground; soldiers stand under canopies.
`battle start` refuses until all live staged entities occupy loaded,
entity-simulated chunks and every body is grounded (it counts bodies with no
dry ground within 8 blocks: stage elsewhere); the staging ticket covers the bounded
battlefield. Its reply ends with the ground census of the standing bodies,
`ground: bodies=N inside=0 floating=0 wet=0` on a correct staging.
In battle, soldiers and horses keep the formation's horizontal drive over
relief: one-block steps are jumped, slopes walked down, and a drive into a
trunk or a wall of two or more blocks turns 45 or 90 degrees around it. `battle
status` ends with `grounding=` (bodies still awaiting ground) and the last
ten-second stall census: `advancing=` live steady soldiers of regiments whose
leader advanced at least four blocks, `stalled=` those of them that moved less
than one block; `engaged=` counts melee regiments within six blocks of their
enemy front.
Battle setup rejects Peaceful: its hostile-mob policy removes even persistent
husks. Switching to Peaceful during a battle therefore ends its armies; ordinary
distance-based natural despawn is disabled. Successful setup sets world rules
`mob_drops`, `mob_griefing` and `spawn_mobs` to false. These changes persist
after reset; restore them explicitly if the world is reused.

The side mix is 50% sword/shield infantry, 20% spearmen, 20% archers and
10% cavalry (integer rounding goes to infantry). Regiments are homogeneous:
archers deploy behind the foot and mounted troops on the flanks, with two-block
cavalry frontage. At 10,000 per side, seven foot regiments deploy 46 files each
(322 front positions); cavalry deploys behind their flanks to fit the staging
ticket. Units use husks with vanilla combat item identities and
`solaris_battle` appearance components: red or blue surcoats, shields and
horse caparisons distinguish the sides. The client needs the authored battle
resources to render these custom models.

`run-battle.sh` creates a new run directory and never overwrites an existing
one. `--world` copies a **stopped** native world, including its spawn and
terrain; without it the server generates a fresh world, not necessarily flat.
Pass `--world-seed <seed>` if the copied world's generation seed differs from
the battle seed.
The server pin comes from `tools/dev/pin-release`; the client must contain the
headless recorder, and the pinned server must contain the recorder/telemetry
slice. Both are required for an actual run. For example:

```sh
tools/battle/run-battle.sh --pin /absolute/server-pin --client /absolute/solaris-client \
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

## Take mode (the recorded shot)

`--take` records a shot, not a measurement ladder. Everything that costs ticks in
the shot window is off or moved before it:

- **Checkpoint cadence.** `server.toml` gets `checkpoint_every_ticks =
  (duration + 120) * 20` (`--checkpoint-every-ticks N` overrides; a value not
  above `duration * 20` is refused). The server counts ticks since its last
  checkpoint; the runner forces one (`save-all`, waits for `Saved the game`)
  right after the final census and just before `battle start`, which restarts
  that counter outside the window. The next periodic capture is then due after
  the recorder has stopped. A capture is 97-219 ms on the tick thread in the
  20k ladder, so this is the spike the shot must not contain. The server must
  be built from `95c4bc59` or later: older pins (for example `2df71e49`) do not
  map `[data] checkpoint_every_ticks` and run on the default 1200 with only a
  `ignoring unknown config key` warning; the runner refuses a take when it sees
  that warning.
- **Phase log off.** `MC_CORE_PHASE_LOG` is removed from the server environment
  even if the caller exported it (`--phase-log` brings it back).
- **No status polling.** Before start, each telemetry batch (about every 100
  ticks, a long-poll on the dashboard, no timer) requests `battle status` only
  to drive the population barrier. After the barrier closes there is no poll
  (`--status-every N` brings it back, 0 = never; default 100 outside a take).
- **Gaps are recorded, not fatal** (`--allow-marker-gaps`, implied): recorder
  marker gaps and skipped telemetry ticks go into `take.json`. Any gap, a phase
  log, status polling or a checkpoint spike in the window marks the take not
  clean (`take_clean` in `result.json`, reasons in `take.json`).
- **Deterministic staging.** The battle is `battle setup <seed> ...` from
  `--seed` (default 42) on a copied flat world; `take.json` records the seed, the
  setup command and the census (`side1/side2/resident/active/horses/regiments`)
  taken right before start. Entity UUIDs are random per run; the census is the
  comparison between takes.
- **Telemetry.** `telemetry.jsonl` is the E1 per-tick export (see the contract:
  `tick`, `wall_us`, `phase_us`, `host` with `process_rss_bytes` and cgroup
  series, `battle`, `recorders`), written without gaps filled. `take.json`
  summarises the shot window `battle_start+1 .. recorder last_tick`: samples,
  missing ticks, `wall_us` mean/p50/p95/p99/max, TPS from `wall_start_unix_ns`,
  max `process_rss_bytes`, deferred updates.
- `--view-distance N` sets the server and recorder view distance (default 12;
  the staging ticket stays 12 chunks, so more only adds terrain around the
  field). `--server-scope [SLICE]` starts the server in its own transient
  `systemd-run --user --scope` unit (`server-scope.json` records its cgroup), so
  the `cgroup_*` host series is the server alone, not server plus recorder.

Evidence for "no checkpoint in the window" is two independent records.
`server.log` keeps the INFO line `checkpoint committed tick=N` for every
checkpoint even with the phase log off; `take.json` lists them under
`server_checkpoints` (`in_window`, the last one before the window, and the
ticks left until the next periodic one). The telemetry is the second record: a
capture runs inside `owner_post_us` (in the 20k ladder 100.8-247.5 ms there
against a 23.8 ms maximum on every other tick), so ticks of the window with
`owner_post_us >= 50 ms` are listed as `checkpoint_spike_ticks`. The spike
threshold is a heuristic calibrated on that ladder; the log line is exact. Either
one inside the window makes the take not clean.

```sh
tools/battle/run-battle.sh --take --pin ... --client ... --client-data ... --cache ... \
  --world /absolute/stopped-flat-world --run-dir /absolute/new-run \
  --seed 42 --scenario field-10k --duration 180 --server-scope
```

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
and resident/active units. Outside take mode the runner requests status about
every 100 recorded ticks (`--status-every`); the report pairs each checkpoint with the nearest sample and shows its
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
