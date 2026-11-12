# Reproducible performance profiles

Solaris is a **draft prerelease**. These profiles measure specific workloads on
stated hardware; they are not performance guarantees, vanilla-parity proofs or
player-capacity promises. Protocol bots are real TCP clients, not graphical
Minecraft clients or human players. No survival/gameplay acceptance claim follows
from a successful measurement.

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
- **Build:** `cargo build --release --bin solaris-v2 --bin player-scale-bots
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

`scale-{100k,500k,1m}-{all-active,mostly-resting,sparse-active}` is a
separate public-command recipe, not a replacement for either official heavy
profile. The all-active and sparse-active recipes use the same eight mob
species as the native control; mostly-resting uses 90% cows and 10% zombies
and rejects a window unless more motion rows actually rested than ran fully.
The motion rest fence treats held stacks as potential temptation only when a
registered temptable blueprint lists that item as breeding food; unrelated edit
materials do not make every temptable animal execute its full decision row.
Sparse-active requests 10% of residents active. An observer is assigned to
every advertised active pen, while adjacent forced-chunk tickets retain the
cold pens without ticking their entity columns. No AI or damage relief is
added. This setup is a stress fixture, not a representative settlement.

Each source-tick sample carries opt-in per-region active-chunk and marked-cohort
motion admission/full/rest counts. The runner checks the exact tick ID, every
region's expected active chunk coverage and marked admissions, an exact
full-plus-resting partition of motion admissions, and a lower bound
on all live resident rows. The saved snapshot separately checks typed fixture
UUIDs before and after the run. Live resident totals include players and natural
entities; they are not represented as exact fixture-only counts. Edge fanout
also records entity-viewer tracked pairs tagged with their source tick; pairs
are not distinct entities or proof of packet delivery. Before warmup, one
opt-in cold capture deduplicates the currently tracked non-self IDs after the
all-viewer pump and requires a matching core source-tick sample. It is capped
at 1,000,000 pairs: overflow blocks the row, records a null unique count and
reports the required pair visits and potential ID count. The resulting unique count is
pre-warmup server-tracked state, not measured-window, geometric or delivered
visibility. Results are noncanonical and make no capacity claim.

For each name, `--smoke --smoke-residents 80` uses the same recipe shape at a
smaller resident count, 2 s warmup and 5 s observation. This is a lifecycle
check only, not a measurement of the advertised ladder size. Full recipes keep
the ordinary 30 s/120 s windows and seed 42; resource failure or missing
coverage fails the run rather than shrinking it.

The scale profiles also accept one noncanonical stress selector:

- `--projectiles 0|100|1000` pulses a fixed-size bank of ordinary arrow-filled
  dispensers. A source tick must contain exactly the requested number of flight
  candidates; the peak is also reported, not a sustained-concurrency claim.
  The unarmed control retains the same bank and must remain at zero.
- `--xp-orbs 0|10000|100000 --xp-players 0|1|100|1000 --xp-locality compact|spread`
  uses a single compact bank of ordinary experience-bottle dispensers. The
  background is retained cold, independently of the explicitly counted observers;
  XP observers move within their assigned chunks without edit or chat traffic.
  Zero observers are allowed only with zero orbs; the dashboard must report zero
  players throughout sampled batches, and all fixture regions stay inactive.
  The receipt reports the actual active population, not the resident total. Zero
  adds an observer region but no orb bank, and rejects active XP orbs. Positive
  canonical counts require an exact owner-observed source tick, not merely offered
  bottles or sustained concurrency; the maximum observed count is also reported.
  The noncanonical one-bottle smoke instead requires 3–11 total XP in 1–3
  owner-observed orb bodies, as a bottle's yield splits into denominations
  ([Bottle o' Enchanting](https://minecraft.wiki/w/Bottle_o%27_Enchanting),
  [Experience orb](https://minecraft.wiki/w/Experience_orb)). It is not proof
  of the canonical exact-body workload. Observer locality is checked against
  received chunk centers; it does not claim exact positions.
- `--containers 1000|100000` creates that many chests and five brewing stands.
  Public item-replace commands supply the stands; periodic water-bottle inputs
  keep useful brewing work present. The source receipt checks discovery counts
  by region, active stands, and a tick where all five actual timers advance.
- `--productive-npcs` is unavailable for every value, including `0`, until
  CivSim 3/3 and the actual brain-owner callsites are integrated. The following
  describes the retained acceptance requirements, not an available workload.
- `--productive-npcs 0|1000|10000` replaces the same number of background mobs
  with unbound villagers in isolated farm cells. Each cell has a composter and
  one crop; cells are separated beyond local harvesting reach and public inputs
  periodically restore mature wheat. The receipt requires each occupied cell
  to change during the villager-brain phase in the measured interval. This
  proves isolated crop work, not certified-settlement stock delivery.
- `--damage-victims 1|1000|10000|100000` replaces that many background mobs
  with isolated cows over powered arrow dispensers. The cows are summoned with
  100 health to survive repeated real damage; this is an authored stress
  condition, not vanilla health or a canonical control. The receipt requires
  the requested number of distinct accepted victims in a single source tick
  and separately reports accepted reactive pieces, not command offers.
- Scale variants accept `--spawn-mobs on|off` and `--spawn-monsters on|off`.
  Public gamerule commands set them after fixture preparation; the saved native
  checkpoint must match before and after the run. This proves endpoint state,
  not per-tick rule or observer counters.
- `--reactive-pairs N` confines marked iron golems with zombies wearing authored
  thorns-X armor. Both have raised health. The setup checks saved armor; the run
  requires at least N accepted reactive damage pieces in the source-tick window.
  This is a total, not a claim that all pieces occur on one tick.
- `--churn-percent 0|1|10` replaces that percentage of the background with
  confined marked cows. Public commands alternate them with pigs and back; the
  saved receipt checks unaffected identities and a lower bound of one complete
  identity turnover between snapshots. This is not a per-tick churn-rate proof.
  Churn, damage and wool triggers use public NBT selectors; their selector work
  is part of the offered workload and is not attributed to the target owner.
- `--daylight-zombies N` and `--wool-sheep N` replace background mobs with
  isolated marked cohorts. Daylight is fixed at 2000 with clear weather; zombies
  are authored with high health and no head equipment, and each saved after-image
  must report burning. Sheep start sheared on stone; normal commands introduce
  grass during the run. The receipt checks saved pre-run wool state and reports
  the post-run state, not a per-tick density.
- Fixture blocks used for projectile, XP, damage and brewing preparation and
  every dispenser's ammunition are checked against the server's read-only
  saved-world export before measurement.
- `--map-frames 0|10000|100000` replaces background mobs with item frames.
  During preparation an ordinary protocol player creates a new filled map in
  survival for each batch of at most 256 frames, then switches to creative and
  interacts with each summoned frame; received item metadata is required.
  Saved before/after images must contain the same frame-to-map identities and
  native map records validated by the server's recovery decoder.

For a small lifecycle check, `--smoke` permits smaller positive stress counts
and `--smoke-residents`; it is not evidence for the named full-size points.
The runner rejects missing source counters, insufficient observed peaks and
dropped trigger writes. Requested work is never substituted for observation.

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
- **Per-profile generated chunks/s: not instrumented.** The production dashboard
  reports resident chunks, which can include reloads and exclude evictions.
  Resident-chunk deltas and received packet counts are not generation throughput.
  See [in-process chunk generation](#in-process-chunk-generation) for the separate
  generator benchmark; it must not be combined with these load profiles.

## Many-items benchmarks

Two **noncanonical** profiles measure how the tick pays for dropped-item entities. Neither fills the
pending table above (`publication_evidence=false`, `noncanonical=true` in the receipt).

### `native-100-kelp12k`: the preserved polluted native-100 world

The native-100 recipe (same 100 bots, 100,000 mobs, view/simulation distance 6, so the same cache key
under the same `data/vanilla`) run against the world that the kelp wash-out bug filled with item
entities: **12,218 item entities** at preparation end (about 98 % kelp), growing to roughly 19,000
by the end of a window on a server that still washes kelp. Main preserved it read-only as
`<work>/targets/perf-fixture-cache-kelp12k`; the profile points `--fixture-cache` there by default
and always passes `--require-cache-hit`.

- The recipe never mentions kelp or items, so a fixed server loads and measures exactly this world
  and the receipt stays comparable across the fix (B/A on one world).
- The profile **refuses to prepare**: with no complete entry for the cache key (the recipe or the
  registry content changed) it fails before touching the cache directory instead of silently
  measuring a clean world. It also pins the entry: key
  `e374422d…`, world tree hash `f07d720e…`, 12,218 saved items; a different world fails the run.
- Item ages in the saved world are at most about 2,900; an item despawns at age 6,000 ticks active. A
  fixed server therefore loses the preserved items during a long window (the production rate, not a
  fault): read the resident population per tick from the `items census:` lines, never assume it.

```sh
python3 -m tools.coreperf.profiles native-100-kelp12k "${COMMON[@]}" \
    --server-wrapper "env MC_CORE_PHASE_LOG=1"
```

### `items-heavy`: N item entities on a sky platform

`python3 -m tools.coreperf.profiles items-heavy --items N [--items-bots 10] "${COMMON[@]}"` builds,
with public commands only (no gameplay, no randomness), a stone platform at y=199 (no terrain, caves or
natural mobs), a walled and roofed 14x14 observer pad at the origin where `--items-bots` (default 10)
creative bots walk at 20 slots/s without edits or chat, and N item entities in four classes (30 %,
30 %, 20 %, 20 %; the ground class takes the rounding remainder), seed 42, view/simulation distance 8:

| Class | Layout | What it exercises |
|---|---|---|
| `ground` | two items per block cell on the floor, kinds chosen so neighbours never merge | resting items, the merge scan, tracking |
| `water` | the same grid in a three-deep still pool (stone ring) | items that never rest: every tick moves |
| `pile_full` | 16 full stacks (count 64) of one kind in one position, piles two blocks apart | a same-kind candidate that cannot receive anything |
| `pile_mixed` | 16 distinct kinds, count 1, in one position | candidates of the wrong kind |

Nothing merges, so the platform population is stationary (checked: the saved snapshot must hold
exactly N items above the floor before and after the run; items elsewhere, natural drops such as the
kelp wash-out items of the seed-42 ocean east of the origin, are counted separately in the receipt and
enter the per-tick census). Random ticks are off (`randomTickSpeed 0`) so that growth, and the
wash-out it triggers, is not part of this benchmark. The ladder is N = 0 (platform, pad and bots only), 10,000 and 30,000;
the maximum is 36,000 (the items must stay inside the 6-chunk item tracking range of every bot).
The fixture cache key contains N. All summons run in the last preparation region so items age only from
their summon to the end of preparation (an item ages 6,000 ticks active before it despawns).

### Per-item attribution (`MC_CORE_PHASE_LOG=1`, no work when unset)

The server log gains, before each Overworld `phase ms:` line, the lines `items census:` (resident
items, on ground, moving, mature, and the item step's summed rayon CPU with the clock cost per timed
item), `items merge:` (on the 20-tick cadence: gather/sort and scan time, candidate and overlapping
pairs, merges), `items expiry:` (items removed at age 6,000 and the removal time), `items views:` (wire
view candidates and changed images, and how many are items), and one `edge items:` line per published
tick (viewers, framed packets and bytes to the bots, items among the entities each viewer holds, planner
visits, item movement/spawn/metadata packets and bytes, and the fanout phases `index_ms`, `plan_ms`,
`fill_ms`). `python3 -m tools.coreperf.items_table --leg n0=<run> --leg n10k=<run> --leg n30k=<run>
--leg kelp1=<run> [--md table.md]` turns the runs into the per-phase table, the clock-corrected
per-item step and merge costs, and the least-squares slope in microseconds per item per tick of every
column across the ladder (phases, laps, `tick outer ms:` and amortized `checkpoint capture ms:` lines,
edge fanout), plus a within-run slope per leg. The step timer sums CPU over the rayon
workers (not wall time); the other lines are wall time on the thread that ran them. The attribution
costs time itself: every timed item carries one clock-read pair (`clock_ns`, subtracted from the step
CPU), the census runs inside the entity commit (`census_ms`) and the views probe inside the views phase
(`probe_ms`); the table reports the entity and views phases net of them, and `probe_ms` on the edge
line is the pump-thread cost of counting pairs, kept out of `total_ms`.

## Reproduce

Run from a checkout with the pinned Rust toolchain, Clang/mold, Python 3, Linux
`taskset`, `lscpu`, `lsblk` and `findmnt`. Import the 26.3 content cache as
described in the [operating guide](../OPERATING.md#vanilla-content-cache)
(`solaris-v2 content import --version 26.3 --download`; the cache joins the 26.3
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
COMMON=(--server-bin "$SOLARIS_SERVER_BINARY"
        --bots-bin "$SOLARIS_BOTS_BINARY"
        --build-receipt "$(dirname "$SOLARIS_SERVER_BINARY")/manifest.json")
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

These are earlier claims kept so later documentation cleanups do not lose them.
They are **not** publication evidence and cannot fill the pending table above.

| Release | Workload | Result | Build | Host |
|---|---|---|---|---|
| v0.0.8 (tag `57b95ae9`, 2026-10-18) | 100 clients, about 100,000 mixed mobs (`native-100` shape) | about 16 TPS; earlier runs below 10 TPS | frame-pointer build | reference host; exact CPU for this run unverified |

Source: [v0.0.8 release notes at the tag](https://github.com/kaiserproger/solaris/blob/57b95ae9/docs/releases/v0.0.8.md#performance). They also
report a more compact entity layout and hot passes that no longer scan every
entity row; they do not establish whether either caused the measured gain.
The current reference host uses a Ryzen 5 7535HS; the release notes do not
identify the CPU for this particular run. Its original `result.json`, MSPT
percentiles and full stage breakdown have not been located. The memory
figures for 100k-mob runs (server RSS about 2.3 GiB, cgroup peak about 8 GB
including page cache; [profiling notes](../development/performance-windows-and-profiling.md#load-harness-and-the-frozen-old-core))
are not known to come from the same run. The next comparable number is a
`native-100` quiet-window run on a 1.0 release candidate.

The working notes committed with the release
([`docs/MEMORY.md` at `1ada2ceb`](https://github.com/kaiserproger/solaris/blob/1ada2ceb/docs/MEMORY.md),
lines 38–42) record the `native-100` rounds before the tag. Their reports and
receipts were referenced under the ignored `.analysis/` directory and were not
found in the paths searched; the host is not named per run.

| Date | Change | Frame-pointer steady TPS | Other figures |
|---|---|---:|---|
| 2026-10-17 | r7 baseline | 8.98 | |
| 2026-10-17 | r9–r10: single-lookup contact revision, page-batched parallel `EntityViews::publish`, parallel read-row prep, villager targets from index | 11.42 | release build 11.91 / 11.56 / 12.72 TPS steady / burst / recovery, p50 83 ms |
| 2026-10-18 | entity layout (rare components boxed, sorted lanes instead of six `slots_where` scans, dense `Body` columns) | 14.52 → 15.87 | entity phase 29.6 → 24.8 ms, RSS −415 MiB |

The working notes associate the last row's 14.52 → 15.87 TPS change with the
grouped entity-layout work. It is compatible with the release's rounded
“about 16 TPS”, but the available sources do not establish that it is the same
run. The missing report also leaves the contribution of individual changes
unverified.

## In-process chunk generation

An existing **Gen32** quiet-window measurement on the same Ryzen 5 7535HS reports
the following Solaris vanilla-style Overworld generator costs. These are
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
