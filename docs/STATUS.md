# Status

What works and how each part was last checked. Updated 2026-11-27 on `973e6e60`.
Update this page in the change that alters what works or how it was proven;
day-to-day progress belongs in commits and the [bug ledger](bugs/README.md).

Labels ([decision V1](../DECISIONS.md#evidence)):

- **Verified**: a real vanilla Java client ran the path against our server
  (unmodded, or with the Loader or the client mod the row names), or a
  differential test compared our result with a real Mojang server on the same
  input.
- **Native-gated**: run end to end with our native client under the harness
  (graphical, Xvfb). It proves our server and our client together, not vanilla
  compatibility.
- **Tested**: unit, fixture, in-process or TCP protocol-bot tests only.
- **Not run**: nothing exercises it.

"Last real run" is the newest passing harness receipt of the strongest kind
found for the area on 2026-11-27 under `.analysis/validation/` of the main
checkout and every worktree. Receipts are not in Git, and removing a worktree
removes its receipts. A run proves only what its scenario asserts. "Rows" counts
ledger rows fixed with a real-client receipt (`client_receipt` in
`bugs/promotions/*.json`: 221 rows, 1 on a vanilla Java client, 2 on Horizon
clients, the rest on the native client).

## Release

- Published: **1.0.0-rc.5 "Pauline"** (core tag `v1.0.0-rc.5`, native client
  release with matching server and plugins); no later tag exists. Notes:
  [RC5](releases/v1.0.0-rc.5.md), all tags: [release notes](releases/README.md).
- RC5 checks on the published assets: checksums match; a cold Linux client
  install entered a new world, relaunched and re-entered it. On a 4-core
  Windows VM without a GPU the first two Play attempts timed out
  (`SP-START-TIMEOUT-SLOW-MACHINE`); walking and disconnecting were not checked
  there. No macOS client or package run exists (core lib tests in CI only).
- Next: an RC6-or-better public debut against the frozen
  [announcement criteria](DEFINITION_OF_DONE.md#finite-announcement-acceptance)
  (owner D91). Tags and publication need the owner's explicit authorization.

## Feature areas

| Area | Status | Last real run | Result |
|---|---|---|---|
| Join, login, protocol window | Verified on 26.3, and on 26.1.2 with a Loader client; 26.2 Tested | 2026-11-24 `settlement-map-qa`, vanilla 26.3 client; 2026-11-15 `java-battle-end-camera`, 26.1.2 Loader client | 7/7 and 8/8 assertions. 26.2 has only the real-cache TCP joins in `translation::join_tests` (ignored by default). Online-mode Mojang login and Velocity forwarding: Tested. 5 rows |
| Chat and commands | Native-gated | 2026-11-26 `native-command-feedback`; 2026-11-15 `native-chat-visibility` | Chat visibility 26/26 (full, commands-only, hidden). 33 rows. No vanilla-client run of chat settings or command suggestions |
| Movement, physics, combat | Native-gated | 2026-11-24 `native-elytra`, `issue1-19-hostile-motion` | 7 and 13 passing runs. The vanilla player differential (diagonal sprint, attribute swapping; [quirk gate](QUIRK_PARITY.md)) has no retained receipt |
| Mobs and villagers | Native-gated | 2026-11-26 `native-v2-dolphin-treasure`, `native-entity-bogged` | 16 rows. Attack on a cow by a TCP bot: 2026-11-25 `protocol-mob-attack-cow`. No vanilla differential for spawning or AI beyond the quirk farms |
| Items, stations, containers | Verified for crafting-grid drags | 2026-11-07 `java-v2-p5-craft-drag-paced`, vanilla Java client | 74/74: cursor retention and custody for left, right and outside-release drags. 26 rows on the native client (latest 2026-11-26) |
| Blocks, redstone, fluids, farming | Verified for one quasi-connectivity scenario | 2026-11-23 quirk gate `qc-piston-bud` against a Mojang 26.1.2 server | Pass. The other 10 quirk scenarios (TNT duping, lever lamp, seven farm rates) have no retained receipt; 5 of them carry `known_divergence`. 8 rows |
| World generation, dimensions | Native-gated (one scene) | 2026-11-24 `native-kelp-ocean-floor` | 1 row. No terrain differential against vanilla retained; the target is wiki-level similarity ([P2](../DECISIONS.md#product)) |
| Survival loop, game rules | Native-gated | 2026-11-26 `native-bed-occupied-state`; 2026-11-24 `native-v2-p4-damage` | 10 and 4 passing runs. 9 rows |
| Saves, WAL, recovery | Native-gated | 2026-11-26 `native-v2-ledger-gravity-tick`; 2026-11-25 `native-v2-corrupt-chunk-near-camera` | 16/16: an imported world's saved block tick survives a restart. 16 rows. Anvil round trip through a Mojang server (`tools.harness.vanilla_world_roundtrip`): no retained receipt. Power cut and Windows soak: Not run |
| Datapacks, resource packs | Native-gated | 2026-11-27 `native-datapack-reload` on `973e6e60` | 47/47: the crafting output follows `/datapack enable` and `disable`. 13 rows. Server-offered resource pack download and render: Not run |
| Administration | Native-gated for commands | 2026-11-11 `admin-commands` | 41/41: `/tellraw`, `/title`, `/bossbar`, `/tick`, `/locate` read back from the client. RCON and Query with an external client: Tested |
| Wasm plugins | Tested | none of its own; first-party plugins run inside the settlement and battle gates | Plugin host unit and startup tests |
| Settlements | Verified for the map-dialog starter-chest race | 2026-11-24 `settlement-map-qa` C1, vanilla 26.3 client; 2026-11-26 `buildings-qa`, native | 7/7 on Java; 22/22 native; `autocraft-qa` 78/78 (2026-11-24); `village-qa` 32/32 (2026-11-23). 47 rows, 1 on Java |
| Battles | Verified for the end camera | 2026-11-15 `java-battle-end-camera`, 26.1.2 Loader client; 2026-11-25 `battle-crash-handoff`, Horizon clients | 8/8; `battle-rearm-recovery` 20/20 with Horizon clients (2026-11-24); `battle-live-spectator-follows-respawn` 9/9 native (2026-11-25). 2 rows |
| Screen sharing, voice, Loader | Verified on Fabric with Loader and Simple Voice Chat | 2026-11-26 `screens-voice-qa`; 2026-11-17 `loader-fabric-mixed-interaction` | 11/11 verdicts: consent, deny, app audio, viewer respawn, SVC proximity Fabric–Fabric and Fabric–native, SVC group; chat, visibility and item handoff between Fabric and native. 4 rows |
| Far terrain | Verified for Distant Horizons on 26.1.2 | 2026-10-22 `dh-gate`, Fabric with DH 3.3.3; 2026-11-19 `horizon-lod-live`, Horizon client | DH pass; Horizon 18/18 at view distance 4, LOD 64. Native `[lod]`: no retained harness receipt. DH on 26.2 and 26.3: Not run |
| Native client, singleplayer | Native-gated | 2026-11-27 `native-model-layers`; 2026-11-25 `native-sp-slow-machine` | 101/101 and 15/15. 41 rows on client presentation. RC5 assets: see [Release](#release) |
| Joins under load | Native-gated | 2026-11-24 `join-lag-gate` | 3/3, 23 passing runs. Measured numbers: [profiles](performance/PROFILES.md) |

## Coverage by client

What each client can be asked for today. `yes`: works; `part`: works with the
restriction named in the cell; `no`: refused or absent, and the cell names what
to use instead. Java columns are unmodded clients; mods go in the Fabric/NeoForge
column. Horizon is the pinned Horizon 26.3 client the harness drives
([Horizon gates](development/real-client-gates.md#battle-crash-recovery-with-real-horizon-clients)).
Evidence for each cell is in [Feature areas](#feature-areas).

| Feature | Java 26.3 | Java 26.2 | Java 26.1.2 | Fabric/NeoForge with mods | Native client | Horizon client |
|---|---|---|---|---|---|---|
| Join and vanilla play | yes | part: newer content arrives as its nearest 26.2 form | part: nearest 26.1.2 form | yes (Loader jars: 26.1.2; Loader `main` also builds 26.3) | yes | yes (26.3) |
| Plugin client content (Loader bundles) | no: login stops in Configuration naming the supported platforms; use a Loader client | no: same | no: same | yes with the Krasnokamen Loader (Fabric, NeoForge, Forge) | yes (Loader wire 3) | no: disconnected without Loader negotiation |
| Far terrain beyond view distance | no: use the native client, Distant Horizons or Horizon | no: same | no: same | part: Distant Horizons 3.3.3 on 26.1.2; 26.2 and 26.3 need their presentation profile | yes (`[lod]`) | yes (`[horizon_lod]`, stored columns only) |
| Proximity voice | no: install Simple Voice Chat or use the native client | no: same | no: same | yes with Simple Voice Chat (opt-in bridge) | yes | no |
| Screen sharing | no: use the native client or Fabric with Loader | no: same | no: same | part: Fabric with Loader only; NeoForge and Forge are refused | yes | no |
| Krasnokamen singleplayer | no: use the native client | no: same | no: same | no: same | yes | no |

Where the cells come from: the adjacent translation chain
([ADR 0015](decisions/0015-multiversion-protocol-translation.md)),
[Loader required-client behaviour](KRASNOKAMEN_LOADER.md#required-client-behavior),
[far terrain](OPERATING.md#far-terrain-lod), [Horizon far terrain](OPERATING.md#horizon-far-terrain)
and [Distant Horizons](OPERATING.md#distant-horizons),
[voice](decisions/0019-voice-chat.md) and
[screen sharing](decisions/0018-in-game-computer.md#screen-sharing-krasnokamenscreen-protocol-1)
admission.

## Known gaps by area

Missing or knowingly different behaviour; the rest of each area is limited only
by its evidence above. Ledger rows track individual bugs.

- **Protocol and login.** Missing endpoint data disables only that version, and
  a missing translation step disables routes across it. Older clients render
  `/swing` kinds other than hand swings as legacy swings without duration.
  Signed chat has no signature or certificate verification and no reportable
  delivery.
- **Commands.** `/execute store` writes only score destinations; block-tag and
  SNBT predicates and rich suggestions are incomplete. `/place` and item
  modifiers are not implemented; `/particle` refuses option-bearing types;
  `/loot` refuses mine and fish contexts; unsupported attribute kinds are
  refused; player `scale` and `step_height` reach clients but not server
  collision. Command blocks lack comparator output, command minecarts and the
  editor (configure them through `/data`). Styled `CustomName` compounds are
  refused (`ENTITY-STYLED-CUSTOM-NAME-REFUSED`, owner decision pending).
  Bossbars are not exported to Anvil.
- **Movement and combat.** Mounted portal travel is unsupported. An eye of
  ender rises and hovers smoothly instead of oscillating.
- **Mobs.** Undead walkers keep blocked direct pursuit. Mob-farm and gold-farm
  mobs drift off the collection pad while airborne, and villager-breeder output
  differs through the farmers' harvest cadence (quirk `known_divergence`).
  Slime hops run on an absolute tick phase, not vanilla's countdown that pauses
  in flight. Dolphins swim straight water waypoints to a located structure.
- **Items and containers.** Special recipe serializers are incomplete.
  Exploration-map loot needs the saved-map callback and omits the map when the
  lookup finds no destination. Item frames lack `Fixed`, `ItemDropChance` and
  complete map decorations. A decorated pot with missing named sides drops
  nothing.
- **Blocks, redstone, fluids.** Pistons move instantly: no short-pulse spit,
  transient collision or moving-piston window, so the TNT BUD duper yields one
  TNT instead of two and carpet and rail dupers do not work. Torch burnout,
  exact update order and some analog comparator sources are missing; the fluid
  destroy list around a push differs. Azalea and mangrove trees, part of
  bone-meal coverage and exact tree shapes are missing; ordinary vines grow only
  downward; landing mobs do not trample farmland.
- **World generation.** Lush and dripstone cave features are incomplete.
  Legacy structures use authored fixed layouts without rotations, processors,
  stronghold biome relocation, the jungle piston puzzle or full monument and
  mansion variants; fortress pieces and ruined-portal aging are incomplete. The
  End central island is our own height field and outer-island assemblies are
  authored. Portals lack passenger assemblies and the 300-tick destination
  lease.
- **Survival loop.** Thunderstorms create no skeleton-horse traps and do not
  deoxidise copper. Temperature approximates by biome family and altitude. Bed
  exits use approximate collision shapes. Some advancements, special recipes
  and natural progression are missing.
- **Saves and administration.** Anvil import serves non-empty proto-chunks
  incomplete, may lose opaque data when regenerating an empty one, refuses
  superflat and custom dimensions, and keeps POI opaque. Datapacks apply only
  the experimental jigsaw worldgen inputs (at restart); any other worldgen or
  dimension resource refuses the whole pack (`DATAPACK-WORLDGEN-OVERRIDES`);
  macros, arbitrary resource and tag types and non-block loot are unsupported;
  `filter` patterns use Rust regex syntax. `server.properties` keys without a
  TOML counterpart only warn.
- **Plugins.** Unsupported hooks (including before-damage) refuse startup;
  there is no reload. Bukkit, Spigot and Paper JARs do not run.

Deliberate differences from vanilla ([P7](../DECISIONS.md#product)):

- Native store and WAL only: no live Anvil serving or dual write
  ([W1](../DECISIONS.md#persistence)).
- Mount logout: a processed Leave dismounts and leaves the vehicle in the world,
  where vanilla stores a sole rider's vehicle with the player; reconnecting
  re-seats the rider ([mount custody](architecture/state-ownership.md#mount-custody)).
- Worldgen datapack content the native generators cannot apply is refused, not
  interpreted ([G3](../DECISIONS.md#world-generation)).
- Exact RNG, AI cadence and vanilla bugs are not reproduced, except the quirks
  players rely on, which the [quirk gate](QUIRK_PARITY.md) checks
  ([P2](../DECISIONS.md#product)).

## Priorities

Work order: [decision P4](../DECISIONS.md#product). Current owner focus
(D123–D129, 2026-11-23/27): close the open ledger rows in batches before the
1 December push of `main`; split `mc-core` into real crates as the top
structural priority ([B1](../DECISIONS.md#build-and-crates)) with measured
incremental-rebuild effect; prune useless unit tests in favour of differential
tests against vanilla ([V2](../DECISIONS.md#evidence)); file-by-file core
code-quality cleanup; documentation in the D129 shape (entry file, rules,
pitfalls, this page, `DECISIONS.md`).

## Work queue

The owner's open GitHub issues are the queue:

```sh
gh issue list -R kaiserproger/krasnokamen --author kaiserproger --state open
```

Known bugs, with status and regression checks, are rows under
[`bugs/rows/`](bugs/README.md): 198 open (82 client, 73 gameplay, 43 harness)
and 548 fixed on `973e6e60` (`python3 -m tools.harness ledger list --status open`).

## Open transitions

A transition replaces one mechanism with another and is open while both are in
the tree. A row exists only where the target is decided (an accepted ADR, a
`DECISIONS.md` row or an owner decision); proposals and supported surfaces are listed
below the table and carry no rule. This is the only register: the change that
opens a transition adds its row, the change that deletes the old side deletes
the row. How a change treats an area with an open transition:
[adding a feature](development/adding-a-feature.md#open-transitions). The
commands below the table reproduce the counts.

| Transition (old → new) | State and evidence | Done when | Owner doc |
|---|---|---|---|
| Harness server launch: direct `subprocess.Popen`/asyncio spawn → `runtime.start_server` | 178 modules in `tools/harness/backends/`; 31 start the server through `runtime.start_server` (35 call sites). 8 call sites in 4 modules still start `krasnokamen` with `Popen` (`client_probes.py` 4, `village_autonomy_qa.py` 2, `seed_review.py` 1, `native_scale.py` 1 through its generic `spawn`), and `catalog_load.py` (also used by `catalog_spain.py`) starts it through `asyncio.create_subprocess_exec`. `runtime.start_server` itself is only `subprocess.Popen`: the sites share `wait_port` and `stop_process`, not a launcher. | One launch function owns start, readiness and teardown of the server, every backend calls it, and no backend runs `Popen` or `create_subprocess_exec` on the server binary. | [AGENT_TOOLING](AGENT_TOOLING.md#receipts-and-failure-diagnosis) |
| Release-binary pinning: the ledger's `.analysis/ledger-builds` registry → the `tools/dev/pin-release` manifest | `regressions._release_server`, `ledger probe` and `promote` consume `pin-release` manifests and verify binary checksum and exact tree against the pin's ledger registration; there is no private release build. `pin-release` still writes `.analysis/ledger-builds/<sha256>-<head>-<diff>.json` and the ledger's `_register_server` still binds receipts to `tree_fingerprint` through it. | The registry and `_register_server` are removed after pin manifests carry the ledger fingerprint and promotion checks the manifest directly. | [tools/dev README](../tools/dev/README.md), [bug ledger](bugs/README.md#the-fixed-rule-single-home) |
| Typed block and item identity: `.path() ==` comparisons and inline `"minecraft:` literals → kinds resolved once at load (target stated in the ratchet source, owner-approved 2026-11-07) | No shared `BlockKind` or `ItemKind` exists (only module-local kinds such as `vehicles::VehicleKind`). The enforced ceilings in `crates/xtask/src/code_health/ratchet.rs` are 2669 literals and 113 comparisons (original baselines 2441 and 88 at `c09e1391`); exceeding either fails the gate. Local cutovers (block tags, upper-tip states resolved once at load) are not a shared kind. | A typed kind exists, new code uses it, both counts are at or below their baselines and no new literal enters. No ADR owns this yet: write it before the first kind lands. | the ratchet source; [ADR 0016](decisions/0016-mob-capabilities.md) for mob types |
| Mob behaviour chosen by type id or name → capability flags ([ADR 0016](decisions/0016-mob-capabilities.md)) | Layers 1 and 2 (plugin registration) and sensors step 1 are implemented. `mc-core` production code still has 8 `== *X_TYPE` comparisons and 12 `matches!(<name>, "minecraft:…")` tests (grep count, includes legitimate uses). | No behaviour shared by more than one mob type is selected by comparing its id or name; each is a flag in `vanilla_capabilities`. | [ADR 0016](decisions/0016-mob-capabilities.md) |
| `mc-core` structure: features inside the big module cycle → each new feature its own module or crate with a narrow interface and one host registration point (owner, 2026-11-16); existing code split into independent crates (owner D124, 2026-11-27) | New features follow it since 2026-11-16. The code-health ratchet counts top-level modules in the largest cycle and fails above its ceiling of 186 (it checks neither edges nor submodule cycles); a submodule's references count as its host's, so the count is checked on the integrated tree. The registration point serves review and parallel lanes; it does not break a cycle. D124 starts the real crate split now, along ADR 0021, with measured incremental-rebuild effect. | Existing features are extracted along ADR 0021 and the big cycle no longer exists. | [ADR 0021](decisions/0021-build-time-and-crate-split.md), [adding a feature](development/adding-a-feature.md#new-features-are-modules) |

```sh
grep -l 'runtime.start_server(' tools/harness/backends/*.py | wc -l   # Popen sites: grep -n 'subprocess.Popen(' there
cargo run -p xtask -- code-health                                     # ratchet values and trend
grep -rnE '== \*[A-Z_]+_TYPE|\*[A-Z_]+_TYPE ==' crates/mc-core/src --include=*.rs | grep -v _tests.rs | wc -l
grep -rnE 'matches!\([^)]*name[^)]*, *"minecraft:' crates/mc-core/src --include=*.rs | grep -v _tests.rs | wc -l
```

Undecided proposals (no rule attached; each becomes a row only when the owner
decides it):

- Tick system graph ([ADR 0017](decisions/0017-tick-system-graph.md)): a
  one-system prototype (`EntityStep`, `crates/mc-core/src/tick/system_graph.rs`,
  run once from the entity phase). The ADR is not approval to migrate the rest of
  the tick; each later step needs the owner's approval and a measured benefit.
  The existing tick is the current path, not an old side.
- Earth WorldPlan ([decision G4](../DECISIONS.md#world-generation)):
  proposed, not ported. `crates/mc-worldgen/src/earth.rs` and `earth/` (15.8k
  lines with tests) still compile into `mc-worldgen`; only `mosaic.rs` and
  `bin/worldgen_mosaic.rs` use them. Port or delete is the owner's call.
- [ADR 0016](decisions/0016-mob-capabilities.md) Layer 3 (Wasm capability
  code), operator data-directory blueprints and persistent sensor subscriptions.
- Content without Java ([ADR 0022](decisions/0022-content-without-java.md)):
  the native RegistryData encoder `mc_data::registry_network` is proven byte for
  byte against vanilla captures but not wired into `content import`; waits on
  the owner's four questions in the ADR. The Java import and the singleplayer
  content bundle are the current path, not an old side.
- Off-screen settlement simulation ([ADR 0023](decisions/0023-offscreen-settlement-simulation.md)):
  accepted; migration transit and flows, game-time economy clocks and deferred
  production/consumption are integrated (slices 1–3). Construction labour and
  birth credit (slices 4–5) remain planned; their current paths still require
  simulation.

Supported surfaces (standing contracts, not transitions; only an owner-declared
sunset would make one):

- Client window: **every release from 26.1.2 through 26.3** (775/776/777),
  canonical 26.3. [ADR 0015](decisions/0015-multiversion-protocol-translation.md)
  owns the adjacent chain in `crates/mc-edge/src/translation/`; `content import`
  installs every endpoint cache and the independently owned step snapshots
  (startup never downloads; [operations](OPERATING.md#client-protocol-compatibility)).
  A version bump adds a step and never drops existing clients. Backfill to 1.7
  is a future constraint, not a compatibility claim.
- Loader ([ADR 0010](decisions/0010-krasnokamen-loader-contract.md), accepted,
  staged): the Loader repository's `main` (`d508697`, same as `origin/main`)
  defaults to `minecraftVersion=26.3` (jars `1.0.0-rc.2`) and still builds
  26.1.2 for Fabric/NeoForge with `-PminecraftVersion=26.1.2`, while
  [KRASNOKAMEN_LOADER](KRASNOKAMEN_LOADER.md) documents 26.1.2 only and
  `LOADER-SOUND-MIXIN` stays open. Whether 26.3 replaces 26.1.2 is the owner's
  decision; until then both are supported.

Other ADR states: [ADR 0018](decisions/0018-in-game-computer.md) (in-game
computer, screen sharing) and [ADR 0019](decisions/0019-voice-chat.md) (voice
chat) are accepted designs with draft implementations; screen sharing and voice
passed `screens-voice-qa` on 2026-11-26 ([Feature areas](#feature-areas)), and
the Fabric computer terminal has no harness gate.
[ADR 0020](decisions/0020-entity-checkpoints-at-one-million.md) is cut over
(live checkpoints write only changed entity rows, `changed_checkpoint_data_for`
in `core/capture.rs`; the materialized capture serves offline conversion, Anvil
export and tests by design) but was accepted with its strict latency criterion
unmet: the 1–2-core profile, the exact-UUID offline fold after reopen and SIGKILL
after a committed checkpoint were never run (last revalidated on `73761dc5`,
2026-11-14). The plugin API ships as `krasnokamen:plugin@0.9.0`;
[ADR 0024](decisions/0024-engine-and-gameplay-packages.md) is accepted, not
implemented.

## Known gaps before 1.0

From the independent whole-core review of RC5 (2026-11-07), updated. Each needs
a fix or an owner decision recorded in the release notes.

| Gap | State |
|---|---|
| The bug ledger as a 1.0 gate | 198 rows open; 1.0 needs them fixed or dispositioned and `regressions-client` run on the release candidate |
| Plugin API `krasnokamen:plugin@0.9.0` has no stability promise | owner decision |
| Scoreboards of ~500+ scores and functions sending 512+ chat lines per tick disconnected recipients (per-tick outbox cap) | fixed after RC5 (`700ae656b`, `OUTBOX-TEXT-FLOOD-KICK`); unreleased |
| Phantoms and pillager patrols never spawned in survival | fixed after RC5 (`886c556f9`); unreleased |
| A world whose first start stopped before the initial checkpoint could never open | fixed after RC5 (`WORLD-FIRST-START-INTERRUPTED`: such a world reopens as a new world); unreleased. An RC4 world in that state (no first-start marker) is not separately proven |
| Conduit, beacon and shrieker block entities panicked the tick on corrupt data | fixed after RC5 (`CONDUIT-RUNTIME-PANIC`, `BEACON-RUNTIME-PANIC`, `SHRIEKER-HYDRATE-PANIC`); unreleased |

Unconfirmed risks, each with the check that would settle it:

- Windows: a WAL batch file briefly locked during rename (antivirus, sync
  tools) stops the world. Check: a ≥1 h Windows soak, singleplayer and hosted,
  Defender on, world in an indexed or OneDrive folder.
- A power cut may leave recent journal batches missing. Each batch file is
  fsynced before rename; on Unix the directory is fsynced every
  `journal_fsync_every_batches` appends (default 10) and before a checkpoint; on
  other platforms directory fsync is a no-op. Whether missing directory entries
  can leave a gap that blocks recovery is unverified; do not assume only a
  trailing suffix is lost. Check: power-cut tests on ext4, XFS and Windows.
