# Public test server operations

The owner runs one public test server on a small VPS. Its address is never
written in the repository: scripts and commands take it from the environment
variable `KRASNOKAMEN_TEST_HOST` (SSH destination, user included); the owner holds
the value, SSH access and every credential. Server operation in general is
documented in [OPERATING.md](../OPERATING.md); this page is the
deployment routine for that host. Deploying needs owner authorization;
deploys are batched until the owner says deploy.

**Owner-only.** Everything here needs access only the owner holds:
`KRASNOKAMEN_TEST_HOST`, an SSH key for it, a host-side RCON helper and `/root` on
the host. A contributor without them can read the routine and rehearse it on a
local machine, but cannot run it. Never copy a real host address, SSH detail,
password or owner-specific receipt path into a tracked file; use
`KRASNOKAMEN_TEST_HOST` and placeholders. The scripts and their environment
variables are described in
[`tools/dev/test-server/README.md`](../../tools/dev/test-server/README.md).

## Host

- Debian 13, 2 vCPUs (AVX2), about 4 GB RAM, no swap, glibc 2.41 or newer,
  JDK 25. Use `ssh -o LogLevel=ERROR "$KRASNOKAMEN_TEST_HOST"` to silence
  post-quantum key warnings. The agent `ssh://` file route does not work with
  this host; use plain `ssh`.
- The checked-in sample [`solaris.service`](../../tools/dev/test-server/solaris.service)
  uses `User=krasnokamen`, `Group=krasnokamen` and `/opt/solaris` as its default
  server directory and `HOME`. The directory holds the binary, config, plugins,
  world and metrics. The sample runs with `--no-console`, `MemoryMax=3G` and
  `Restart=on-failure`. Verify the installed unit and overrides before relying
  on those values. A startup failure can loop restarts; stop the verified unit
  before debugging.
  The deployment has used a `RUST_BACKTRACE=1` drop-in; check that it is
  present if you need backtraces.
- Content cache: `/opt/solaris/.local/share/krasnokamen/content/<version>`.
  Use the installed unit's service user and `HOME`, including any overrides;
  the unit name is not the account name. For the checked-in sample:
  `runuser -u krasnokamen -- env HOME=/opt/solaris ./krasnokamen content import
  --config server.toml --version <ver> --download`. Verify with
  `content verify --config server.toml` as the same account (no `--version`).
- `server.toml`: `motd` names the RC (bump it with each deploy); view distance
  16, simulation distance 6; bind `0.0.0.0:25565`; `online_mode = false` by owner
  decision (the native client needs offline mode) with the `krasnokamen-auth` plugin
  and `[auth] require_plugin_login = true`; dashboard on `127.0.0.1:25580`, never
  public. No operator grants for bots.
- RCON listens on loopback only; run commands through the host's RCON helper
  over SSH. `ops.json` is read only at startup, so grant operator with the
  `op` command over RCON instead of editing the file.
- A static file server (Caddy, port 8080, root `/srv`) serves
  `/srv/static/`: the client content manifest `krasnokamen-content.json` and the
  bundles `krasnokamen-content-<ver>-r<N>.tar.zst` (see
  [releases](release-and-tags.md#client-release-and-the-singleplayer-content-bundle)).

## Redeploy, keeping the world

1. Build locally with `cargo build --release --locked -p mc-edge --bin krasnokamen`,
   and check the glibc requirement:
   `objdump -T <bin> | grep -oE "GLIBC_[0-9.]+" | sort -Vu | tail -1`.
2. Smoke locally **with the exact plugin set you will deploy**: a temp dir with
   the server's `server.toml` changed to a private port, a private dashboard
   port, bind `127.0.0.1` and a copy of the plugins; run as a supervised service
   ready on the port and `is listening`; require the expected
   `loaded_packages=N`. A core change of the plugin ABI makes old components
   fail with `failed to convert function to given type`.
3. Keep a copy of the deployed binary named by SHA under ignored
   `.analysis/deploy-test-server/`.
4. Upload as `krasnokamen.next`, then in **one** SSH command on the host: flush
   metrics (`systemctl start solaris-metrics.service`), stop the unit, back up
   the world (`tar czf /root/world-backup-<stamp>.tgz world`), keep the previous
   binary as `krasnokamen.prev`, move `.next` into place, `chown krasnokamen:`, bump
   the MOTD, start, and wait for readiness on the new process only:
   `timeout 400 journalctl -u solaris -f -n 0 -o cat | grep -m1 -E
   "is listening|panic|failed to start"`. Plugins swap the same way
   (`plugins.prev`, extract into `plugins.next`, swap).
5. Verify from outside with a status ping (handshake with the current protocol,
   next state 1): version and description; the plugin count in the log. With
   `krasnokamen-auth` deployed, the plugin repository's `tools/auth_live_check.py`
   helpers drive a raw-protocol probe that expects the register prompt and then
   a successful login (it needs an absolute `--output-dir` and an
   `XDG_DATA_HOME` containing a content cache).
6. On failure restore `krasnokamen.prev` immediately and restart, then fix
   forward.

Cold start with a fresh plugin JIT cache takes about 80 s; warm restarts take
2-3 s. A restart kills all sessions: tell agents with attached clients to
relaunch.

## Fresh-world reset

**Do not run the checked-in reset helper.** The
[`deploy-reset.sh`](../../tools/dev/test-server/deploy-reset.sh) inspected at
`84116c517df0` stops and starts `solaris.service` but checks whether
`krasnokamen.service` is active. If that other unit is missing or inactive,
the check does not stop the script. It then creates and lists a tar archive
and runs `rm -rf world` without verifying the unit it stopped is inactive.
Listing the archive does not prove a quiescent, restorable backup.

The helper also defaults `KRASNOKAMEN_SERVER_USER` to `solaris` for `chown`,
while the checked-in `solaris.service` runs as `User=krasnokamen` and
`Group=krasnokamen`. This can fail or leave the wrong ownership after world
deletion. It does **not** remove `metrics/.cursor`; the collector tracks the
server instance in that cursor. These are source observations, not a claim
about the unit installed on the VPS.

A fresh-world reset wipes every `krasnokamen-auth` account. Do not proceed on
owner approval alone: require a corrected helper, explicit approval for the
exact reset, verified installed unit/account/world identity and a verified
restorable backup. Release upgrade and `WORLDGEN_REVISION` guidance is in
[releases](release-and-tags.md#core-release-cut); use the actual service user.

## Pitfalls

- Never combine `set -e` with a `| grep -v` filter whose output may be empty:
  the filter exits 1.
- During restart the old process logs `ERROR` lines ("journal work channel
  closed") and `journalctl --since` has one-second granularity. Match only
  `is listening`, `failed to start` and `panic`, and follow from `-n 0`.
- Default journal output truncates lines: use
  `journalctl -o cat --no-pager | sed 's/\x1b\[[0-9;]*m//g'`.
- Before restarting a server with players, confirm 0 players via the
  dashboard `/stats`.

## Metrics (no perf on the host)

- The dashboard exposes `/stats` (TPS, tick percentiles, entities, chunks,
  players, plugin), `/load-metrics?after_tick=N` (a ring of 16384 per-tick
  samples with total and phase microseconds; `instance_started_unix_ns` changes
  on restart) and `/profile`.
- [`tools/dev/test-server/collect-metrics.py`](../../tools/dev/test-server/collect-metrics.py)
  runs from `solaris-metrics.timer` every 5 minutes and folds only **closed**
  UTC minutes into `/opt/solaris/metrics/YYYY-MM-DD.jsonl` (p50/p95/p99/max,
  phase mean/max, slowest ticks, a `/stats` line). Its cursor stores
  `<instance> <last emitted tick>`. Emitting the open minute and advancing the
  cursor produced duplicate partial rows. The ring covers about 13 minutes, so
  the timer interval must stay below that. Redeploy the collector together with
  binaries that add phases.
- A local user timer pulls the files daily into ignored
  `.analysis/testserver-metrics/`. Phase metrics are not function-level; use a
  profiler locally on a specific slow phase.

## Clients and bots against the server

- Agents use the native client headless
  (`play --headless --profile offline-test --server "${KRASNOKAMEN_TEST_HOST#*@}:25565" --username
  <unique, <=16> --automation --mcp-http 127.0.0.1:0 --artifact-dir <dir>`):
  no Xvfb or GPU, about 2% CPU and 170 MB idle. Windowed only for captures, on a
  private Xvfb display. Launch as a long-lived service, not `&` in a one-shot
  shell; one client per agent.
- Fifteen swarm players on the 2-vCPU host held TPS 20 with p99 at most 21 ms
  (2026-10).
- **No bots on the live world** (2026-10-24, newer than the 24-hour soak
  practice of 2026-10-23). The soak driver
  [`tools/dev/test-server/soak.py`](../../tools/dev/test-server/soak.py)
  remains for owner-approved runs: start `player-scale-bots`, send
  `{"event":"run"}` on `ready`, `{"event":"stop"}` on `schedule_complete`, wait
  for `final`; any `failure` ends the soak (a restart loop is reconnect load,
  not a soak). Bots older than the auth/obstructed-column fix get kicked by
  anti-fly; unassisted survival bots never edit blocks. One bot's TCP reset ends
  the whole bot process: tell server failure (journal `panic`, `NRestarts`,
  `ActiveEnterTimestamp`) from network (`Connection reset by peer`). Match
  readiness events exactly (`"event":"ready"` also matches inside
  `ready_clients`).
- The workstation cannot reach minecraft.wiki; reference files can be fetched
  through this host (see [real-client gates](real-client-gates.md)).
