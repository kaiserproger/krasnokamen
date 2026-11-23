# Benchmarks: weak hardware, core scaling, worldgen, VPS

**Draft prerelease:** measurements describe a workload, binary and host, not a
performance guarantee, capacity promise or vanilla-parity result.
Every published number comes from one documented command, a release build and
a fixed seed, with `result.json`, `result.md` and a hardware record.
`tools/bench` selects matrix cells; it delegates server/fixture/bot lifecycle to
[`tools.coreperf.profiles`](PROFILES.md) and generation to `world pregenerate`.
[PROFILES owns exact-tick TPS/MSPT and RSS definitions](PROFILES.md#windows-and-metric-definitions);
[measurement policy](../development/performance-windows-and-profiling.md) owns
quiet windows, lanes and comparison rules.

## CPU counts are logical CPUs

On the reference 6-core/12-thread host, SMT siblings are `0-1, 2-3, … 10-11`:
**`0-5` means three physical cores**, not six.

| `--cpu-mode` | Server CPUs for increasing N | Meaning |
|---|---|---|
| `logical` (default) | `0`, `0,1`, `0-3`, `0-5`, `0-7`, `0-11` | VPS-like logical CPUs, including SMT siblings |
| `physical` | `0`, `0,2`, `0,2,4,6`, `0,2,4,6,8,10` | One CPU per physical core; excessive counts skipped |

Bots use remaining CPUs, preferably untouched physical cores. At full-pool
server affinity they share it; tables disclose this.

## Prerequisites

Linux, Python 3.11+, `taskset`, `lscpu`, `lsblk`, `findmnt`, cgroup v2 and
`systemd-run` (user session or root). Import the 26.3
[content cache](../OPERATING.md#vanilla-content-cache); select its `version.json`
directory with `--vanilla-data-dir`, `$KRASNOKAMEN_VANILLA_DATA_DIR`, or
`./data/vanilla`. Optional GNU `/usr/bin/time` adds worldgen peak RSS and
CPU-ms/chunk. Use the `performance` governor for comparisons
(`cpupower frequency-set -g performance`); its actual value is recorded.

[Build and pin server and bots once](PROFILES.md#build-once-and-retain-provenance),
outside measurement. The bench never builds: same-tree nonincremental
`pin-release` manifests are required; bare binaries, incremental pins and
handwritten receipts refuse. Binary kind/target, source commit and dirty state
come from manifests, not the current checkout. Commands use
`$KRASNOKAMEN_SERVER_BINARY`/`$KRASNOKAMEN_BOTS_BINARY`, or explicit
`--server-bin`/`--bots-bin`.

On the shared host use Main's absolute `tools/dev/mem-run.sh -x -m 6G` for
measurement, within the policy's authorized windows; the lane serializes itself,
without separate GO. Pass `--slice solaris-heavy.slice` so nested limited scopes
count there. Results use a new `.analysis/bench/<UTC>-<command>-<pid>/`
(`--out` overrides; never overwrite): `result.json`, `result.md`, and full
profile receipts under `runs/`.

## `weak-hw`: the "$6 VPS" shape

```sh
python3 -m tools.bench weak-hw
python3 -m tools.bench weak-hw --cores 1 --memory 2G --profiles idle,explore-3
python3 -m tools.bench weak-hw --cpu-mode physical --cores 2
```

| Option | Default / contract |
|---|---|
| `--cores`, `--cpu-mode` | 2, `logical` |
| `--memory` | `4G`; `none` disables |
| `--profiles` | `idle,explore-20,native-100`; [workload definitions](PROFILES.md#profiles-and-conditions) |
| `--worldgen-radius` | 256 blocks; 0 skips |
| `--prepare-memory` | `8G` for preparation, not the limited server |

The server runs with `taskset` and `systemd-run --scope -p MemoryMax=…
-p MemorySwapMax=0`, execed in place to preserve PID/RSS sampling. Worker counts
follow affinity. Bots and preparation are outside the server cap; fixtures use
the whole CPU pool once, then each cell copies the pristine `--fixture-cache`.
The 100-bot/100,000-mob `native-100` can exceed 4G: OOM is failure, not omitted.

Each profile reports TPS, MSPT p50/p99/max, server RSS steady/peak and
**resident chunks/s**, a dashboard proxy including reloads, not generation.
The separate fresh-world generation line uses seed 42 around (0,0), the same
CPU/RAM limits, and generated chunks divided by time from first generation
progress to completion: world opening excluded, light bake/store writes included.
`explore-3` changes only the player count from `explore-20`; label both workloads,
never report their difference as an optimization.

Optional **usable-view time:** set `KRASNOKAMEN_CLIENT_BINARY` to a pinned native
client with the [verified content environment](../development/real-client-gates.md#prerequisites).
Each local capped cell then runs `weak-first-view`: same server CPU/RAM limits,
client on bot CPUs, seed 42, VD 8, fresh world/private settings. Time starts before
server launch and ends at a GPU-confirmed Play view without loading overlay,
including server, Xvfb and client startup. The linked receipt keeps screenshot,
binary identities, process RSS and command. Without a client the field is absent,
never replaced by bot login/readiness timing.

## `scaling`: one workload at 1, 2, 4, 6, 8, 12 CPUs

```sh
python3 -m tools.bench scaling
python3 -m tools.bench scaling --cpu-mode logical,physical
python3 -m tools.bench scaling --profile explore-20 --cores 1,2,4
```

Default: `native-100`, logical series, `--memory none`. Hold profile/seed/window
fixed; vary only affinity. Speedup = lowest-count MSPT p50 / N-count MSPT p50;
efficiency = speedup / (N / lowest N). TPS saturates at 20 and cannot show
scaling there. `native-100` is CPU-bound; the reference `explore-20` used about
0.3 core. Shared bot CPUs and skipped excessive physical-core counts are disclosed.

## `worldgen`: chunks/s, optional vanilla head-to-head

```sh
python3 -m tools.bench worldgen --cores 1,2,4,6,8,12 --repeat 3
python3 -m tools.bench worldgen --cores 2 --radius 512 --vanilla-jar /path/to/server.jar
```

Each CPU cell pregenerates a fresh square, seed 42 (`--seed` overrides),
`--center 0,0`, default radius 256 = 33×33 chunks. `--repeat` gives fresh-world
repetitions and median rate; output includes speedup against lowest count,
CPU-ms/chunk and peak RSS.

Optional Mojang **bundler** jar selection: `--vanilla-jar`,
`$KRASNOKAMEN_VANILLA_JAR`, then `./.analysis/server.jar`; Java must exist.
No jar is copied/committed; absence produces a skip note. Vanilla uses the same
requested square, CPUs and memory cap, `-XX:ActiveProcessorCount=N`, heap 60%
of cap, ≤16×16-chunk `forceload add` tiles and a generated datapack counting
loaded chunks every ten ticks until `BENCH_READY`, then `save-all flush`.
Rate = requested chunks / time from first forceload through finished save,
excluding JVM startup. Vanilla also generates a required neighbour halo, so its
per-requested-chunk rate is flattered less than Krasnokamen's. This rough ratio
compares unequal incidental work, **not parity**, nor the
[single-thread in-process figures](PROFILES.md#in-process-chunk-generation).

## `vps`: a remote host, server there, bots here

`vps` relocates the `weak-hw` server via `RemoteSession`; local fixtures, bots,
open-loop schedules, exact server ticks and final cohort checks remain.
Defaults: `idle,explore-20`, all remote CPUs, no memory cap; add `native-100`
only if it fits.

```sh
python3 -m tools.bench vps --host root@203.0.113.7 \
  --remote-data-dir /opt/solaris/.local/share/krasnokamen/content/26.3 \
  --server-bin "$KRASNOKAMEN_SERVER_BINARY" --bots-bin "$KRASNOKAMEN_BOTS_BINARY"
# Optional: --ssh-options="-p 2222 -i KEY" (use `=`: value starts with a dash)
# --cores 2 --memory none
# --profiles idle,explore-20 --worldgen-radius 256 --game-port 25590
# --dash-port 25591 --bot-cpus 4-7 --tunnel-game
```

Remote names are `solaris-bench-*`. **The runner never writes `/opt/solaris`
or stops/restarts the `krasnokamen` unit**; only its content cache is read.

1. Local preflight; one key-authenticated BatchMode/ControlMaster SSH session.
   Remote prerequisites: `rsync taskset systemd-run systemctl tail timeout bash`,
   usable cache and `--remote-dir` (default `/var/tmp`), free ports, compatible
   glibc. Record VPS CPU/RAM/kernel/load, busiest processes and live-unit state.
   Arrange an authorized live-unit stop separately to avoid contaminated numbers.
2. Build fixture locally once; per run rsync binary, fixture and `ops.json` to
   `/var/tmp/solaris-bench-<pid>-<time>/`. Rewrite config for game
   `0.0.0.0:25590`, dashboard `127.0.0.1:25591`; start one transient
   `systemd-run --remain-after-exit` unit with affinity and optional RAM/no-swap
   cap (`--user` for non-root).
3. Wait via `tail --pid` for dashboard readiness, not polling. Verify game
   reachability; `--tunnel-game` forwards it if necessary but costs VPS sshd CPU.
   Dashboard always uses SSH forwarding.
4. Local bots offer the normal schedule. Align server-clock ticks with median
   offset/RTT from nine SSH round trips, retained in `runs/*/run/remote.json`.
   Sample remote `/proc/<pid>/status` for `VmRSS`/`VmHWM`.
5. SIGTERM the unit for final checkpoint, record exit status, rsync the world
   back for cohort checks, stop/reset unit and remove its directory. `finally`
   also cleans up Ctrl-C/SIGTERM. After controller death, remove leftovers with
   `systemctl stop 'solaris-bench-*'; rm -rf /var/tmp/solaris-bench-*`
   (`--user` for a non-root unit).

Network latency affects sustainable bot work, recorded separately, not in-server
MSPT. Remote SMT topology is not probed: each vCPU is treated as a core, so remote
`physical` equals `logical`.

## Player-capacity ladder

`tools.coreperf.capacity_ladder` uses ordinary release server/protocol bots and a
fresh generated world per rung, not prepared fixtures. It reuses
[catalog-load](../development/catalog-load.md) session/config/login machinery.
It reports exact dashboard `/load-metrics` ticks, phase means, thread CPU,
network, checkpoint cost, memory and optional diagnostic perf; pass/fail means
only the stated operator thresholds.

```sh
# Nonincremental, non-PGO release pins first; --out must be new.
# Use the measurement lane/window required by current performance policy.
tools/dev/mem-run.sh -x -p -m 5G \
  python3 -m tools.coreperf.capacity_ladder \
  --server-bin /abs/krasnokamen --bots-bin /abs/player-scale-bots \
  --out /abs/new/capacity-run --workload build --view-distance 8 \
  --clients 20 50 100 --server-cpus 8-9 --bot-cpus 10-11
```

| Options | Defaults |
|---|---|
| `--workload`, `--view-distance`, `--clients` | `build` (`build\|fight`), 8, `20 50 100` |
| `--group-size`, `--spacing`, `--seed` | 5, 256 blocks, 42 |
| `--steady-seconds`, `--burst-seconds`, `--recovery-seconds` | 120, 60, 20 |
| `--readiness-seconds`, `--prepare-concurrency` | 1500, 3 |
| `--server-cpus`, `--bot-cpus` | `8-9`, `10-11` |
| `--vanilla-data-dir`, `--server-stop-seconds` | `data/vanilla`, 180 (loaded-world final save can exceed 10 s) |
| `--perf-seconds`, `--perf-delay-seconds` | 0, 20; diagnostic capture within steady window |

For a shared four-thread envelope use `--server-cpus 8-11 --bot-cpus 8-11`;
compare only equal lanes/envelopes. Each rung has offline auth, dashboard,
VD = simulation distance, operator bots for creative landing, and otherwise
default settings. Logins run three at a time in 5×5-block groups of five,
groups spaced 256 blocks: views overlap only partly. Demand is wall-clock
open-loop, not response-paced:

| Window | Movement/bot | Edits/bot, build / fight | Chat/bot |
|---|---|---|---|
| Steady, 120 s | 20 Hz | 2 / 1 Hz | 1 Hz |
| Burst, 60 s | 20 Hz | 5 / 2 Hz | 2 Hz |
| Recovery, 20 s | Readers only | None | None |

`build`: creative, fixed day 6000, 43–56-block walking legs with sprint/jump,
break/place/chat. `fight`: survival/respawn, fixed night 18000, natural hostiles
hurt walking bots; **bots do not attack**. The schedule's edit offers are not
proof of survival edits; [PLAYER_CAPACITY](PLAYER_CAPACITY.md) owns admission
and completion accounting.

Active `wall_us` excludes pacing sleep; TPS counts completed ticks over the
window. Defaults: sustained TPS ≥19.5 in 5 s bins, p99 ≤50 ms, max ≤250 ms,
server RSS ≤3072 MiB, no disconnects. Overrides: `--min-tps`,
`--sustained-seconds`, `--max-tick-p99-ms`, `--max-tick-max-ms`, `--max-rss-mb`.
A lost connection ends the window at its last completed tick (`partial`);
short partial windows are weak evidence.

Output: `<out>/summary.json`, `<out>/rung-N/result.json`, raw samples/logs.
Old runs are not current capacity evidence: measure the build being discussed.

## Smoke checks (pipeline proof, not numbers)

`--smoke`: 2 s warmup + 5 s observation, cheap `idle,explore-20` defaults,
scaling `explore-20` at 1/2 CPUs, worldgen radius 64. Smoke or windows shorter
than 30+120 s have `publication_evidence: false` and a SMOKE banner.

For `explore-20`, Ready requires Login/Configuration, teleport to the
256-block-spaced anchor, complete 17×17 received chunk view (VD 8), a walkable
runway, landing-position view and input-ordered preparation echo. Three bots
prepare concurrently on unpregenerated terrain: the 300 s smoke deadline bounds
thousands of cold chunks, not just handshakes. Never start demand before usable
views. A longer readiness override is diagnostic, not passage of that bound;
canonical non-smoke readiness is 1800 s.

Busy-host diagnosis: `MC_CORE_PHASE_LOG` exposes Linux `chunk admission cpu`
per admitted chunk and `chunk admission ms` batch counts. Compare equal
fixtures/CPUs; CPU excludes descheduling, wall/Ready time do not. Deterministic
operation-count regressions explain solver savings, not quiet integrated
readiness/TPS acceptance.

```sh
M=<work>/solaris/tools/dev/mem-run.sh
$M -m 8G python3 -m tools.bench weak-hw --smoke --slice solaris-heavy.slice
$M -m 8G python3 -m tools.bench scaling --smoke --cpu-mode logical,physical --slice solaris-heavy.slice
$M -m 8G python3 -m tools.bench worldgen --smoke --cores 1,2
```

Pure CPU-allocation/receipt/parser/remote-config tests:
`python3 -m unittest tools.bench.test_bench`.

## Reading a result

Publish `result.md`; `result.json` contains `runs[]` (weak/VPS),
`series.<mode>[]` (scaling), or `results[]` (worldgen). It records host
`lscpu`/CPU→core→socket map, RAM/kernel/OS/governor/load, source commit/binary
SHA, per-run cpuset/physical-core count and memory cap.
Per-run fields: `status` (`ok|failed|skipped`), `error`, `tps`,
`mspt_ms{p50,p95,p99,max}`, `server_rss_mib{steady_median,peak}`,
`cpus_server`, `physical_cores_touched`, full `receipt` path.
Failure retains receipts, exits nonzero and never presents partial numbers as success.

## Limits of these measurements

- One run/cell except worldgen `--repeat`; 120 s p99/max are not stable tails.
- Bots are not graphical clients; VPS adds real network latency.
- MemoryMax includes page cache; process RSS does not. Swap is off.
- Governor/background load are recorded, not controlled.
- Vanilla head-to-head compares unequal incidental engine work.
