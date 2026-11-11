# Operating Solaris

The server binary is `solaris-v2` (`mc-edge` over the native `mc-core`;
release packaging may name it `solaris`). This guide is the operator reference
for the current runtime. Each procedure has one owner page:

| Topic | Owner |
|---|---|
| Configuration keys and defaults | [`example.toml`](../example.toml) |
| Moving a vanilla or Paper world in, and rolling back | [MIGRATION.md](MIGRATION.md) |
| Plugin package layout, deployment and limits | [PLUGINS.md](PLUGINS.md) |
| Client-side Loader install | [SOLARIS_LOADER.md](SOLARIS_LOADER.md) |
| What vanilla behavior is covered | [COMPATIBILITY.md](COMPATIBILITY.md) |

Contents:
[Startup and content](#startup-and-content) ·
[Small hardware](#small-hardware) ·
[Configuration, network and security](#configuration-network-and-security) ·
[Administration](#administration) ·
[World operations and backup](#world-operations-and-backup) ·
[Observability](#observability)

## Startup and content

### Running the server

```sh
solaris-v2 --config server.toml
```

`--config` defaults to `server.toml`. `--no-console` disables stdin commands
(use it for services). Unknown config sections and keys warn and are ignored: a
warning is not proof that a legacy setting took effect. There is no `--check`
mode; startup validates the real deployment (content, world, plugins) and
refuses to serve when any part is invalid. Tracing goes to stderr (filter with
`RUST_LOG`); command feedback goes to stdout.

Offline subcommands (`content`, `world`, `backup`, `restore`) run without
starting the server and are described below. Those that open a world take its
exclusive lease and refuse a running world.

### Vanilla content cache

Solaris does not distribute Minecraft data. Derive a cache from your licensed
server bundles with JDK 25 (`JAVA`/`JAVAC` may select it). A cache joins two
releases: the **26.3** bundle supplies every protocol identity (block states,
items, entities, the per-state light/mining/explosion/map-colour reports and
the exact registry data the client receives); the **26.1.2** bundle supplies
the gameplay data pack (worldgen, loot, recipes, advancements, tags,
structures), whose JSON grammar Solaris's loaders read.

```sh
solaris-v2 content import --version 26.3 --download --cache /path/to/cache
# or from jars you already have:
solaris-v2 content import --from /path/to/26.3/server.jar \
  --gameplay-from /path/to/26.1.2/server.jar --cache /path/to/cache
solaris-v2 content verify --cache /path/to/cache
```

`--version` selects canonical `26.3`; the command also installs 26.1.2 and
26.2 endpoint caches and both adjacent mapping snapshots. A bundle you do not
supply is downloaded (verifying size and SHA-1); `--download` cannot be combined
with `--from` or `--gameplay-from`. Registry capture launches disposable vanilla
worlds solely for Configuration; it never touches your gameplay world.
The cache also holds 26.3 English language data for console and RCON feedback;
reimport a cache that lacks `assets/minecraft/lang/en_us.json`.

Each cache and step snapshot is staged, validated and published atomically;
failure preserves that artifact's previous copy. Import fails if any required
version cannot be installed. `verify` is offline and checks canonical gameplay
content, not the whole deployment. First start with no cache runs this same
window-wide importer before opening a world or listener. Existing caches are
reused offline; a broken existing canonical cache fails with an import instruction.

Selection order: `[data].vanilla_data_dir`, `SOLARIS_CONTENT_CACHE`,
`$XDG_DATA_HOME/solaris/content/26.3` (otherwise
`~/.local/share/solaris/content/26.3`), then `data/vanilla` in the working
directory and checkout ancestors. An explicit invalid override fails without
silently choosing another source. `--cache` affects only that offline command.

### Client protocol compatibility

Solaris serves **every Minecraft Java release from 26.1.2 through 26.3**:

| Client release | Protocol |
|---|---|
| 26.1.2 | 775 |
| 26.2 | 776 |
| 26.3 (canonical) | 777 |

`content import --download` and a fresh first start install the entire window;
no cache from an earlier installation is needed. With a canonical cache at
`.../content/26.3`, endpoint caches live at `.../content/26.1.2` and
`.../content/26.2`; independent mapping snapshots live at
`.../content/steps/776-775` and `.../content/steps/777-776`. For an arbitrary
explicit canonical path `/path/cache`, companions are `/path/cache-26.1.2`,
`/path/cache-26.2`, and `/path/cache-steps/<pair>`.

Startup loads every available profile and logs its release/protocol. A missing
or unusable **endpoint** cache disables only that client version. A missing
**step** snapshot disables all client routes crossing it, naming that pair in
the log; there is no direct nonadjacent bypass. Canonical clients are unaffected.
Rerun `content import --download` with the same canonical `--cache` to repair
missing artifacts. Valid older steps are retained across a re-import or bump.

Status advertises each available client's own release/protocol. Unsupported or
disabled Login versions receive vanilla's `multiplayer.disconnect.incompatible`;
unsupported Status requests advertise canonical so the list marks them incompatible.
A bump adds a step and retains every older release
([ADR 0015](decisions/0015-multiversion-protocol-translation.md)).

Each profile sends its own registries, tags and known pack. Missing features
use the nearest representation: for example, cinnabar is granite for 26.1.2
but remains cinnabar for 26.2. Dynamic registries and `item_model` with a
server resource pack may provide client-accepted data; blocks, entities and
mechanics cannot be added by datapacks. Unknown/malformed payloads fail closed.
This is protocol compatibility, not identical rendering or mechanics.

## Small hardware

Treat CPU and RAM budgets as measured workload envelopes, not minimum-hardware
guarantees. [The weak-hardware benchmark](performance/BENCHMARKS.md#weak-hw-the-6-vps-shape)
keeps dedicated-server budgets separate from the combined singleplayer budget:
server CPU affinity and its memory cap do not include the graphical client.
`explore-3` means three exploring players, not the twenty-player workload under
a different name.

### Singleplayer with software Vulkan

On 2026-11-07, fresh seed 42, installed 26.3 content, 1280×720 Low preset,
Vulkan lavapipe, and client plus server sharing four logical CPUs (`0-3`) and
an 8 GiB cgroup cap:

| Quiet release run | Play → usable world | Play → observed look input | Combined peak |
|---|---:|---:|---:|
| RC5 defaults, near 8 / far 1024 | 17.87 s | 18.17 s | 2.87 GiB |
| Automatic software-adapter defaults, near 4 / far 256 | 11.38 s | 11.52 s | 2.37 GiB |

Receipts under `.analysis/validation/`:
`20261107T125724-native-sp-weak-hardware-1ev8zgzc/result.json` and
`20261107T125848-native-sp-weak-hardware-jd4y97cu/result.json`.
Each has `singleplayer/scenario.json` with the actual-window readiness,
ordinary-input observation, process RSS and cgroup peak. Both runs closed the
loading overlay before control was measured. These are startup measurements;
the captured 124 ms candidate frame interval is not a sustained-FPS result.
They exclude first-time content installation and do not establish a physical
laptop's minimum RAM or integrated-GPU performance.

Software adapters use the existing background CPU far-terrain generator,
instead of placing software compute on the frame queue. An unconfigured
software adapter uses near distance 4 and far distance 256. Explicit valid
saved settings win; changing another option does not persist the automatically
selected distances as user choices. Hardware-adapter defaults are unchanged.

The choices were also measured one at a time in the same diagnostic `-p`
lane (four logical CPUs `8-11`, shared 8 GiB, lavapipe, same fresh world and
display settings). Each step retains the preceding step:

| Step | Play → usable world | Play → observed look input |
|---|---:|---:|
| RC5, software GPU far generation, near 8 / far 1024 | 27.80 s | 29.10 s |
| Route software far generation to the CPU | 18.81 s | 19.30 s |
| Then far 1024 → 256 | 17.18 s | 17.42 s |
| Then near 8 → 4 | 14.77 s | 14.99 s |

Diagnostic receipts under `.analysis/validation/`, each ending in
`/singleplayer/scenario.json`:
`20261107T121704-native-sp-weak-hardware-zzxm8lss`,
`20261107T122303-native-sp-weak-hardware-ahrtobcc`,
`20261107T122448-native-sp-weak-hardware-lm6cmhn2`, and
`20261107T122606-native-sp-weak-hardware-cn16cmcw`.
These single profiled samples select useful defaults; they are not quiet-lane
publication numbers or a statistical guarantee. The separate quiet pair above
measures the combined automatic policy with no saved-settings override.

Singleplayer startup listens for the local server's actual progress,
Listening, failure and process-exit events. Slow startup is not failure merely
because 180 seconds elapsed. The loading screen publishes each changed
startup status; Cancel remains available. World-entry readiness still requires
the normal terrain/light and GPU-completion fences, not an elapsed-time bypass.

## Configuration, network and security

### Supported configuration

[`example.toml`](../example.toml) documents every section and its starter
values: `[server]`, `[network]`, `[auth]`, `[proxy]`, `[dashboard]`,
`[properties]`, `[rcon]`, `[query]`, `[resource_pack]`, `[anti_xray]`, `[lod]`,
`[distant_horizons]`, `[mob_griefing]`, `[anti_cheat]`, `[plugins]`, `[data]`,
`[voice]` and `[computer]`. Keys it omits:

```toml
[server]
max_players = 20                 # concurrent Play sessions
enable_command_block = false     # command-block execution gate
# whitelist = true               # initial enforcement, see access control
[data]
# backup_dir = "/srv/solaris/backups"   # console `backup` destination
[block_log]                      # history behind /co
enabled = true
retention_days = 30              # 0 keeps records until max_records
max_records = 200000             # per dimension; oldest go first
[dashboard]
# metrics_bearer_token = "..."   # protects /metrics only
[msmp]                           # see "Minecraft Server Management Protocol"
enabled = false
bind_address = "127.0.0.1"
port = 25585
bearer_token = ""
allow_remote = false
```

A missing section takes the built-in defaults, which differ from the starter
file: loopback TCP 25565, world `world`, seed 0, view distance 8, simulation
distance 6, 20 players and **offline authentication**. Use explicit values.
Relative world, content and plugin paths resolve against the process working
directory, not the TOML directory; access-list files such as `ops.json` live
beside the selected config. The old `[admin]`, `[simulation]`, `[autoscale]`,
`[chunk_pipeline]`, worldgen-mode and per-package grant/roster/reload settings
are not supported; the key list in `crates/mc-edge/src/config.rs` is the
authority when in doubt.

Behavior worth knowing:

- `view_distance` is the server's upper bound; each connection uses the smaller
  of its requested distance and this limit (minimum two chunks) and changes it
  live when the player changes render distance. Residency is the union of
  player views, the simulation radius and a margin; tickets and dirty-chunk
  pins keep extra chunks loaded without sending them.
- `pause_when_empty_seconds` (vanilla's `pause-when-empty-seconds`): with no
  player online for that long the world stops advancing (mobs, random ticks,
  redstone, weather, daylight) while network, console and saves keep working.
  Entering the pause performs one checkpoint; the first join resumes ticking.
  Zero or negative never pauses.
- The native singleplayer client pauses the world while its menu is open with
  `solaris_pause on|off`. These need an online level-four operator and exactly
  one online player; a join or that operator's disconnect resumes the world.
  This differs from `/tick freeze` (below): world tick numbers do not advance.
- `spawn_protection` (default 16) protects a square of that radius around the
  Overworld world spawn from block edits by non-operators. It is inactive while
  `ops.json` is empty; `0` disables it.
- `[mob_griefing]` switches narrow the vanilla `mobGriefing` gamerule, which
  stays the master gate: a disabled blast still hurts entities and emits its
  client event, it only stops removing blocks. They are config, not gamerules.
- `/gamerule randomTickSpeed` is capped at 4096; saved larger values are
  clamped on load with a warning.
- `worldgen_gpu = true` needs a build with `--features gpu` and a hardware
  adapter; otherwise startup logs why and the CPU generates.

### Network bind and port

Clients connect to `network.bind_address:port`. `127.0.0.1` is local-only;
`0.0.0.0` listens on all IPv4 interfaces. Configure the firewall/NAT for the TCP
port independently: authentication does not change listener exposure. Offline
identities can be impersonated, operator names included; restrict offline
servers to trusted players or a correctly authenticated network boundary
(`auth.online_mode = true`, or a Velocity proxy).

Connection admission:

- `connection_throttle_ms` (minimum gap between accepted connections from one
  address) and `max_connections_per_ip` (concurrent cap), both default `0`
  (off); a refused connection closes before the handshake.
- Independently, at most eight pre-play connections per socket IP are accepted
  for direct clients (Play sessions do not count). This is disabled in Velocity
  mode, where the proxy owns connection control.
- When accepting exhausts file descriptors or socket buffers while sessions are
  open, the listener logs a warning and pauses admission until a session ends;
  the same failure with no open session, or any other listener error, stops the
  server. One aborted or reset pending connection is skipped.
- Handshake, Status and Login share one 30-second deadline. In Play the server
  sends a keepalive every 15 seconds and closes after 30 seconds without the
  matching echo.

#### Velocity proxies and transfers

```toml
[server]
accepts_transfers = false
[proxy]
mode = "velocity"  # or "none"
secret = "..."     # the proxy's forwarding.secret
```

With `mode = "velocity"` the listener trusts only Velocity modern forwarding
(`velocity:player_info`, one answer verified by HMAC-SHA256 against `secret`)
and adopts the forwarded UUID, name, skin and client address, so access lists,
`/ban-ip` and the dashboard see the real address. A direct client, wrong secret,
mismatched name or unknown forwarding version is disconnected during Login.
`auth.online_mode` must be `false` (the proxy authenticates; the config refuses
to load otherwise) and `secret` must be non-empty. The handshake hostname is not
an identity source: bind the listener to a private interface and expose only the
proxy.

`/transfer <hostname> [port] [players]` (level 3) sends the vanilla Transfer
packet (default port 25565). Incoming transfers are refused with
`multiplayer.disconnect.transfers_disabled` unless `accepts_transfers = true`.

### Packet and movement anti-cheat

Play packet ids in the implemented prefix (`0x00`–`0x44`) each have a token
bucket; all other ids share one overflow bucket, so an unknown id cannot renew a
burst or grow limiter storage (at most 70 buckets per connection). Defaults:
500 packets per second, burst 1000; an exceeded bucket disconnects with a clear
message. Set either packet value to `0` only when an upstream boundary owns this
protection.

Survival players who repeatedly report an unsupported position without
descending are corrected after `max_airtime_ticks` (default 80) and then
disconnected with "Flying is not enabled on this server". Creative/spectator
flight, elytra, levitation, slow falling, water, climbables, vehicles, upward
knockback and slime blocks reset or bypass the check. `0` disables it; do that
only when another authoritative movement check replaces it.

### Anti-xray

`[anti_xray] overworld` / `nether` (both default true; the End has no ores).
Engine mode 1 (Paper style): an ore whose six neighbours are all opaque solid
blocks is sent as the surrounding rock (`stone`, `deepslate`, `netherrack`); an
ore touching air, fluid, glass or another non-opaque block is sent as it is.
Exposing a hidden ore (mining) sends its real state at once. It hides ores only
from a client reading chunk data: cave- and tunnel-exposed ores stay visible, as
in vanilla, and chunks next to an unloaded chunk are not hidden on that side.

### Far terrain (LOD)

`[lod]` answers tile requests from the **native Solaris client** only (vanilla
clients never see it). A tile is 16×16 column samples spaced 2 to 64 blocks.
Stored chunks answer with their top surface block (one with collision or
holding water or lava, as vanilla's `MOTION_BLOCKING`; flowers, grass and thin
snow are skipped) as of the last stored image (later edits are not shown);
never-generated columns answer with the generator's
cave-free router surface, sampled on the server, so far terrain costs server CPU
in proportion to the never-generated area players look at. Requests beyond
`max_distance`, above `max_inflight_tiles` per connection or not fitting the
connection's queue are dropped (the client asks again); `workers` bounds the CPU
spent.

`share_terrain_keys = true` sends each native client the compiled far-terrain
program so it can generate unstored terrain on its GPU. The program contains
neither the level nor the positional seed, but the noise octaves derive from the
positional seed through a non-cryptographic generator that can be inverted;
neither step was shown infeasible. **Treat enabling it as publishing the world
seed to every native player.**

### Distant Horizons

`[distant_horizons]` enables an independent Java Play custom-payload service on
`distant_horizons:msg` (DH network protocol **16** only; `radius_chunks`
accepted range 1–128, in chunks). It is separate from native `[lod]`, and
enabling it never shares terrain keys. The initial endpoint is MC 26.1.2 +
DH 3.3.3; MC 26.2 / DH 3.3.3 and MC 26.3 / DH 3.3.4 need the corresponding
Minecraft presentation profile; protocol-16 agreement alone is not a client
compatibility pass. Client acceptance status is in
[COMPATIBILITY.md](COMPATIBILITY.md).

Terrain is sampled off-tick, without simulation residency or native writes, from
the latest committed chunks (resident, then committed-but-unflushed edits, then
the stored native image); the server never sends the seed, the density program or
seed-derived noise tables, and the same anti-xray policy as vanilla
presentation applies. Requests use the authoritative player position, dimension
and world border. Per player: 2 requests per second, 2 outstanding sections,
4 MiB pending DTO budget, 500 KB/s; globally 8 pending sections, a 64 MiB
encoded cache, 2000 KB/s, on the shared `[lod]` worker lane (never a second
pool). Coverage that was genuinely never stored is generated on that worker
through the normal load-or-generate path (so pregenerating avoids the cost);
corrupt stored coverage fails closed with an error and is never replaced by
generated terrain or air. Live edits are pushed to negotiated clients through a bounded
dirty queue (8192 leaves) that alternates with ordinary requests so building
cannot starve initial sync.

Rollout: stop the world, pregenerate the initial radius with
[`world pregenerate`](#offline-chunk-pregeneration) (block units, plus boundary
and light halo), then start the server; blobs build lazily from native data.
Clients without DH get no LOD service and ordinary play is unaffected.

Known client limitation: DH 3.3.3 may not propagate material-only block swaps
(unchanged geometry) to parent LODs, so the live-edit gate changes geometry and
material together; no synthetic version bits are sent to work around it. The
graphical gate is `python3 -m tools.harness run dh-gate --platform fabric|neoforge`;
its prerequisites, pinned artifacts and evidence rules are in
[`tools/harness/dh-artifacts.json`](../tools/harness/dh-artifacts.json) and
[real-client gates](development/real-client-gates.md).

### RCON and UDP Query

Both are disabled by default. `[rcon] enabled = true` requires a non-empty
`password`; `bind_address` defaults to loopback, independent of the game
listener, and `port` to `25575`. RCON sends its password and full level-four
commands **unencrypted**: use a trusted network or an SSH tunnel, never an
exposed Internet listener. `--no-console` disables stdin only, not an enabled
RCON. Authenticated commands take the same admin route as stdin, serialized
with it and with other RCON clients, and replies carry command feedback. The
console-only `backup` reports admission immediately and logs its result;
plugin-deferred feedback is not a synchronous completion receipt. Request
bodies are bounded to 4096 bytes, replies are split into 4096-byte frames, at
most 16 clients are active and stalled reads/writes time out after 30 seconds.

`[query] enabled = true` starts GameSpy4 UDP Query on `network.bind_address`
(`query.port`, default 25565, separate from TCP). Basic stat reports MOTD, map,
player counts and game address; full stat adds the version and **the player
roster**, cached for five seconds. Challenges are session-bound and expire in 60
seconds; requests are bounded to 128 per second. Bind failures refuse startup.

### Server-offered vanilla resource packs

`[resource_pack]`: `url` (HTTP(S) only; empty disables), optional `sha1` (40 hex
digits), `required` (default false), `prompt` (a JSON text-component string) and
optional `id` (UUID, derived from the URL when absent). The client downloads the
ZIP, so serve it from a client-reachable URL.

The edge offers the pack during Configuration and waits for the matching
terminal client status before Finish Configuration; accept/download progress is
not enough. For a required pack, decline, invalid URL, download/reload failure
or discard disconnects the client; optional failures still join. Unknown pack ids
and malformed statuses refuse the connection. Negotiation is bounded to 120
seconds and 32 packets. Required-pack policy trusts the client's acknowledgement,
not a proof of rendered assets, and is independent of Loader bundle negotiation.

### Authentication and chat

- Chat is **unsigned system chat** (`enforces_secure_chat = false`). Signed
  command and chat-session metadata is bounded and decoded, but signatures and
  certificates are not verified: online login authenticates the connection's
  identity, not each message.
- With an auth-capable plugin (for example `solaris-auth`) or
  `[auth] require_plugin_login = true`, a pending session gets an empty,
  temporary presentation at a fresh spawn rather than the saved inventory,
  vitals, position or operator advertisement; the saved record stays untouched
  until the plugin unlocks that exact session. Pending sessions cannot trigger
  pressure plates, travel dimensions or ask for LOD terrain, and are excluded
  from persistence and the authenticated roster. Deployment details are in
  [plugin operations](plugins/OPERATIONS.md#operator-deployment).
- Chat and native communication commands share a per-sender spam counter: 20
  points per input, one removed per tick, disconnect at 200 (vanilla's
  threshold). Operators share it; console commands are not charged.
- Player capacity is reserved atomically after Configuration and rechecked with
  the access lists before Join.

### Importing `server.properties`

TOML remains the canonical configuration. At startup Solaris reads
`server.properties` beside the selected `--config` TOML when present; select
another file with `[properties] file = "migration/server.properties"` (relative
to the TOML; an explicitly selected missing file is an error). The file is never
rewritten and is not a second live authority: **explicit TOML leaf → imported
supported property → Solaris default**. The reader follows Java `Properties`
rules (comments, `=`/`:`/whitespace separators, continuations, `\uXXXX`, Latin-1
bytes, last duplicate wins). Every unsupported key warns by name without printing
its value; invalid supported values fail startup.

| Vanilla property | Solaris TOML key |
| --- | --- |
| `motd` | `server.motd` |
| `max-players` | `server.max_players` |
| `view-distance`, `simulation-distance` | `server.view_distance`, `server.simulation_distance` |
| `accepts-transfers`, `pause-when-empty-seconds` | `server.accepts_transfers`, `server.pause_when_empty_seconds` |
| `white-list` | `server.whitelist` (initial persisted policy) |
| `enable-command-block` | `server.enable_command_block` |
| `spawn-protection` | `server.spawn_protection` |
| `server-ip`, `server-port` | `network.bind_address`, `network.port` |
| `online-mode` | `auth.online_mode` |
| `level-name`, `level-seed` | `data.world_dir`, `data.seed` |
| `enable-rcon`, `rcon.port`, `rcon.password` | `rcon.enabled`, `rcon.port`, `rcon.password` |
| `enable-query`, `query.port` | `query.enabled`, `query.port` |
| `resource-pack`, `resource-pack-sha1` | `resource_pack.url`, `resource_pack.sha1` |
| `require-resource-pack`, `resource-pack-prompt`, `resource-pack-id` | `resource_pack.required`, `resource_pack.prompt`, `resource_pack.id` |

An empty `server-ip` means `0.0.0.0`. Non-numeric seeds use Java's UTF-16 string
hash; an empty seed keeps Solaris's default rather than a random one. Importing
`level-name` does **not** import Anvil data: convert the stopped world
explicitly ([MIGRATION.md](MIGRATION.md)).

## Administration

### Server console and access control

Stdin lines, with or without a leading `/`, run at operator level four. The
console has no player entity or inventory: relative coordinates and nearest
selectors originate at world spawn, and executor-dependent commands need an
explicit player or selector. Translatable feedback renders in English on stdout
and RCON. Useful console commands: `help`, `list`, `op`, `deop`, `save-all`,
`backup` (see [hot backup](#hot-backup-of-a-running-server)) and `stop`.
`stop` and SIGINT/SIGTERM request a normal shutdown with a final durable
checkpoint.

Operators come from the vanilla `ops.json` beside the config. Access lists are
vanilla files beside the config: `ops.json`, `banned-players.json`,
`banned-ips.json` (temporary rows carry the vanilla `expires` stamp) and
`whitelist.json`, plus Solaris's own `muted-players.json`. Existing vanilla
files can be dropped in unchanged.

- `/whitelist on|off|add|remove|list|reload` and `/banlist players|ips` need
  level 3. Enforcement persists in `whitelist.enabled` (`true`/`false`) beside
  the list; `[server] whitelist` (or imported `white-list`) initializes it only
  when that file is absent, so a restart keeps console changes. Malformed state
  refuses startup rather than disabling admission. Bans win; operators bypass
  the whitelist as in vanilla, and the player cap only with `bypassesPlayerLimit`
  in `ops.json`. `reload` re-reads a hand-edited list; `add` resolves a live or
  already-known player, or an offline-mode name.
- `/kick`, `/ban` and `/pardon` accept selectors such as `@a`, which resolve only
  online players; literal profile names can also target known offline players
  (or offline-mode names). `/ban-ip` takes an IPv4 address or an online player
  name; `/pardon-ip` takes only an IPv4 address. `/ban` adds one optional token
  to vanilla's order: a duration (`30m`, `2h30m`, `1d`, `1w`); anything that does
  not parse as a duration is the first word of the reason.
- `/mute <player> [duration] [reason]` and `/unmute` stop ordinary chat and the
  native communication commands (`/me`, `/msg`, `/reply`, `/tell`, `/w`,
  `/teammsg`, `/tm`, `/say`, `/tellraw`, `/title`); other rights are unchanged.
  `/warn <player> [reason]` sends one private line and keeps no record.
- Selectors support type filters on `@s`, several score objectives in one
  `scores={...}`, repeated positive or negated NBT filters and empty `tag=` /
  `tag=!` tests; `/tag` updates the same retained entity data.

### Minecraft Server Management Protocol

Solaris exposes the vanilla 26.1.2 management-protocol subset as JSON-RPC 2.0
over a Bearer-authenticated WebSocket (`rpc.discover`, player list/kick,
allowlist, bans, operators, status, save, stop and system messages). It is off by
default and refuses startup without a `bearer_token` (see the `[msmp]` keys
above). The endpoint is plaintext: keep it on loopback, or terminate TLS at an
authenticated reverse proxy; `allow_remote` is required for non-loopback binds.

### Announcements, tick control and `/locate`

- `/tellraw`, `/title` and `/bossbar` (level 2) use vanilla syntax with SNBT
  components; `nbt` components are refused and targets may be in any dimension.
  Custom boss bars are one global registry, saved through the tick WAL and the
  checkpoint; vanilla's `custom_boss_events.dat` export is not wired.
- `/tick query|rate <1-10000>|freeze|unfreeze|step|sprint` (level 3). `freeze`
  holds the autonomous world (entities, random and scheduled block updates,
  redstone, fluids, spawning, weather, daylight) while players still move, build
  and chat. The WAL tick number keeps advancing, so absolute-tick deadlines fire
  on `unfreeze`. Rate, freeze and sprint state is never saved.
- `/locate structure|biome|poi` (level 2) runs on a worker thread and never
  blocks the tick; a request is refused while two are pending or eight queued.
  Structure searches read saved native or imported start metadata and plan
  uncovered chunks without generating them (villages within 100 chunks, other
  structures within 256); biomes are searched out to 6400 blocks and POIs within
  256 blocks of loaded chunks. Corrupt or unsupported start metadata fails closed.

### Datapacks and command blocks

Solaris loads **directory** packs from `<world_dir>/datapacks/<pack>/` at
startup. Each needs `pack.mcmeta` with a `pack` object. Packs are ordered by
directory name and the lexically later resource wins. ZIP files, symlinks,
worldgen or custom-dimension resources and unimplemented resource kinds fail
startup with an explicit error rather than falling back to built-in content.

Supported resources (singular 26.1.2 directory names):

- `data/<ns>/function/<path>.mcfunction`: native commands, blank lines and `#`
  comments; macros and a leading `/` are refused.
- `data/<ns>/tags/function/<path>.json`: `values`, nested `#tag`, optional
  `{ "id": "...", "required": false }` entries and `replace`. `minecraft:load`
  runs on installation/reload; `minecraft:tick` every simulation tick. Cycles and
  excessive expansion are rejected.
- `data/<ns>/recipe/<path>.json`: serializers the native recipe parser already
  supports; an existing id is replaced in place, new ids are appended.
- `data/<ns>/loot_table/blocks/<block>.json`: plain `minecraft:block` tables with
  one `minecraft:item` entry per pool, supported roll counts and
  `minecraft:set_count`; an empty table suppresses drops. Other conditions,
  entry types and loot domains are refused.

Level-2 commands: `/function <id|#tag>`, `/schedule function <id|#tag> <time>
[append|replace]`, `/schedule clear <id|#tag>`, `/reload` and
`/datapack list [available|enabled]`, `/datapack enable <pack> [first|last|before
<pack>|after <pack>]`, `/datapack disable <pack>` (`create` is refused; the
client command tree advertises only `list [enabled]`). Schedule times use ticks
or `t`/`s`/`d`; zero or negative delays are rejected. Schedules survive restart.
`/reload`, `enable` and `disable` compile off the tick thread and swap
functions and tags at a tick boundary. **Recipe and loot changes are
startup-only:** a candidate that changes them is refused whole and the old set is
kept, as is a candidate with load errors. Schedules and load/tick tags run from
the overworld source; functions keep the source dimension they were invoked from.

Execution is bounded: each root command or function invocation gets its own
`minecraft:max_command_sequence_length` budget shared by nested functions and
`execute` branches; `minecraft:max_command_forks` bounds the contexts one
modifier stage produces; recursion uses an explicit stack with a 64-frame bound
across dimension dispatch; at most 65536 due schedules run per tick; pack
compilation is limited to 16 MiB of text, 16384 entries and 32 nesting levels.
Excess work aborts with feedback. Nothing reads the filesystem on the tick path.

Command-block execution is off unless `[server] enable_command_block = true`;
then impulse, chain and repeating blocks run native commands. Placement needs
creative mode and operator permission; configure a placed block with
`/data merge block <x> <y> <z> {Command:"time add 1",auto:1b}`. Impulse blocks
fire on redstone rising edges, repeating blocks use the durable scheduled queue,
chains follow facing with bounded traversal and run once per tick, and
**conditional blocks** check the success count of the command block behind their
facing. Commands whose native owner reports no outcome count as unsuccessful for
that check. Relative coordinates resolve at the command block. The client editor
packets, comparator result output and command minecarts are not implemented.

Map-making command surface (full status in [COMPATIBILITY.md](COMPATIBILITY.md)):

- `send_command_feedback false` suppresses player-source success replies for
  the native command paths that honor it; failures remain visible and console
  success remains enabled. Command execution and ordinary chat are unchanged.

- `/data get entity|block` (level 2) reads native owners, including live
  inventory and timers, without editing state; paths accept dotted keys, indices
  and compound filters and must select one value. `/data merge block` writes only
  command-block `Command` and `auto`. Namespaced storage supports `get`, `merge`,
  `remove` and `modify` (world-global, journaled). Entity and block `/data modify`
  are not supported.
- `/summon` accepts the vanilla string `variant` field for cows, pigs and chickens
  (`minecraft:temperate`, `minecraft:cold`, `minecraft:warm`) and zombie nautiluses
  (`minecraft:temperate`, `minecraft:warm`). For example,
  `/summon cow ~ ~ ~ {variant:"minecraft:cold"}`. The species' registry validates
  the value before allocation; typed appearance metadata, the tick journal and
  vanilla checkpoint fields retain it. Painting variants keep their painting
  registry and holder codec. See the vanilla [cow](https://minecraft.wiki/w/Cow),
  [pig](https://minecraft.wiki/w/Pig), [chicken](https://minecraft.wiki/w/Chicken)
  and [zombie nautilus](https://minecraft.wiki/w/Zombie_Nautilus) entity data.
- `/clone <begin> <end> <dest> [replace|masked|filtered <block|#tag>]
  [normal|force|move]` copies loaded regions within the `max_block_modifications`
  gamerule limit. Block-entity copies use the command-data snapshot; live state
  held outside it (crafter, lectern, campfire) is not fully covered. `/place` is
  not implemented.
- `/execute` supports `as`, `at`, `positioned`, `rotated`, `facing`, `anchored`,
  `align`, `in`, `if|unless entity|block|score` and `if|unless blocks <a> <b>
  <dest> all|masked`, and `store result|success score` for commands that report
  an outcome (others refuse with "Command does not support execute store").
  Block conditions match ids and properties, not block tags or SNBT; general NBT
  predicates and non-score store destinations are not supported.
- `/playsound` delivers positional sounds; `/particle` supports parameterless
  particles only.

### Forced chunks and ender-pearl loaders

`/forceload add|remove|query` (level 2) is vanilla's command: block-column
coordinates, at most 256 chunks per command, per dimension (the console is the
overworld; `/execute in` selects another). A forced chunk is a persisted ticket
(restored after restart and written to Anvil `chunk_tickets.dat` on export) that
keeps the chunk plus a two-chunk ring loaded and the chunk plus a one-chunk ring
block-ticking with nobody nearby; entities simulate only in the central chunk
unless a player's simulation distance covers its neighbours.

A thrown ender pearl holds an `ender_pearl` ticket (same extents, 40-tick timeout
refreshed while it flies, never persisted) while its owner is online in any
dimension. An offline owner's pearl is parked in the checkpoint and does not fly
or age until the owner returns. One owner's pearls hold at most 32 distinct
ticket chunks; excess pearls stay frozen outside simulation. Bubble-column
stasis chambers are not supported (no bubble-column entity physics).

### Block log and rollback

A CoreProtect-style log records player, mob (`#creeper`, `#tnt`, ...), `#fire`
and plugin (`#plugin:<id>`) block changes with the previous block state and
block-entity contents, plus per-slot player container transactions. Natural
changes (fluids, growth, decay, redstone, pistons) and console/command edits are
not recorded. Operators (level 3):

- `/co inspect` (`/co i`) toggles the inspector: clicking a block prints its
  history without touching it.
- `/co lookup [u:<players>] [t:<time>] [r:<radius>] [page]` (`/co l`): newest
  first, ten per page; `u:` takes names or `#sources`, `t:` takes `30s`, `15m`,
  `1h30m`, `2d`, `1w`, `r:` is a square in x/z over all heights.
- `/co rollback u:<players> t:<time> [r:<radius>]` (`/co rb`) reverts matching
  changes newest first in one tick; `/co restore` (`/co rs`) re-applies rolled-back
  changes oldest first. `t:` is required; without `u:` the radius defaults to 10.

Rollback is one fenced native command: every cell of the selection must be
loaded or nothing changes, the count is bounded by `max_block_modifications`, and
a record whose block (or container slots) someone else changed afterwards is
skipped and counted, never overwritten. A rolled-back chest gets its recorded
contents back; items the griefer already took stay with them. Rolled-back
records stay listed `(rolled back)` and are skipped by a second rollback.
Commands and records are per dimension.

Records ride the tick WAL and live in the `block_log` table of
`native/state.redb`, so they survive restarts, hot backups and restores (Anvil
export does not carry them) and carry across game-version upgrades; a row naming
a block or item the new version lacks stops startup like any unsupported native
state. Retention follows `[block_log]`; `enabled = false` stops recording but
keeps existing history queryable until it expires.

### Plugins

`[plugins] directory = "plugins"` selects the package directory; omit it to run
without component packages. Package layout, manifest, capabilities, Loader
requirements, deployment rules and diagnostics are owned by
[PLUGINS.md](PLUGINS.md); the first-party packages live in
[solaris-default-plugins](https://github.com/kaiserproger/solaris-default-plugins).
Operator-visible facts: a configured directory that does not exist is created
empty and logged; a malformed package, duplicate id or command root, or failed
startup refuses the whole server start; deploying an auth-capable package locks
every new session until the package unlocks it, whatever `require_plugin_login`
says. Production builds do not need sibling plugin source or compile guests at
startup. Death-grave custody can be verified offline with
[`world probe`](#probing-a-stopped-world).

## World operations and backup

### Native worlds and Anvil interchange

Serving persists a native database (`native/state.redb`) and the tick WAL, not
Anvil region files. Player, world and plugin effects recover at the same tick
boundary; an acknowledged operation is not an fsync guarantee, so an unflushed
tail may roll back consistently after a crash. Corrupt or incompatible saved
state fails closed. Keep the same seed, content cache and deployment
configuration, back up before changing binaries or world identity, and never
bypass identity validation by hand-editing saved metadata. Anvil is a
compatibility format, not a live fallback or dual-write store.

Convert only stopped worlds; the destination must not exist, the source is left
unchanged and a receipt is written:

```sh
solaris-v2 --config server.toml world import-anvil \
  --source /srv/anvil-world --destination /srv/native-world --receipt /srv/import.json
solaris-v2 --config server.toml world export-anvil \
  --source /srv/native-world --destination /srv/exported-world --receipt /srv/export.json
```

The full procedure (preparing a Paper or vanilla source, what survives, receipt
review, rollback) is owned by [MIGRATION.md](MIGRATION.md). Facts specific to
the converter:

- Import accepts Java worlds through 26.1.2 and native 26.3 worlds
  (`DataVersion` **5023**); intermediate snapshot versions are refused.
- Ordinary exports stay **4790 / 26.1.2**. A record is promoted to **5023** only
  when it keeps a native 26.3 chunk origin or contains new component or named
  pot-template shapes; `level.dat` and the receipt then report `26.3` / `5023`,
  otherwise `26.1.2` / `4790`. A promoted export needs a 26.3 vanilla server.
- Native-only plugin and settlement state and the block log have no vanilla
  equivalent; Anvil is not a backup format for them. Use native backup/restore.
- Simulation tickets are not an offline generator (they advance simulation and
  persist ticket ownership); use [`world pregenerate`](#offline-chunk-pregeneration).
- The headless fixture gate is `python3 -m tools.harness.anvil_interchange
  --binary <solaris-v2> --jar <26.1.2 server-bundler.jar> --eula-file
  <already-accepted eula.txt> --cache <content> --output <new dir>`. It supplies
  no legal acceptance and claims no real-client coverage.

A worldgen revision bump refuses existing worlds (`identity mismatch`). To keep a
stopped world, back it up and run `solaris-v2 --config server.toml world
upgrade-worldgen`: it rewrites only the stored revision (refusing any other
difference), leaves generated chunks as they are and applies the new generator
to chunks generated afterwards, with a hard seam at the old explored border. See
[Village generation](VILLAGE_GENERATION.md#keeping-a-world-across-a-revision-bump).

#### Probing a stopped world

`world census` counts stored entities, villagers, chunks and settlements, and
`world probe` reads exact blocks, container slots, stored player inventories and
stored item entities, both read-only on a stopped world and writing a JSON
receipt that must lie outside the world directory:

```sh
solaris-v2 --config server.toml world census --source WORLD --receipt census.json
solaris-v2 --config server.toml world probe --source WORLD --receipt probe.json \
  --block X,Y,Z
```

A probe receipt is PASS only when every step completed; it is refused when the
journal holds complete batches after the checkpoint (start and stop once more to
fold them). The harness profile `graves` uses it end to end:
`python3 -m tools.harness run graves -- --plugins-dir DEPLOYED_PACK`.

### Offline chunk pregeneration

`world pregenerate` fills the native store with overworld terrain before players
connect. Stop the server first: it opens the world exactly like serving (same
plugin-applied registry, village plans and worldgen rules), holds the exclusive
lease and refuses a live world. A missing world directory is created fresh.

```sh
solaris-v2 --config server.toml world pregenerate --radius 512
solaris-v2 --config server.toml world pregenerate --radius 256 --center=-1024,2048
```

`--radius` is a block radius on both axes (`512` covers 65×65 chunks) and fails
closed above 4,194,304 chunks; `--center` defaults to the world spawn. Chunks
are written as immutable native baselines in batches of 64, using the same
workers and light bake as the live world, so a pregenerated chunk is what a join
would have generated. Stored chunks are skipped, so an interrupted run resumes
and a rerun only fills what is missing. Only the overworld is pregenerated; there
is no in-game pregeneration command.

### World size, pruning and compaction

Chunk baselines and side-state rows are stored zstd-compressed with a checksum (a
pregenerated overworld chunk costs about 3.7 KB of rows instead of 126 KB). Rows
from older builds stay readable and are rewritten compressed on the next save of
that chunk or by `world compact`. A row failing its length, frame or checksum
validation is refused like any corrupt native record.

These commands need the server stopped, take the exclusive lease and refuse a
running world. Back up first.

```sh
solaris-v2 --config server.toml world compact
solaris-v2 --config server.toml world prune --keep-radius 4096 --dry-run
solaris-v2 --config server.toml world prune --keep-radius 4096
solaris-v2 --config server.toml world prune --keep-radius 2048 --center=-1024,2048 --max-inhabited-ticks 600
```

`world prune` forgets overworld chunks outside `--keep-radius` blocks of
`--center` (default the world spawn) whose baseline the Solaris generator created
and that still equal it: no edited block, no persistent entity row, no side-state
difference, and an inhabited clock of at most `--max-inhabited-ticks` (default
`0`). Everything else is kept, including every chunk imported from Anvil (the
generator cannot recreate it) and every chunk stored before generator provenance
was recorded (such worlds prune nothing; the report counts them as "kept not
generator-made"). A pruned chunk is generated afresh with the current generator
when next visited, so after `upgrade-worldgen` that is new terrain. The command
first opens and closes the world once to fold the crash journal tail into the
store, then compacts and prints the file size before and after. The Nether and
the End are not pruned.

### Native backup and restore

Offline commands need the server stopped: they take the exclusive world lease,
lock the native database and refuse live worlds.

```sh
solaris-v2 backup --world-dir /srv/solaris/world --out /backups/world-2026-10-15
solaris-v2 restore --from /backups/world-2026-10-15 --world-dir /srv/solaris/restored
solaris-v2 --config restored-server.toml
```

A backup holds the native database, WAL/journal and other world files with a
versioned SHA-256 manifest (lease files excluded). Restore verifies the full
inventory and hashes before writing; the target must be empty or absent and
separate from the backup. Missing, changed or extra files, incompatible
manifests and symlinks are refused. Retain the TOML, plugin artifacts and access
lists separately; backups are never pruned automatically.

Database copies read through the file handle holding the lease, including on
Windows; they do not release the lock or reopen `state.redb` to copy it.
Anvil conversion and census receipts use `/`-separated world-relative paths on
every host. The Windows `.solaris-world.lock` file is coordination, not imported
world metadata or backup contents.

#### Hot backup of a running server

Set a destination and type `backup` in the server console (not available to
in-game players; refused while `backup_dir` is unset or `--no-console` is used):

```toml
[data]
backup_dir = "/srv/solaris/backups"
```

The server forces a native checkpoint, then its single database writer copies
`native/state.redb` and the tick-WAL batches appended since into
`backup_dir/backup-<UTC>-tick<N>/` with the same manifest as the offline command.
`N` is the last tick the copy recovers; players keep playing and no tick pauses,
but the next periodic checkpoint and generated-chunk saves wait until the copy
finishes (WAL appends continue and dirty state stays resident). The copy is
assembled in a hidden `.backup-<UTC>.partial` directory and renamed only after
the manifest is durable, so a failed backup leaves nothing behind. Restore with
`solaris-v2 restore --from <backup> --world-dir <new>`, or point `world_dir` at
a copy of the backup directory: opening replays the WAL to tick `N`, exactly as
crash recovery does. It covers world state only.

## Observability

### Dashboard and metrics

The dashboard is off by default, read-only and unauthenticated. It serves JSON at
`/stats`, `/tps` (rolling TPS plus p50/p95/p99 MSPT), `/profile` and
`/load-metrics`, and a Prometheus text scrape at `/metrics`.

```toml
[dashboard]
enabled = true
bind_address = "127.0.0.1"
port = 8080
allow_remote = false
metrics_bearer_token = "replace-with-a-secret"   # optional, /metrics only
```

The scrape includes TPS, tick p50/p95/p99, the latest per-phase timings, online
players, loaded chunks, entity count, resident memory, network bytes, reliable
drops and skipped pump ticks. A token protects only `/metrics`: keep the loopback
bind or put the JSON endpoints behind an authenticated reverse proxy. A
non-loopback address needs `allow_remote = true`; otherwise the dashboard does
not start and a warning is logged. Operators at level 2 can run `/tps` and
`/mspt` in game; both report the same rolling window and are observation only.

### Input log

`[server] input_log = true` (default off) writes, per executed tick, the inputs
that tick consumed to `<world_dir>/input-log/<start_tick>-<unix_ms>.jsonl`:
player inputs, synthetic leaves, debug spawns and remote chunk tickets as JSON
lines. A dedicated writer thread behind a bounded queue never blocks the tick; a
stalled disk drops records and leaves a `Gap` marker, and a crash can lose the
unflushed tail. The log contains what players sent, so **treat it as sensitive**:
`login`/`register`/`changepassword` commands and all text of a session the login
gate still holds locked are withheld, but text typed into an unlocked session
(chat, plugin commands under other names) is written as sent. Plugin commands,
wall-clock reads and live chunk-load timing are not inputs of this log; only the
test-only replayer consumes it today. Format: `crates/mc-core/src/input_log.rs`.
