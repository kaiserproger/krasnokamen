# Reproducible performance profiles

**Draft prerelease:** these hardware/workload measurements prove neither capacity,
vanilla parity nor survival/gameplay acceptance. Protocol bots are TCP clients,
not graphical clients or humans.

## Publication status

**Measured 2026-11-13** on the reference host below, tree `74aed9fc`, with
release pins from `tools/dev/pin-release --no-incremental`. The runs used the
exclusive measurement lane (`tools/dev/mem-run.sh -x -p`, scope `CPUQuota=600%`):
seed 42, 30 s warmup, 120 s measured, 6 GiB server cgroup. All four receipts
are complete (`status: measured`, `publication_evidence: true`) and are kept on
the reference host under `receipts/PerfReference/74aed9fc/`.

| Profile | Server RSS peak / steady (MiB) | MSPT p50 / p95 / p99 / max (ms) | TPS | Generated chunks/s |
|---|---|---|---|---|
| `idle` | 271.3 / 264.8 | 0.217 / 0.327 / 0.433 / 3.646 | 20.00 | not instrumented |
| `explore-20` | 679.0 / 673.3 | 8.618 / 27.729 / 32.978 / 50.272 | 20.00 | not instrumented |
| `native-100` | 1675.5 / 1611.6 | 63.450 / 79.179 / 88.588 / 210.675 | 15.17 | not instrumented |
| `villager-heavy` | 2112.8 / 2107.7 | 140.110 / 157.006 / 169.489 / 187.242 | 7.12 | not instrumented |

Bot RSS is reported separately: 77.1 MiB (`explore-20`), 125.2 MiB
(`native-100`), 124.3 MiB (`villager-heavy`). Before measurement the
`native-100` world held 293 item entities. The previous measurement (tree
`931b1699`, 7.91 TPS) held about 19,000, almost all of them kelp washed out by
flowing water; the kelp-wash fix removed them. That fix, the redstone entity
query (`67407dd9`) and the entity-commit spike fix (`17c963b4`) all landed
between the two trees, and these runs do not split the gain between them.
`villager-heavy` (120,363 entities, 20,000 of them villagers) is measured here
for the first time; its preparation had failed before the settlement-label fix
(`3633261b`).

Only a complete `measured` receipt from an approved quiet window can replace a
cell. Preparation, short smoke results and the historical in-process generator
benchmark below cannot.

## Reference hardware and build

- **CPU:** AMD Ryzen 5 7535HS with Radeon Graphics, 1 socket, 6 physical cores /
  12 SMT threads. Server affinity is logical CPUs 0–7 (four whole physical
  cores, both SMT siblings); protocol bots use 8–11 (the other two cores).
- **RAM:** 32 GB installed class; Linux `MemTotal=31,551,464 KiB`
  (30.09 GiB usable). Each invocation is bounded by a 6 GiB memory cgroup;
  that cap is not the reported process RSS.
- **World/artifact disk:** Samsung SSD 980 500GB, NVMe, ext4 on
  `/dev/nvme1n1p1`. The OS/root disk is a separate Samsung SSD 970 EVO Plus
  500GB. Worlds are not on tmpfs.
- **OS/kernel:** Ubuntu 26.04.1 LTS (x86-64), Linux `7.0.0-38-generic`.
- **Server source:** `0a86cfffc173d9fe13e9d0795a6c5494fc33a222` plus Main's
  operator chat-spam parity fix in `crates/mc-core/src/tick/input/social.rs`:
  operators still accumulate the saturating counter but are not kicked. The
  original baseline rejected fixture preparation with `disconnect.spam`.
- **Build:** `cargo build --release --bin krasnokamen --bin player-scale-bots
  -j 2`; rustc 1.99.1 (`58cf8f9c3`, LLVM 23.1.1). Cargo release defaults:
  optimization level 3, debug information off, LTO off, 16 codegen units,
  incremental off. Repository target flags are
  `-C link-arg=-fuse-ld=mold`, linker `clang`. No additional `RUSTFLAGS`,
  frame-pointer, native-CPU, PGO or BOLT transformation.
- **Binary identity:** the retained build receipt records SHA-256 hashes of both
  the server and protocol-bot executables. The runner rejects mismatched hashes;
  use the receipt accompanying each run, not a hash from an earlier build.

## Profiles and conditions

All four use seed **42**, the ordinary generated Overworld (not flat, synthetic,
or Earth WorldPlan), native storage and tick WAL, offline local authentication,
no plugins, clear weather, easy difficulty, daylight enabled and
`randomTickSpeed=3`. Natural spawning is disabled by public preparation commands;
pre-existing natural entities are not silently deleted from the count. Every
receipt records their actual types/counts before and after the run. All entity
AI, health, movement and collision remain enabled: no `NoAI`, invulnerability,
refills or reduced AI cadence.

| Profile | Protocol clients | Named fixture entities | Server view / simulation distance | Activity |
|---|---:|---|---:|---|
| `idle` | 0 | No summoned cohort | 8 / 8 | Generated spawn area, no clients or player input; empty-server pause disabled so simulation continues |
| `explore-3` | 3 | No summoned cohort | 8 / 8 | Same terrain, spacing and per-player movement as `explore-20`; the 1–5-player weak-server shape, not a substitute for the 20-player result |
| `explore-20` | 20 | No summoned cohort | 8 / 8 | One client per terrain anchor, 256-block grid spacing; creative, straight-line walking with sprint/jump legs; no block edits or recurring commands |
| `native-100` | 100 | 100,000 mixed mobs; no fixture villagers | 6 / 6 | Creative observers distributed across roofed pens; movement at 20 offered slots/s, dig/place at 2/s and chat at 1/s per client |
| `villager-heavy` | 100 | Same 100,000 mixed mobs **plus** 20,000 villagers | 6 / 6 | Same activity, distributed across mob and villager pens; real beds and composters |

The mixed cohort is **12,500 each** of cow, sheep, pig, chicken, zombie, skeleton,
spider and creeper. Those eight kinds and the heavy-profile villagers carry
`PersistenceRequired:1b` and a named fixture marker. Villagers are additional to
100,000 mobs, not included in that total. Saved-world checks compare typed cohort
UUIDs before and after; death/loss is a failed run, never hidden by replacement.
Natural entities outside the marker are listed separately in the receipts.

The heavy fixture is shared with the PGO experiment: public `/fill`, `/setblock`
and `/summon` commands create species-separated, roofed pens, up to 1,024 entities
per pen. Adjacent same-row mob pens share observer anchors; each villager has a
bed and composter in the bounded village pens. This is an operator-authored dense
stress scene, **not** 20,000 independent, naturally generated productive villagers.
Fixture creation uses distance 8; measured restarts use the distances above.

Spider pens have three blocks of interior headroom (floor top Y=64, roof Y=67);
the other mixed-mob pens retain their Y=71 roofs. Spiders still take ordinary
damage after detaching from a wall, so taller pens permit damaging free falls.
Climb contact also resets retained fall distance in the native movement path:
ceiling bounces must not accumulate separate short drops into false damage.
The lower physical ceiling bounds a single unassisted drop without changing AI,
health or damage rules. The recipe hash changes the fixture-cache key, so old
tall-pen fixtures are not reused.

Explorers use the existing bot's received-terrain collision checks. They may
turn when terrain is missing/unwalkable or the height change is too large;
`--explore` disables the ordinary 200-tick turn, not collision. Read
`chunk_boundaries_crossed`, route-refusal and movement counters in the receipt.
It is not flying, teleport spam, or a promise that every client advances
indefinitely through terrain. Login includes operator-assisted safe landing.

### Noncanonical resident-scale variants

`scale-{100k,500k,1m}-{all-active,mostly-resting,sparse-active}` uses public
commands and is **not** either official heavy profile or a representative
settlement/capacity claim.

| Variant | Fixture and admission |
|---|---|
| all-active | Same eight species as native control |
| mostly-resting | 90% cows, 10% zombies; more motion rows must actually rest than run fully |
| sparse-active | Same eight species as native control; requests 10% active; observer per advertised active pen; adjacent forced-chunk tickets retain cold pens without ticking their entity columns |

No AI/damage relief. Held stacks count as potential temptation only when the
registered temptable blueprint lists that breeding food, not for unrelated
edit materials.

Every source tick carries opt-in regional active-chunk and marked-cohort
motion admissions/full/rest counters. Acceptance checks exact tick identity,
every region's expected coverage/admissions, exact full+rest partition and a
lower bound on live residents. Saved before/after typed UUIDs independently
check fixture survival. Live totals include players/natural entities, not exact
fixture-only counts.

Edge tracked pairs carry their source tick; they are neither unique entities
nor delivered packets. One opt-in pre-warmup cold capture deduplicates tracked
non-self IDs after the all-viewer pump and requires a matching core sample.
The 1,000,000-pair cap fails closed on overflow: unique count null, required
pair visits/potential IDs reported. This is server-tracked pre-warmup state,
not measured-window, geometric or delivered visibility.

Full runs keep seed 42 and 30/120 s windows; missing coverage or resource failure
fails instead of shrinking the workload. `--smoke --smoke-residents 80` preserves
recipe shape at reduced population, 2/5 s, for lifecycle checks only.

One noncanonical stress selector may be added:

| Selector | Physical recipe and required observation |
|---|---|
| `--projectiles 0\|100\|1000` | Ordinary arrow-filled dispenser bank. The unarmed zero control keeps the bank and must stay at zero in every sample. For positive N, one source tick must contain exactly N flight candidates; report peak separately, not sustained concurrency. |
| `--xp-orbs 0\|10000\|100000 --xp-players 0\|1\|100\|1000 --xp-locality compact\|spread` | One compact experience-bottle dispenser bank; background retained cold independently of counted observers. Observers move in assigned chunks without edit/chat. Zero observers requires zero orbs, zero dashboard players throughout sampled batches and all fixture regions inactive. A zero-orb run with observers adds their region but no bank and refuses active XP. Positive canonical counts require an exact owner-observed source tick and report maximum, not offered bottles or sustained concurrency. |
| `--containers 1000\|100000` | N chests plus five brewing stands supplied by public item-replace and periodic water bottles. Check regional discovery, active stands and a source tick advancing all five actual timers. |
| `--productive-npcs 0\|1000\|10000` | **Unavailable for every value, including 0**, until CivSim 3/3 and actual brain-owner callsites integrate. Retained target: replace N background mobs with unbound villagers in isolated composter/crop cells beyond local harvesting reach; public inputs restore mature wheat; every occupied cell must change in the measured villager-brain phase. Isolated crop work, not certified-stock delivery. |
| `--damage-victims 1\|1000\|10000\|100000` | Replace N background mobs with isolated cows over powered arrow dispensers, authored 100 HP for repeated real damage. Not vanilla health/canonical control. Require N distinct accepted victims in one source tick; report reactive pieces separately from offers. |
| `--reactive-pairs N` | Confined marked golems and thorns-X-armored zombies, both raised-health. Check saved armor and at least N accepted reactive pieces over the window, not necessarily one tick. |
| `--churn-percent 0\|1\|10` | Replace that background percentage with confined marked cows; public commands alternate pigs and cows. Saved images check unaffected identities and at least one complete identity turnover, not per-tick rate. |
| `--daylight-zombies N` | Replace background with isolated high-health, bare-headed marked zombies; clear day 2000, every saved after-image burning. |
| `--wool-sheep N` | Replace background with isolated marked sheep, initially sheared on stone; introduce grass by normal commands. Check saved pre-wool state and report post-state, not per-tick density. |
| `--map-frames 0\|10000\|100000` | Replace background with frames. Ordinary player creates a fresh map in survival per ≤256-frame batch, then equips summoned frames in creative; require received item metadata. Before/after images retain identical frame→map identities and recovery-decoder-validated native map records. |

XP smoke uses one bottle: require 3–11 total XP in 1–3 owner-observed bodies,
not the canonical exact-body workload
([bottle](https://minecraft.wiki/w/Bottle_o%27_Enchanting),
[orb denominations](https://minecraft.wiki/w/Experience_orb)).
Observer locality checks received chunk centers, not exact positions.
XP receipts report actual active population, not resident totals.

Scale variants also accept `--spawn-mobs on|off`/`--spawn-monsters on|off`.
Public post-preparation gamerules must match both saved checkpoints; this
proves endpoints, not per-tick rules/observers. Churn, damage and wool public
NBT-selector work belongs to offered load, not the target owner.
Before measurement, read-only server exports check projectile/XP/damage/brewing
fixture blocks and every dispenser's ammunition.

Smoke permits smaller positive stress counts, never evidence for full-size
points. Missing source counters, insufficient observed peaks or dropped trigger
writes fail; requested work never substitutes for observed work.

### Windows and metric definitions

- A fresh server process opens the prepared world. Fixture generation and all
  client login/setup are **outside** the measured interval.
  After dashboard readiness the collector waits for the first pushed exact-tick
  sample before starting warmup; an empty or invalid stream still fails closed.
- After all requested clients are ready: **30 seconds of active warmup**, then
  **120 seconds measured**, using the same offered workload in both intervals.
  Idle uses the same wall durations without bots. Changing either duration
  creates a different profile and is recorded in the receipt.
- **MSPT p50/p95/p99/max** use nearest-rank percentiles of every exact source
  `wall_us` sample whose `wall_start_unix_ns` lies in the half-open measured
  interval. This is active core-iteration wall time, excluding pacing sleep;
  dashboard rolling percentiles are not substituted.
- **TPS** is completed source iterations starting in that interval divided by
  its **full 120 seconds**, not the span from first to last observed sample.
  A missing/reset/overwritten tick stream invalidates the result.
- **RSS steady** is the median sampled server process RSS during measurement.
  **RSS peak** is Linux `VmHWM`, the server-process lifetime peak observed during
  measurement (therefore including restart/login/warmup, not the separate fixture
  builder or later offline snapshot reader). Bot RSS is recorded separately,
  never added to a number labelled server RSS. Samples follow exact-tick stream
  notifications, approximately once per 20 ticks; they are not a continuous RSS
  trace or a cgroup/page-cache measurement.
- Each run checks the live player count, successful complete bot schedule,
  movement, clean server/bot exits and saved fixture identities. A low TPS value
  is reported, not converted into a capacity pass or discarded.
- **`summary.threads`** is a first-to-last per-thread-name sample of
  `/proc/<pid>/task/<tid>/{stat,schedstat}` (CPU seconds and schedstat field 2,
  run-queue wait), taken about once a second, with the same-named threads summed.
  A name missing from either sample, or whose thread count changed, is
  `unavailable` rather than a lifetime delta read as a window; `tick_thread` is
  the `mc-core-tick` row and a run without it fails (`require_thread_wait`), so a
  laps-only receipt cannot pass. The `*_per_tick` fields divide by the ticks of
  the source samples between the two process samples, not by the whole window.
  A `--remote-host` session reads them on the host that runs the server.
- **`summary.journal`** is the window delta of the WAL counters the server
  reports under `/stats.journal` (bytes written, file and directory fsync counts,
  cumulative service time), with `fsync_ms_mean` over *both* fsync counts,
  `fsync_ms_max_since_start` (a process-lifetime `fetch_max`, not a window
  figure), bytes per source tick and the last batch size. A server without the
  counters yields a named `unavailable`.
- **`summary.errors`** is the window delta of the reliable and input drop
  counters, every abnormal connection-close cause that moved
  (`reliable-overflow`, `*-write-failed`, `*-reliable-full`, …) and the last
  warning list. A normal end (client EOF, core-ordered kick, the lifecycle's own
  removal) is not a cause.
- **`perf_window`** is the last-level counter capture of exactly the measured window: one
  `perf stat -x,` attached to the process that owns `mc-core-tick`, started at the window's first
  sample and ended by its own `sleep` (counters only, no `--per-thread`, so the shutdown's
  `killpg` never lands on perf). It reports each event's count with the percentage of the window
  it ran, and `fills_per_resident` / `fills_per_resident_per_second` from
  `ls_dmnd_fills_from_sys.all`. A `<not counted>` value, a running percentage below 100 (a
  multiplexed counter) or a missing/refused perf is a named `unavailable`, never a quoted number;
  an unavailable capture does not fail the window's other figures.
- **`client_tick_arrival`** is the bot-observed interval between the server's
  clock heartbeats (p50/p95/p99/max) and the per-tick period derived from the
  heartbeat's own game-time delta, per open-loop window; a window the bot report
  leaves out is named unavailable.
- **Per-profile generated chunks/s: not instrumented.** The production dashboard
  reports resident chunks, which can include reloads and exclude evictions.
  Resident-chunk deltas and received packet counts are not generation throughput.
  See [in-process chunk generation](#in-process-chunk-generation) for the separate
  generator benchmark; it must not be combined with these load profiles.

## Many-items benchmarks

`items-heavy` below is noncanonical (`publication_evidence=false`,
`noncanonical=true`), never a publication-table result. The other many-items
workload is ordinary `native-100` on a **retained kelp-polluted cache**:
`--require-cache-hit` prevents silently preparing a clean replacement.
The 2026-11-12 window scripts name its cache (~4,000 items, 1,270 moving).
The recipe itself names neither kelp nor items, allowing the fixed server to
load the same world across a fix.

In window-d, items represented ~4% of tracked pairs and <1% of packets:
the planner gates their movement at vanilla's 20-tick interval; most of the
~44,000 packets/tick were mobs.

### `items-heavy`: N item entities on a sky platform

`python3 -m tools.coreperf.profiles items-heavy --items N [--items-bots 10] "${COMMON[@]}"`
uses public commands only (no gameplay/randomness), seed 42, VD/SD 8:
stone floor y=199 (no terrain/caves/natural mobs), deck y=207 and walled/roofed
14×14 observer pad at origin.
Ten default creative bots walk at 20 offered slots/s without edit/chat.
Item classes split 30/30/20/20%, ground taking the rounding remainder:

| Class | Layout | What it exercises |
|---|---|---|
| `ground` | two items per block cell on the floor, kinds chosen so neighbours never merge | resting items, the merge scan, tracking |
| `water` | the same grid in a three-deep still pool (stone ring) | items that never rest: every tick moves |
| `pile_full` | 16 full stacks (count 64) of one kind in one position, piles two blocks apart | a same-kind candidate that cannot receive anything |
| `pile_mixed` | 16 distinct kinds, count 1, in one position | candidates of the wrong kind |

Before/after snapshots require exactly N items above the floor: no merges or
pickups. Other drops (including seed-42 ocean kelp) are separate receipt counts
but enter tick census. The deck keeps bots at y=208 outside pickup reach;
without it, highest-surface/longest-runway landing put bots among items.
Early window-d lost 1,853/10,000 to inventories and counted 60 merges
(water lost none), hence the physical separation.

`randomTickSpeed=0` excludes growth/wash-out. Ladder: 0 (platform/pad/bots),
10,000, 30,000; maximum 36,000 keeps all items within every bot's six-chunk
tracking range. N is in the cache key. Summons occur in the final preparation
region to limit active aging (despawn at 6,000 ticks).

### Per-item attribution (`MC_CORE_PHASE_LOG=1`, no work when unset)

Opt-in item log probes precede each Overworld `phase ms:` line; the edge probe
has the per-published-tick cadence shown below:

| Line | Contents |
|---|---|
| `items census:` | Resident/ground/moving/mature items; summed rayon step CPU and timed-item clock cost |
| `items merge:` | 20-tick gather/sort/scan times, candidate/overlap pairs, merges |
| `items expiry:` | Age-6,000 removals and time |
| `items views:` | Wire candidates/changed images, including item counts |
| `edge items:` (each published tick) | Viewers, framed packets/bytes, tracked items, planner visits, item movement/spawn/metadata packets/bytes; `index_ms`, `plan_ms`, `fill_ms` |

`python3 -m tools.coreperf.items_table --leg n0=<run> --leg n10k=<run>
--leg n30k=<run> --leg kelp1=<run> [--md table.md]` reports phases,
clock-corrected item step/merge costs, least-squares µs/item/tick ladder slopes
and within-run slopes. Columns include phase laps, `tick outer ms:`,
amortized `checkpoint capture ms:` and edge fanout.
Only the rayon step is summed worker CPU; other timers are executing-thread
wall time. Subtract each timed item's `clock_ns` pair. Entity `census_ms` and
views `probe_ms` are removed from their phase totals; edge `probe_ms` is
pump-thread counting cost excluded from `total_ms`. Instrumentation is not free.

## Reproduce

Run from a checkout with the pinned Rust toolchain, Clang/mold, Python 3, Linux
`taskset`, `lscpu`, `lsblk` and `findmnt`. Import the 26.3 content cache as
described in the [operating guide](../OPERATING.md#vanilla-content-cache)
(`krasnokamen content import --version 26.3 --download`; the cache joins the 26.3
protocol identity with the 26.1.2 gameplay pack, so no separate legacy cache is
needed for these profiles). `tools.coreperf.profiles` reads `./data/vanilla` by
default; point it elsewhere with `--vanilla-data-dir`.
Neither Mojang bytes nor prepared worlds are shipped in Git.

### Build once and retain provenance

Use immutable `pin-release` artifacts for both the server and protocol bots,
from the same source tree. The runner checks their manifests, binary names and
hashes, and rejects pins from different trees. On the owner host it also requires
the canonical pin store and exact manifest paths. Build outside measurement:

```sh
ROOT="$(pwd -P)"
eval "$(/run/media/kaiserroman/games/solaris-work/solaris/tools/dev/pin-release "$ROOT" --no-incremental)"
eval "$(/run/media/kaiserroman/games/solaris-work/solaris/tools/dev/pin-release "$ROOT" --bots --no-incremental)"
```

`pin-release` builds through the owner-host memory runner and retains source and
binary provenance. Pass `--no-incremental` (as above): the default incremental pin
has more codegen units and the profile runner refuses it for measurements. Do not compile during somebody else's measurement window.

The pinning commands above are owner-host tools. On another Linux host, copy
each pin directory with its original `manifest.json` unchanged and pass the
relocated binary paths below. The manifest retains the producer's absolute
path; outside the owner host, the matching filename and SHA-256 bind it to
the copied binary without requiring the producer's filesystem layout.

### One command per profile

The following Bash array only supplies the common binary/provenance paths;
each invocation owns preparation/cache reuse, startup, warmup, measurement,
shutdown, native snapshot checks and its receipt directory.

```sh
COMMON=(--server-bin "$KRASNOKAMEN_SERVER_BINARY"
        --bots-bin "$KRASNOKAMEN_BOTS_BINARY"
        --build-receipt "$(dirname "$KRASNOKAMEN_SERVER_BINARY")/manifest.json")
python3 -m tools.coreperf.profiles idle "${COMMON[@]}"
python3 -m tools.coreperf.profiles explore-20 "${COMMON[@]}"
python3 -m tools.coreperf.profiles native-100 "${COMMON[@]}"
python3 -m tools.coreperf.profiles villager-heavy "${COMMON[@]}"
```

Fixture preparation uses the harness's memory guard, so execute these commands
inside a bounded cgroup. On the shared host, prefix **each** command with
`<work>/solaris/tools/dev/mem-run.sh -x -p -m 6G`; the lanes serialize
themselves, so no Main GO is needed.
On another Linux host use the same runner or an equivalent bounded cgroup,
record the cap and run without competing builds/tests. The published affinity
is server logical CPUs `0-7`, bots `8-11`; override `--server-cpus` and
`--bot-cpus` for a different topology and report the new mapping. Logical CPU
IDs are not a count of physical cores.

Default output is a unique `.analysis/validation/<UTC>-profile-<name>-<pid>/`.
`--out` can choose a **new** absolute directory; existing receipts are never
overwritten. `--fixture-cache` selects the shared pristine cache. A cache hit
copies the world, verifies its content hash and re-reads the named entity cohort;
binary hashes identify both its producer and current consumer. Use
`--prepare-only` outside the quiet window to populate it first. A `prepared`
receipt is **not** a measurement. The PGO cache is reusable only when the recipe,
content and fixture parameters match exactly; instrumented PGO timings are never
published as these release measurements.
Public input-echo fences during preparation share the bot's total
`--prepare-seconds` readiness deadline rather than a separate per-batch timeout.
Regional fixture clients use open-loop readiness to complete their setup before
Ready; no Run event is issued and their required schedule is never started.
The setup client executes fixture commands before selecting its landing, retaining
received chunk states and updates so the landing uses the final received support
and clearance rather than terrain that the fixture later replaced.
After setup commands, the operator requests a viewer-world fence. The edge
waits until the post-request snapshot has no owed block or chunk repairs and
queues the response behind those world frames; the bot processes them before
selecting its landing. The separate chat echo fences input order only.
Scale-pen spawn centers are inset from the walls for the widest cohort body
(the 1.4-block spider), including at full 1,024-resident grid capacity.
The bot continues processing packets and replying to heartbeats while waiting
for a slow `save-all flush` to complete; measured open-loop demand is unchanged.

For a lifecycle-only check outside an exclusive window, add `--smoke`: the same
full population runs for 2 seconds warmup and 5 seconds observation. Its status
is `smoke`, with `publication_evidence=false`; even a successful smoke is not
an approved performance measurement. Use the ordinary memory-capped lane, not
`-x -p`, while Main's quiet windows are suspended.

### Evidence retained by each invocation

- `result.json`: status (`measured`, `prepared`, `smoke` or `failed`), exact invocation,
  workload conditions, host CPU/RAM/OS/kernel/disks/filesystem, release build
  receipt, registry hashes, commands/exits/log paths and the measurement summary.
- `fixture.json`, `prepare/receipt.json`: exact public commands, typed cohort,
  preparation provenance and cache status.
- `run/server.toml`, `run/server-command.json`, `run/schedule.json`, bot command
  and `run/bots-report.json`: actual configuration and offered/observed activity.
- `run/samples.jsonl`: raw exact ticks, process RSS, dashboard populations and
  bot counters; `run/result.json`: measured window boundaries and statistics.
- Server/bot logs plus read-only native snapshot receipts. Failed or incomplete
  runs retain evidence and return nonzero; do not publish their partial summary
  as a successful profile.

## Historical results

Historical claims are kept so later documentation cleanups do not lose them.
They are **not** current publication evidence and cannot fill the pending table above.

| Release | Workload | Result | Build | Host |
|---|---|---|---|---|
| v0.0.8 (tag `57b95ae9`, 2026-10-18) | 100 clients, about 100,000 mixed mobs (`native-100` shape) | about 16 TPS; earlier runs below 10 TPS | frame-pointer build | reference host; exact CPU for this run unverified |

Source: [v0.0.8 notes at the tag](https://github.com/kaiserproger/krasnokamen/blob/57b95ae9/docs/releases/v0.0.8.md#performance).
They mention compact entity layout and removal of whole-row hot scans, without
causal attribution. CPU for that run, original `result.json`, MSPT percentiles
and full stages remain unverified/unlocated. The reference host is a Ryzen 5
7535HS; separate 100k-mob figures (~2.3 GiB RSS/~8 GB cgroup including cache,
[profiling notes](../development/performance-windows-and-profiling.md#load-harness-and-the-frozen-old-core))
are not tied to that run. A new comparable result needs a quiet-window
`native-100` release-candidate run.

Pre-tag rounds appear in [working notes at `1ada2ceb`](https://github.com/kaiserproger/krasnokamen/blob/1ada2ceb/docs/MEMORY.md),
lines 38–42. Referenced ignored `.analysis/` receipts were not found in searched
paths; per-run hosts were not named.

| Date | Change | Frame-pointer steady TPS | Other figures |
|---|---|---:|---|
| 2026-10-17 | r7 baseline | 8.98 | |
| 2026-10-17 | r9–r10: single-lookup contact revision, page-batched parallel `EntityViews::publish`, parallel read-row prep, villager targets from index | 11.42 | release build 11.91 / 11.56 / 12.72 TPS steady / burst / recovery, p50 83 ms |
| 2026-10-18 | entity layout (rare components boxed, sorted lanes instead of six `slots_where` scans, dense `Body` columns) | 14.52 → 15.87 | entity phase 29.6 → 24.8 ms, RSS −415 MiB |

The notes attribute 14.52→15.87 TPS to grouped layout work; it is compatible
with rounded “about 16”, but neither identical-run provenance nor individual
contributions are established without the missing report.

## In-process chunk generation

An existing **Gen32** quiet-window measurement on the same Ryzen 5 7535HS reports
the following Krasnokamen vanilla-style Overworld generator costs. These are
**single-threaded in-process generation**, not full-server requested chunks/s,
not exploration throughput and not Mojang's Java server.

| Seed | Median ms/generated chunk | Observed min–max (ms/chunk) | Reciprocal rate (chunks/s) |
|---|---:|---:|---:|
| 712816 | 17.335 | 17.290–17.355 | 57.69 |
| 5617830 | 14.145 | 14.114–14.231 | 70.70 |
| 42 | 13.786 | 13.734–13.796 | 72.54 |

Each seed has three runs of 36 fresh chunks, `x,z=-3..2`; each repetition creates
a new generator, with data/tag/village-plan setup outside the timer. Ores,
vegetation and intersecting structure plans remain enabled. Lighting is measured
separately and **excluded** from the generation numbers above, as are network,
player simulation, persistence and startup. No explicit warmup run was excluded.
The rate column is `1000 / median(ms/chunk)`, not an independently measured
concurrent throughput. Three observations are not stable p99 evidence.

Provenance: Gen32's `default-measurements.json`, `simd-default` rows, measured
with `mem-run.sh -x -p -m 6G measure-default.sh`, default x86-64 release target
with Rust/Cargo rustflag environment overrides removed. Frozen executable
SHA-256: `d327b04e683d59eced6a0d61859d62b856639507a18285ba7afbc856e15ca53b`.
This historical source identity is base
`b8210b456857a0a43f03a0bc1530204d5d728eb9` plus the fp32/ore patch
`83b5d1a6a98d98e7ab3ddbdc5d42fd37c1bf499448231ad37dcd34a565250b6b`
and SIMD patch
`f969e468a707bbed311bad4dd38d6222eb5d5b940005c75a620f12c3969b9db1`;
it is not a new measurement of the profile server binary. Its raw artifacts are
not retained in the repository.

Raw ms/chunk observations, in run order:

```text
712816:  17.33538888888889, 17.35509172222222, 17.29047088888889
5617830: 14.11377777777778, 14.14462250000000, 14.23136988888889
42:      13.78616155555556, 13.79593002777778, 13.73403130555556
```

The [retained stage benchmark](../../crates/mc-worldgen/src/terrain/profile_tests.rs)
reproduces that workload in the current checkout. Build it **before** the quiet
window, then run the compiler-reported release test executable:

```sh
cargo test -p mc-worldgen --lib --release --no-run
# TEST_BINARY is the mc_worldgen executable printed by Cargo above.
"$TEST_BINARY" --ignored --exact terrain::profile_tests::vanilla_stages --nocapture --test-threads=1
```

`vanilla_stages_shared_solids` measures the same chunks as chunk workers
generate them with the shared solids cache (one density fill per chunk, timed
apart), and `vanilla_stages_gpu_density` (`--features gpu`) with GPU density;
every mode also prints `bake=`, the light a worker bakes with no neighbours.

Village placement caches the mirror/rotation result for each processed block
state for the duration of one piece, after processors and before waterlogging.
This avoids repeated registry/property resolution for repeated palette states;
the cache is discarded after the piece and can be removed if that resolution
ceases to dominate. It is not a per-chunk or live-server speedup claim. The
fixed-seed region block fingerprint and village placement tests fence output.

Use the same memory/quiet-window rules as above. No server-throughput ratio
against vanilla is claimed.
