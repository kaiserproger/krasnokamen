# Migrating a vanilla or Paper server

Krasnokamen 1.0 is a **release candidate (prerelease)**, not a drop-in Paper
replacement. This guide moves a stopped, ordinary Minecraft Java world (26.1.2,
or a native 26.3 world) into Krasnokamen's native store and describes how to convert
it back. Test on copies before changing the public endpoint. A successful
conversion receipt proves conversion, not complete gameplay compatibility:
consult [COMPATIBILITY.md](COMPATIBILITY.md). Everyday commands and
configuration are in [OPERATING.md](OPERATING.md); this page owns the migration
procedure and rollback.

Recheck every command against the binary you deploy (`krasnokamen --help`,
`content import --help`, `world import-anvil --help`, `world export-anvil
--help`); release packaging may name it `krasnokamen`.

## 1. Prerequisites and a recoverable source

- Use one `krasnokamen` build throughout the rehearsal.
- Install **JDK 25** (`java` and `javac`) for the one-time content extraction.
  Have the licensed official **26.3 and 26.1.2 server bundles** (not a Paper
  launcher JAR), or let `content import` download them. For the vanilla rollback
  rehearsal, have the bundle of the release your export reports (usually
  26.1.2). Review Minecraft's license/EULA yourself; this guide does not accept
  terms.
- Reserve disk space for the unchanged backup, a staging copy, the native world,
  an Anvil export and conversion staging; there is no safe fixed ratio. Never use
  a live world's directory as a destination or run two writers on it.
- Record the actual numeric world seed (`seed` in the old console), game version,
  enabled datapacks, dimension list, game rules, difficulty, spawn, border,
  authentication/proxy mode and plugin versions. An empty or textual
  `level-seed` is **not** the numeric seed of an existing world.
- Run `save-all flush`, then `stop` on vanilla/Paper and wait for the process to
  exit. Back up the **entire server installation** (every world, config, access
  list, plugin JAR/config/database, resource pack and player files), keep that
  backup read-only, record checksums and rehearse restoring it.

Examples use `/srv/migration/anvil-source` for a disposable stopped source copy,
`/srv/krasnokamen` for the new installation and seed `42`; substitute your paths and
**actual seed**. Do not pre-create the native or export destination (the
converter requires that it not exist); create their parents and the receipts
directory.

### Paper layout and restrictions

Do not point import at the installation root. It expects one world root
containing `level.dat` plus its dimension, entity and player stores. Vanilla
26.1.2 uses `dimensions/minecraft/{overworld,the_nether,the_end}` and
`players/{data,advancements,stats}`; older layouts (`region`, `entities`, `DIM-1`,
`DIM1`, `playerdata`, `advancements`, `stats`) have explicit conversion mappings,
export uses the modern layout, and mixed old/new active locations are rejected.
Symlinks and non-regular entries in the source refuse conversion.

Paper may keep the Nether/End in sibling world directories and plugins may add
other worlds or files. There is **no Paper installation importer**, multiworld
merger or automatic dimension discovery. On a staging copy, use the Paper
version's documented return-to-vanilla procedure to assemble one vanilla world,
open and stop it with a vanilla server first, and verify all three dimensions
before importing. Never choose between two `level.dat` files or overwrite
duplicate saved-data paths.

Only the three standard dimensions with the normal vanilla noise-generator
settings and biome presets are admitted. Superflat, amplified or custom
generators and custom dimensions are not a supported migration route; do not
rewrite their worldgen metadata to pass validation.

## 2. Import the licensed content cache

Content import is **not world import**. It extracts the registries and game data
that both conversion and serving need. `--version` accepts only `26.3`; the cache
pairs the 26.3 protocol identity with the 26.1.2 gameplay pack:

```sh
krasnokamen content import --version 26.3 --download --cache /srv/krasnokamen/content/26.3
# or: --from /path/to/26.3/server.jar --gameplay-from /path/to/26.1.2/server.jar
krasnokamen content verify --cache /srv/krasnokamen/content/26.3
```

Set the same absolute `[data].vanilla_data_dir` in the conversion and serving
config; `--cache` applies only to its offline command. Selection order, failure
behavior and the optional separate 26.1.2 cache for legacy clients are in
[OPERATING.md](OPERATING.md#vanilla-content-cache).

## 3. Translate configuration explicitly

Krasnokamen does **not** read `bukkit.yml`, `spigot.yml`, `paper-global.yml` or
per-world Paper configuration, and reads only a subset of `server.properties`
([import rules](OPERATING.md#importing-serverproperties)). Unknown TOML keys warn
and are ignored; a successful start does not prove an invented mapping worked.
Start from [example.toml](../example.toml), not a mechanical hyphen-to-underscore
rewrite.

A minimal isolated rehearsal configuration at `/srv/krasnokamen/server.toml`:

```toml
[server]
name = "Migration rehearsal"
motd = "Private migration test"
max_players = 20
view_distance = 8
simulation_distance = 6

[network]
bind_address = "127.0.0.1"
port = 25566

[auth]
online_mode = true

[data]
world_dir = "/srv/krasnokamen/native-world"
seed = 42 # MUST equal the saved world's actual numeric seed
vanilla_data_dir = "/srv/krasnokamen/content/26.3"
```

Preserve the source authentication/UUID scheme. Do not switch an online server to
offline mode to make login easier: inventories, ops, whitelist and bans are
UUID-keyed. For a Velocity proxy configure `[proxy]` and firewall the backend
([OPERATING.md](OPERATING.md#velocity-proxies-and-transfers)); an offline backend
with no trusted forwarding is not equivalent. Relative paths resolve against the
**process working directory**, not the TOML directory; access-list files live
beside the selected TOML.

### `server.properties` mapping

Each row describes the **property**, not the corresponding game mechanic:

- **supported**: imported from `server.properties` beside the TOML (or set the
  stated TOML key directly).
- **different**: configure another surface or account for a semantic difference;
  the property itself is not read.
- **ignored**: no equivalent operator setting; it warns by name and is dropped.

| Vanilla key | Status | Krasnokamen mapping / migration action |
|---|---|---|
| `accepts-transfers` | supported | `[server].accepts_transfers` |
| `allow-flight` | different | No direct flag. `[anti_cheat].max_airtime_ticks = 0` disables the native airtime check, not all movement validation; test your flight use case. |
| `broadcast-console-to-ops` | ignored | No matching switch. |
| `broadcast-rcon-to-ops` | ignored | No matching switch; RCON is `[rcon]`. |
| `bug-report-link` | ignored | No matching switch. |
| `difficulty` | different | Imported saved difficulty and the native `difficulty` command; no TOML key. |
| `enable-code-of-conduct` | ignored | No equivalent setting. |
| `enable-jmx-monitoring` | ignored | No JVM/JMX; use the [dashboard and metrics](OPERATING.md#dashboard-and-metrics). |
| `enable-query`, `query.port` | supported | `[query].enabled`, `[query].port` (UDP, off by default). |
| `enable-rcon`, `rcon.port`, `rcon.password` | supported | `[rcon].enabled`, `.port`, `.password`; plaintext, see [RCON](OPERATING.md#rcon-and-udp-query). |
| `enable-status` | ignored | No status toggle. |
| `enforce-secure-profile` | ignored | No matching policy; chat is unsigned and online authentication is not signed-chat enforcement. |
| `enforce-whitelist` | different | Login whitelist uses `whitelist.enabled`; do not assume vanilla's online-player recheck. |
| `entity-broadcast-range-percentage` | ignored | No percentage knob. |
| `force-gamemode` | ignored | No forced-login gamemode. |
| `function-permission-level` | ignored | Directory-datapack functions run with the invoking source's permission; no config knob. |
| `gamemode` | different | No TOML key. A converted world's `level.dat` `Data.GameType` (0 to 3; absent means survival; any other value fails the conversion) becomes the mode a player without a saved record joins in. `/defaultgamemode` changes it, it persists (journal and checkpoint) and export writes it back to `GameType`. Worlds converted before this change have no stored value and export `GameType` 0. Saved players keep their own gamemode. |
| `generate-structures` | ignored | No toggle; saved terrain is imported, future terrain uses Krasnokamen generation. |
| `generator-settings`, `level-type` | different | Import validates saved normal-world descriptors; custom or flat/amplified generators are rejected. |
| `hardcore` | ignored | No hardcore deployment setting; retained metadata does not prove hardcore semantics. |
| `hide-online-players` | ignored | No status-list privacy setting. |
| `initial-disabled-packs`, `initial-enabled-packs` | ignored | Pack selection is directory order plus `/datapack`; see [datapacks](OPERATING.md#datapacks-and-command-blocks). |
| `level-name` | different | `[data].world_dir` is the **new native** directory, not the Anvil source; `[server].name` is a display name. |
| `level-seed` | different | `[data].seed` is a signed integer matching the saved world; resolve blank/text seeds on the old server. |
| `log-ips` | ignored | No logging/privacy toggle; review log handling separately. |
| `management-server-enabled`, `-host`, `-port`, `-secret` | different | `[msmp].enabled`, `.bind_address` (remote needs `allow_remote`), `.port` (25585), `.bearer_token`; review the [method/auth surface](OPERATING.md#minecraft-server-management-protocol). |
| `management-server-allowed-origins`, `management-server-tls-*` | ignored | No origins or TLS settings in `[msmp]`; never expose the plaintext endpoint as if it inherited TLS. |
| `max-chained-neighbor-updates` | ignored | No knob. |
| `max-players` | supported | `[server].max_players` |
| `max-tick-time` | ignored | No watchdog property; supervise the service externally. |
| `max-world-size` | different | No cap setting; inspect the imported border and native `worldborder`. |
| `motd` | supported | `[server].motd` (a TOML string, not Java `.properties` escaping, when set in TOML). |
| `network-compression-threshold`, `use-native-transport`, `region-file-compression` | ignored | No operator keys (native transport; native store + WAL, no Anvil compression selector). |
| `online-mode` | supported | `[auth].online_mode`; the built-in default is **false**, so set it explicitly. |
| `op-permission-level` | different | Preserve per-user `level` in `ops.json`; no global default. |
| `pause-when-empty-seconds` | supported | `[server].pause_when_empty_seconds`; zero or negative disables the pause. |
| `player-idle-timeout`, `status-heartbeat-interval`, `text-filtering-*` | ignored | No matching keys. |
| `prevent-proxy-connections` | ignored | Not mapped to Velocity trust; configure `[proxy]` explicitly. |
| `rate-limit` | different | `[anti_cheat].packet_rate_per_second` / `packet_burst` are per-Play-packet-id buckets, not the same algorithm. |
| `require-resource-pack`, `resource-pack`, `-id`, `-prompt`, `-sha1` | supported | `[resource_pack].required`, `.url`, `.id`, `.prompt`, `.sha1`; see [resource packs](OPERATING.md#server-offered-vanilla-resource-packs). |
| `server-ip`, `server-port` | supported | `[network].bind_address`, `[network].port`; choose an explicit address rather than copying an empty value. |
| `simulation-distance`, `view-distance` | supported | `[server].simulation_distance`; `[server].view_distance` is an upper bound capped by each client's request. |
| `spawn-protection` | supported | `[server].spawn_protection`: a square radius around the overworld spawn, inactive while `ops.json` is empty; operators bypass it. |
| `sync-chunk-writes` | different | Native WAL/checkpoints control durability; not a synchronous-Anvil switch. |
| `white-list` | different | Imported only as the initial `[server].whitelist` policy; copy `whitelist.json` and set `whitelist.enabled` beside the TOML or run `whitelist on`. |
| `allow-nether`, `pvp`, `spawn-animals`, `spawn-monsters`, `spawn-npcs` (older/Paper) | ignored | No same-named toggles; review native game rules and compatibility. |
| `enable-command-block` (older/Paper) | supported | `[server].enable_command_block`; see [command blocks](OPERATING.md#datapacks-and-command-blocks). |

Paper-specific anti-xray, packet limiting and connection throttling must be
translated intentionally: Krasnokamen exposes `[anti_xray]`, `[anti_cheat]` and
`[network].connection_throttle_ms` / `max_connections_per_ip`, not Paper YAML
(see [OPERATING.md](OPERATING.md#configuration-network-and-security)).

## 4. Move access lists, not identity assumptions

With Krasnokamen stopped, copy these files from the old **server root** to
`/srv/krasnokamen/`, beside `server.toml` (not into `native-world`):

| File | Handling |
|---|---|
| `ops.json` | Vanilla UUID/name/level/`bypassesPlayerLimit` rows are read directly. Check every level and bypass flag. |
| `whitelist.json` | Vanilla UUID/name array, read directly. Copying it does **not** enable enforcement. |
| `banned-players.json` | Vanilla player bans, including reasons and expiration, read directly. |
| `banned-ips.json` | Vanilla IP bans, including expiration, read directly; verify forwarded-address policy behind a proxy. |
| `usercache.json` | Not read. Keep it in the backup for rollback; it does not let Krasnokamen resolve those names. |

If the old server enforced a whitelist, create `/srv/krasnokamen/whitelist.enabled`
containing `true` **before** first admission, or run console `whitelist on`
before exposing the port. Operators bypass the whitelist, as in vanilla; bans
still win. `whitelist reload` re-reads a hand-edited list; a missing list loads
empty and malformed JSON fails startup. Preserve the actual UUID rows and test
commands against a joined or already-known player rather than assuming an
arbitrary offline name resolves. Keep the authentication mode and proxy-forwarded
UUIDs consistent with the source.

## 5. Convert the stopped world

```sh
krasnokamen --config /srv/krasnokamen/server.toml world import-anvil \
  --source /srv/migration/anvil-source \
  --destination /srv/krasnokamen/native-world \
  --receipt /srv/migration/receipts/import.json
```

There is no `--dry-run`: a rehearsal is a **real import into a new disposable
destination**. The receipt's parent must exist and the receipt must lie outside
both source and destination. Conversion refuses live worlds, checks
seed/content/layout compatibility and preserves the source. Do not work around a
refusal by removing version tags, locks or identity metadata; resolve it on the
staging copy or stay on the original server. Java 26.3 worlds (`DataVersion`
5023) are accepted in addition to worlds through 26.1.2; intermediate snapshot
versions are refused.

Read the receipt, not just the exit code: require `status: "PASS"`, then review
`counts`, `dimensions`, `normalizations`, `upgrades`, `known_gaps` and
`opaque_retention`. `PASS` may coexist with recorded preservation losses.
Duplicate entity UUIDs drop later bodies (original NBT is kept as separate
evidence); some empty unfinished chunks regenerate rather than survive. Region
compression, palette and compound order, omitted defaults and `LastUpdate` are
normalized, not byte-identical. The receipt's file inventory uses CRC32, not a
cryptographic signature.

### What survives, and what does not

Storage retention and active gameplay support are different: unsupported active
records can **reject** an import rather than silently disappear, and a retained
field is not proof the native game implements its behavior. Do not promise
lossless migration because an unknown field is kept in an opaque record.

| Domain | Migration behavior and limits |
|---|---|
| Blocks, biomes, chunk metadata | Registry state identity; unknown states/properties refuse import and missing properties take the registered default. Every 4×4×4 biome cell is kept by identifier. Heightmaps and light arrays are kept, but native edits can recompute them. New terrain uses Krasnokamen generation, not bit-identical vanilla; inspect explored-border seams. |
| Protochunks | Non-full chunks with terrain or owned payload keep their status; all-air non-full chunks without owned payload are omitted and regenerate. Structure starts/references of empty protochunks may be discarded and recorded as a loss. |
| Entities | Typed state and unowned fields are kept; villagers (offers/gossip), item frames, armor stands, paintings, minecarts and boats are decoded, species-specific forms that are unsupported fail with file/chunk/UUID context. Wire ids are reminted. Check villagers/trades, passengers, pets/owners, vehicles and named mobs individually. |
| Containers and other block entities | Chest, barrel, furnace, hopper and brewing-stand inventories and timers are typed; unowned metadata (`CustomName`, `lock`, `components`) is retained alongside them, but a retained lock is **not enforced** by native gameplay. Signs, banners, spawners, shulkers, lecterns, beehives, campfires and jukeboxes keep their NBT by position unless a native mutation replaces it; validators may refuse unsupported forms. Verify each important block entity. |
| Player data | UUID-keyed player NBT: inventory, ender chest, XP, effects, spawn, attributes, recipes, position and gamemode have native owners and unowned fields are retained. A first-time player joins in the world's `GameType` (see `gamemode` above). A missing dimension falls back at login/respawn and is loss-reported. Test a real returning player in all dimensions. |
| Advancements, statistics | Owned JSON is imported and re-emitted semantically; orphan documents stay opaque; stats-only UUIDs can create native offline-player records. Custom criteria execution is not implied. |
| Time, weather, game rules, spawn, border, dragon fight, wandering trader | Typed metadata refreshed from native state; 26.1.2 stores several formerly `level.dat` fields in separate saved-data files. Verify each dimension, especially raids and the End fight. |
| Maps | Upgraded and retained as saved data; check item map ids and rendered pixels. Byte retention does not prove live exploration updates or decorations. |
| Scoreboards, teams, raids | Typed native owners. Each score keeps its `Locked` flag (`/scoreboard players enable` unlocks a trigger, `/trigger` consumes it). Deferred old scoreboard data that conflicts with an active native scoreboard fails export. Verify objectives, scores, team options and display slots. |
| Scheduled ticks | Native queues keep identity, priority and remaining delay; no-ops and duplicates are normalized and counted; delays outside the Anvil integer range fail export. |
| POI, structure starts | POI files stay byte-exact opaque files, not native POI authority. Full-chunk structure metadata is retained. |
| Other `data/*.dat` and auxiliary files | Explicit allowlist only: `icon.png`, `level.dat_old`, `resourcepacks/resources.zip`, `advancements/`, `stats/`, `data/`, `players/`, and dimension `poi/` and `data/`. Unknown active saved-data `.dat` semantics refuse import; opaque retained files share a 16 MiB aggregate record budget. Everything else in the source tree (including `datapacks/`) is **not carried**. |
| Datapacks, custom dimensions | Not migrated. Copy directory datapacks to `<native-world>/datapacks/` yourself ([supported subset](OPERATING.md#datapacks-and-command-blocks)); ZIP packs, worldgen packs and custom dimensions are unsupported, so do not migrate a pack-dependent world unless losing that function is acceptable. |
| Resource packs | The world's `resourcepacks/resources.zip` is retained; server-offered packs are configured through `[resource_pack]`. |
| Bukkit/Paper plugin data | Not interpreted. External SQL stores, plugin folders, claims, economy, permissions and custom items need a separately validated migration; keep the originals for rollback. |
| Krasnokamen-native plugin, settlement and block-log state on export | Vanilla cannot run or interpret it, and Anvil is not a backup format for these domains. Keep a native backup. |

Implementation boundaries: [world conversion](../crates/mc-world-conversion/src/lib.rs),
[import](../crates/mc-world-conversion/src/import.rs),
[export](../crates/mc-world-conversion/src/export.rs) and
[admin lists](../crates/mc-edge/src/admin.rs).

## 6. Replace plugins deliberately

**Bukkit/Spigot/Paper JARs are not supported natively.** Do not copy a Paper
`plugins/` directory into Krasnokamen's package root. The normal plugin surface is
sandboxed Wasmtime components for `krasnokamen:plugin@0.9.0`, deployed as
`plugin.toml` + `plugin.wasm` packages through `[plugins].directory`
([PLUGINS.md](PLUGINS.md); an experimental offline converter for a tiny Bukkit
slice is described there with its limits). First-party packages live in
[krasnokamen-default-plugins](https://github.com/kaiserproger/krasnokamen-default-plugins).
Matching plugin names do not imply compatible configuration or databases.

Rehearse world conversion without plugins first. Inventory every old plugin's
function, persistence, commands and permissions, then deploy only supported
replacement packages following their own migration notes. Registry- or
worldgen-changing deployments can alter native world identity, so do not assume
adding them after conversion is harmless. Permission, claim and economy checks
must pass before public login is enabled.

## 7. Start, verify, stop and prove rollback

```sh
krasnokamen --config /srv/krasnokamen/server.toml
```

Wait for the listening message and successful content, world and plugin
admission. On the private endpoint, join with an unmodified supported client
(every release from 26.1.2 through 26.3; import installs all caches, see
[OPERATING.md](OPERATING.md#client-protocol-compatibility)) and
complete the checklist below. Stop with console `stop` and wait for the process
to exit and write its final checkpoint. Serving writes the native
`native/state.redb` and tick WAL, **not** the old Anvil files; keep the same
config, content and deployment with the native world.

### Downgrading the Krasnokamen binary

Do not run an older Krasnokamen binary on a world that a build with cow, pig,
chicken and zombie-nautilus variant persistence has served. The tick WAL refuses
to recover (the older decoder does not know the variant records), and a
checkpoint is rewritten without them: the older build ignores the `variant`
field of a loaded entity and never writes it, so cow, pig and zombie-nautilus
variants are lost silently when the older binary checkpoints a loaded chunk.
Restore a backup instead.

### Rollback option A: restore the untouched original

Stop Krasnokamen and restore the entire original vanilla/Paper installation from the
pre-migration backup. Reapply the old port/proxy routing only after verifying it
starts. This is the most predictable rollback but **discards all progress made on
Krasnokamen**. Never mix the original player files with a newer world export.

### Rollback option B: export Krasnokamen progress to Anvil

While Krasnokamen is stopped:

```sh
krasnokamen --config /srv/krasnokamen/server.toml world export-anvil \
  --source /srv/krasnokamen/native-world \
  --destination /srv/migration/anvil-export \
  --receipt /srv/migration/receipts/export.json
```

Require a `PASS` receipt and review gaps and normalizations again. Export
recovers the native checkpoint and WAL tail into a **new** Anvil world. It is
written as 26.1.2 (`DataVersion` 4790) unless a record had to be promoted to
26.3 (5023); `level.dat` and the receipt report the highest version emitted
(`Version.Name`), and a promoted export needs a vanilla server of that release.
It is not a downgrade to older Minecraft versions. Retain a native-world backup
as well. Export writes the WAL frame as `Data.Time`, so exporting and importing a
world that was ever frozen adds the held (frozen) ticks to its game age.

In a separate vanilla test installation of the reported release, put the export
at the configured `level-name` path, keep your already accepted EULA, set a
private bind/port and the right auth mode, and run:

```sh
java -Xms512M -Xmx2G -jar /srv/minecraft/server-<release>.jar nogui
```

Wait for vanilla's `Done` message, then check landmarks, containers, named mobs,
scoreboards and a real player. Run `save-all flush`, `stop` and verify a second
start if the export will become the live world. A start alone is not a
normalized-NBT comparison or gameplay proof.

Access lists live outside the export: copy the **latest** Krasnokamen `ops.json`,
`whitelist.json`, `banned-players.json` and `banned-ips.json` back beside
vanilla's properties, restore `white-list=true` as appropriate, and keep the old
usercache (a cache, not player state). Translate any changed configuration back
explicitly. For Paper, use that version's supported vanilla-layout intake and
restore the matching plugin and database backups. Export cannot recreate Bukkit
state or Krasnokamen-only plugin effects; validate this boundary before accepting
player progress.

## Verification checklist before cutover

- [ ] Whole old installation backed up and restoration tested; source stopped and
  checksummed; binary/content version and numeric seed recorded.
- [ ] Every non-default property and plugin has a supported replacement, an
  accepted difference, or a migration-blocking decision.
- [ ] Content verify passes; the import destination is new; the receipt is `PASS`
  and all `known_gaps`, drops, normalizations and opaque retention are reviewed.
- [ ] Overworld, Nether and End landmarks, spawn, portals, borders, weather,
  difficulty and game rules are correct; new-terrain seams are acceptable.
- [ ] Named/locked containers keep names, locks and exact items; signs, banners,
  furnaces, hoppers, redstone, maps and important block entities are checked.
- [ ] Pets, villagers/trades, passengers, vehicles and named entities survive.
- [ ] Returning players keep UUID, inventory, ender chest, XP, health, position,
  dimension, respawn, gamemode, advancements and statistics; a new join works.
- [ ] Scores, team options, display slots and map ids/rendering are checked.
- [ ] Operator and non-operator permissions, whitelist denial, active and expired
  bans and proxy identity/address behavior are tested before opening the firewall.
- [ ] Replacement plugins load and protect claims, economy and permissions; no
  unsupported datapack or plugin behavior is being assumed.
- [ ] Clean Krasnokamen stop/restart succeeds without changed seed or content identity.
- [ ] The export receipt passes, a vanilla server of the reported release opens
  the export and the same landmarks, containers and player state are verified;
  rollback losses are understood.
- [ ] Only then stop the rehearsal, schedule a final source freeze/import and
  switch the public port/proxy. Keep the original backup and both receipts.

A headless fixture exercises only its named records; it is not proof of player,
Paper-plugin or full-world compatibility.
