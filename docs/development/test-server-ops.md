# Public test server operations

The owner runs one public test server on a small VPS. Its address is never
written in the repository: scripts and commands take it from the environment
variable `SOLARIS_TEST_HOST` (SSH destination, user included); the owner holds
the value, SSH access and every credential. Server operation in general is
documented in [OPERATING.md](../OPERATING.md); this page is the
deployment routine for that host. Deploying needs owner authorization and the
advisor's `[approved <ID>]` (see
[review committee and breaker](review-committee-and-breaker.md)); deploys are
batched until the owner says deploy.

## Host

- Debian 13, 2 vCPUs (AVX2), about 4 GB RAM, no swap, glibc 2.41 or newer,
  JDK 25. Use `ssh -o LogLevel=ERROR "$SOLARIS_TEST_HOST"` to silence
  post-quantum key warnings. The agent `ssh://` file route does not work with
  this host; use plain `ssh`.
- `/opt/solaris/` is owned by the system user `solaris` (`HOME=/opt/solaris`):
  `solaris-v2`, `server.toml`, `plugins/`, `world/`, `metrics/`, the metrics
  collector. Unit: [`tools/dev/test-server/solaris.service`](../../tools/dev/test-server/solaris.service)
  (`--no-console`, `MemoryMax=3G`, `Restart=on-failure`), plus a drop-in setting
  `RUST_BACKTRACE=1`. A startup failure loops restarts: stop the unit before
  debugging.
- Content cache: `/opt/solaris/.local/share/solaris/content/<version>`.
  Import as the service user:
  `runuser -u solaris -- env HOME=/opt/solaris ./solaris-v2 content import
  --config server.toml --version <ver> --download` (about 80 s); verify with
  `content verify --config server.toml` (no `--version`).
- `server.toml`: `motd` names the RC (bump it with each deploy); view distance
  16, simulation distance 6; bind `0.0.0.0:25565`; `online_mode = false` by owner
  decision (the native client needs offline mode) with the `solaris-auth` plugin
  and `[auth] require_plugin_login = true`; dashboard on `127.0.0.1:25580`, never
  public. No operator grants for bots.
- RCON listens on loopback only; run commands through the host's RCON helper
  over SSH. `ops.json` is read only at startup, so grant operator with the
  `op` command over RCON instead of editing the file.
- A static file server (Caddy, port 8080, root `/srv`) serves
  `/srv/static/`: the client content manifest `solaris-content.json` and the
  bundles `solaris-content-<ver>-r<N>.tar.zst` (see
  [releases](release-and-tags.md#client-release-and-the-singleplayer-content-bundle)).

## Redeploy, keeping the world

1. Build locally with the release command from AGENTS.md (`--locked`), and
   check the glibc requirement:
   `objdump -T <bin> | grep -oE "GLIBC_[0-9.]+" | sort -Vu | tail -1`.
2. Smoke locally **with the exact plugin set you will deploy**: a temp dir with
   the server's `server.toml` changed to a private port, a private dashboard
   port, bind `127.0.0.1` and a copy of the plugins; run as a supervised service
   ready on the port and `is listening`; require the expected
   `loaded_packages=N`. A core change of the plugin ABI makes old components
   fail with `failed to convert function to given type`.
3. Keep a copy of the deployed binary named by SHA under ignored
   `.analysis/deploy-test-server/`.
4. Upload as `solaris-v2.next`, then in **one** SSH command on the host: flush
   metrics (`systemctl start solaris-metrics.service`), stop the unit, back up
   the world (`tar czf /root/world-backup-<stamp>.tgz world`), keep the previous
   binary as `solaris-v2.prev`, move `.next` into place, `chown solaris:`, bump
   the MOTD, start, and wait for readiness on the new process only:
   `timeout 400 journalctl -u solaris -f -n 0 -o cat | grep -m1 -E
   "is listening|panic|failed to start"`. Plugins swap the same way
   (`plugins.prev`, extract into `plugins.next`, swap).
5. Verify from outside with a status ping (handshake with the current protocol,
   next state 1): version and description; the plugin count in the log. With
   `solaris-auth` deployed, the plugin repository's `tools/auth_live_check.py`
   helpers drive a raw-protocol probe that expects the register prompt and then
   a successful login (it needs an absolute `--output-dir` and an
   `XDG_DATA_HOME` containing a content cache).
6. On failure restore `solaris-v2.prev` immediately and restart, then fix
   forward.

Cold start with a fresh plugin JIT cache takes about 80 s; warm restarts take
2-3 s. A restart kills all sessions: tell agents with attached clients to
relaunch.

## Fresh-world reset

Only with explicit owner approval, and only after a verified backup.
[`tools/dev/test-server/deploy-reset.sh`](../../tools/dev/test-server/deploy-reset.sh)
runs on the host with `set -euo pipefail`: stop, assert inactive, back up and
`tar tzf` the backup, remove the world, swap binaries, start, wait for
readiness. Also remove `metrics/.cursor`. A fresh world wipes every
`solaris-auth` account.

Release-to-release upgrades (new required content files, `WORLDGEN_REVISION`
bumps) are in [releases](release-and-tags.md#core-release-cut) step 4; on this
host run `world upgrade-worldgen` as the service user.

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
  (`play --headless --profile offline-test --server "${SOLARIS_TEST_HOST#*@}:25565" --username
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
