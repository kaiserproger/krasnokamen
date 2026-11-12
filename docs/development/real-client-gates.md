# Real-client gates in practice

What counts as evidence is fixed elsewhere: [AGENTS.md](../../AGENTS.md)
(actual-client gates, release server binary, breaker), the
[bug regression contract](../bugs/README.md), the
[break-it QA protocol](../QA_AGENT_PROTOCOL.md) (gate hygiene, negative
controls, variance) and [AGENT_TOOLING.md](../AGENT_TOOLING.md) (harness
commands, receipts, Java/Loader client variables). This page is the working
recipe and the traps for graphical and native-client gates. Profile and scenario
lists come from `python3 -m tools.harness list` and `explain`, not from here.

## Prerequisites

Everything below is reproducible from public sources, except where a line says
**owner-local**. Sibling repositories: the native client
([solaris-client](https://github.com/kaiserproger/solaris-client)), the Loader
([solaris-loader](https://github.com/kaiserproger/solaris-loader)) and the first-party
plugins ([solaris-default-plugins](https://github.com/kaiserproger/solaris-default-plugins)),
checked out beside this repository unless an environment variable says otherwise.

### 1. Build the release server and the native client

- Server: `cargo build --release -p mc-edge --bin solaris-v2`
  (the harness reads it from `SOLARIS_SERVER_BINARY`; debug servers drop to 2-9
  TPS while chunks generate, so timing-sensitive gates need release).
- Client (in the client checkout): `cargo build --release --bin solaris-client`.
  On Linux it links ALSA and needs the distro development packages (the client's
  release workflow installs `pkg-config libasound2-dev libudev-dev
  libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxi-dev
  libxrandr-dev libgl1-mesa-dev libvulkan-dev cmake`). The windowed gates also
  need a Vulkan 1.1+ driver; Mesa `lavapipe` software Vulkan works, slowly.
- **ALSA sysroot (owner-local workaround).** The owner's machine has no root
  install of the ALSA headers, so the client checkout carries an ignored
  `.analysis/build-support/usr/lib/x86_64-linux-gnu` (an unpacked ALSA 1.2.11
  development package: `libasound.so*`, `pkgconfig/alsa.pc`, plus a
  `libxkbcommon.so` link). With root, install the packages above and skip it.
  Without root, unpack the distro's `libasound2-dev` (and `libxkbcommon-dev`)
  packages into a directory yourself and point both variables at its `lib`
  directory: `PKG_CONFIG_PATH=$B/pkgconfig LIBRARY_PATH=$B`, with
  `B=<sysroot>/usr/lib/x86_64-linux-gnu`. Missing `PKG_CONFIG_PATH` makes
  `alsa-sys` panic; missing `LIBRARY_PATH` fails linking `-lasound`. Check the
  binary's mtime afterwards: an early-exiting wrapped build leaves the old binary.
  On the shared host wrap the build:
  `<work>/solaris/tools/dev/mem-run.sh -m 10G cargo build --release --bin solaris-client`.

### 2. Build the client data root from public sources

The client refuses to start without a complete data root. Let the launcher
produce it, from a release build of the client repository:

```sh
solaris-launcher --headless-bootstrap --data-dir <root>      # needs internet access
```

`--headless-bootstrap` is the whole first-run install without a window (also
usable in CI or on a server); `--force` reinstalls and `--manifest URL` overrides
the content manifest baked into the launcher. It downloads the Solaris facts
bundle (data tables only) and, from Mojang's own endpoints, the 26.3 `client.jar`
and the asset objects, then unpacks the textures, models and fonts. No Mojang
account is involved and no Mojang bytes are in Git. The resulting layout
(`launcher/src/paths.rs` and the client README in the client repository):

```text
<root>/facts/                     Solaris facts tables
<root>/data/vanilla/reports/      blocks.json, block_mining.json, block_light.json, registries.json, ...
<root>/resources/assets/          textures/models/fonts unpacked from client.jar
<root>/assets/asset-index.json    Mojang asset index (+ objects/)
<root>/mojang/versions/26.3/      version json and client.jar
<root>/cache/  installed.json     launcher state
```

`<root>` defaults to `$XDG_DATA_HOME/solaris-client` (`~/.local/share/solaris-client`).
Without internet access the client's own alternative is `solaris-client setup
--from-dist DIST --assets ASSETS`, which links a locally prepared bundle and an
asset cache. The owner's `<work>/qa-armada/template-data` is only a launcher
output kept on the owner's disk; recreate it with the command above.
The complete content bundle (a full server cache, for singleplayer only) and the
facts publishing flow are the client repository's business, not a gate
prerequisite.

### 3. Environment block

Keep these in one env file per gate wave and source it; never retype paths.

```sh
export SOLARIS_SERVER_BINARY=<core>/target/release/solaris-v2   # release build
export SOLARIS_CLIENT_ROOT=<client checkout>                    # default: solaris-client beside the main checkout
export SOLARIS_CLIENT_BINARY=<client>/target/release/solaris-client
# The data root from step 2:
export SOLARIS_CLIENT_DATA_DIR=<root>/data/vanilla              # the vanilla metadata dir, not <root>
export SOLARIS_CLIENT_RESOURCE_ROOT=<root>/resources
export SOLARIS_CLIENT_ASSET_ROOT=<root>/assets
export SOLARIS_CLIENT_ASSET_INDEX=<root>/assets/asset-index.json
# Sibling repositories, only to select a checkout other than the one beside the main checkout
# (worktrees resolve it too: runtime.sibling_root):
# Java probes and survival: point at canonical content with adjacent protocol caches:
export SOLARIS_SERVER_VANILLA_DATA_DIR=<content-root>/26.3
export SOLARIS_LOADER_ROOT=<solaris-loader checkout>            # Java/Loader clients; default: beside the main checkout
export SOLARIS_DEFAULT_PLUGINS_ROOT=<solaris-default-plugins>   # standard-pack and plugin gates; same default
```

*Owner host:* builds land in a pooled target dir (`targets/slot-<l>`) that
other trees reuse, so the harness refuses `SOLARIS_SERVER_BINARY` or
`SOLARIS_CLIENT_BINARY` inside a slot. Pin instead:
`<work>/solaris/tools/dev/pin-release <worktree> [--client]` builds once per
tree (HEAD plus the hash of uncommitted changes), copies the binary to
`<work>/pins/<sha12>[-<diff8>]/` with a manifest, prints the
`SOLARIS_SERVER_BINARY=` (or `SOLARIS_CLIENT_BINARY=`) line and reuses an
existing pin without building. Pins are immutable; the build is incremental by
default, `--no-incremental` makes the full build performance windows require. A harness profile that
builds the release server itself (`regressions-client`) hard-links its build
into the run's artifact dir.

- A headless native run (`--client native`) needs `SOLARIS_CLIENT_DATA_DIR`
  with its block and registry reports. `--windowed` additionally needs
  `SOLARIS_CLIENT_RESOURCE_ROOT`, `SOLARIS_CLIENT_ASSET_ROOT` and
  `SOLARIS_CLIENT_ASSET_INDEX`, including the assets directory, objects and a
  nonempty asset index. Missing or incomplete paths fail before launch.
  Real-client runs require executable `SOLARIS_SERVER_BINARY` and native runs
  also require `SOLARIS_CLIENT_BINARY`; paths under owner-host pooled
  `targets/slot-*` are refused. Pin copies outside the pool and use those paths.
  Unlisted profiles are rejected for native; see
  [AGENT_TOOLING](../AGENT_TOOLING.md#harness-entrypoint).
  Headless native profiles create a temporary writable XDG root with links to
  the template containing `data/vanilla`, `facts` and `resources/assets`;
  it is removed after the profile exits.
- A single wrong path (for example an item properties file under the wrong repo)
  leaves the client on a white "loading content" screen forever without naming
  the file; diff the env against the last green gate line by line.
- The client's own data folder is `$XDG_DATA_HOME/solaris-client`. Give each gate
  a fresh `XDG_DATA_HOME` whose `solaris-client/` **symlinks** (never copies; 650 MB
  per run filled the disk) `facts assets data resources mojang`. A shared XDG
  leaks `client-settings.json` between gates (an FOV assertion broke that way).
  Without it the client looks under `~/.local/share/solaris-client` (a dangling
  `facts` symlink there) and exits before MCP is ready. `scene-shot` does this
  itself (symlinks those four plus `assets`, copies the rest, forces fov 70).
- The singleplayer scenarios (`native-world-first-start-interrupted`,
  `native-sp-relaunch-lease-race`) need `SOLARIS_SERVER_BINARY` naming the release
  server and an imported 26.3 content cache at `<core>/data/vanilla`
  (`solaris-v2 content import --version 26.3 --download --cache data/vanilla`,
  see [OPERATING](../OPERATING.md#vanilla-content-cache)); they copy it into the
  private client data root as `servers/content/26.3`.
- Run each graphical gate through the scene slot so at most six run on the
  host and the gate's scope outranks tests and builds for CPU:
  `<work>/solaris/tools/dev/scene-slot.sh <work>/solaris/tools/dev/mem-run.sh -m 6G
  python3 -m tools.harness run <profile> --client native --windowed`.

### 4. Which gates need owner-held access

No harness gate needs a Mojang or Microsoft account: the native client uses the
`offline-test` profile (official authentication is not implemented) and the
Java/Loader client uses offline identities. The `SOLARIS_CLIENT_MCP_TOKEN` and
the bridge secrets are per-run random values you generate. What only the owner
holds:

- the public test server (`SOLARIS_TEST_HOST`, SSH, `solaris-auth` passwords,
  `SOLARIS_SOAK_AUTH`): [test server operations](test-server-ops.md), the
  headless join smoke below against that host, `tools/dev/test-server/soak.py`;
- the Windows VM login: [Windows and macOS client QA](windows-and-macos-client-qa.md);
- release publication (`gh`, tags, GitHub secrets):
  [releases](release-and-tags.md).

Never write any of these values, addresses or paths into tracked files.

## Evidence classes

Keep them separate in every report:

- **Functional:** headless native client, protocol bots, unit/TCP tests. A
  headless native run records `details.headless=true` and proves received state
  only.
- **Graphical:** native `--windowed` under the harness Xvfb with retained frames
  and pixel predicates, plus a human look at least one screenshot. Comparing
  light arrays or other internal state does not prove rendering.
  `present_requested` does not prove physical monitor scanout.
- **Java client:** proves vanilla-client compatibility for the path it ran; it is
  diagnostic for native-client rendering claims. Each retained release uses
  its own automatically imported endpoint cache and adjacent translation route.

Native `minecraft_click_container_slot` bypasses hit-testing; GUI-scale or layout
proofs need a physical XTest click (pattern: `_scroll` in
`tools/harness/backends/native_p4.py`).

## Isolation (owner rules, 2026-10-23/24)

- **Never the owner's desktop.** Every agent client runs on its own Xvfb display
  (never `:0`/`:1`) with `WAYLAND_DISPLAY` unset; the harness forces X11
  (`SDL_VIDEODRIVER`/`GDK_BACKEND=x11`) and scene-shot fails if `client.log` says
  `video driver: wayland`. Browser/noVNC viewers need their own proven headless
  isolation (QA protocol, breaker procedure).
- **Never the owner's microphone or speakers** (he is often in a call). Minimum:
  `ALSOFT_DRIVERS=null` for every agent-launched Java/Minecraft client (QA
  protocol). Stronger isolation used for bot fleets and QA hunts: a private null
  ALSA config (`pcm.!default { type null }`, `ctl.!default { type null }`) via
  `ALSA_CONFIG_PATH`, `PIPEWIRE_REMOTE=/nonexistent`,
  `PULSE_SERVER=unix:/nonexistent`, private `XDG_RUNTIME_DIR`; verify with
  `wpctl status` that no new stream appeared. A null output sink alone isolates
  only output, not capture. Harness audio gates record a virtual null-sink
  monitor, never the microphone; the desktop mic indicator during such runs is
  expected.
- Private ports, data roots and worlds per run; kill only processes you started,
  by PID.

## Native scenario rules

- Native runs the client headless (`play --headless`) against the harness-started
  server over an ephemeral loopback MCP endpoint: no Java, no Xvfb. Descriptor
  readiness uses inotify complete-write events and process pidfd, followed by
  authenticated MCP initialization; scenario readiness uses MCP state events,
  never sleeps. Each run uses an isolated descriptor/token directory under the
  client's `.analysis/harness-runs/`; the 0600 bearer token and private nonce are
  not receipted.
- The `survival` server uses an isolated seed-81 world with no plugins by default,
  no operators and hostile spawning disabled. The generic harness fallback builds
  the current debug server; it does not satisfy real-client gate requirements.
  For these gates pass the pinned `--release --locked` server through
  `SOLARIS_SERVER_BINARY` as required by AGENTS.md (path and SHA-256 are receipted).
  `--plugins-dir PATH` (an existing directory used directly as `[plugins].directory`)
  is accepted by the profiles that allow it and supplies no plugin-specific
  scenarios or unlock proofs.
- `--windowed` owns Xvfb and launches `play --windowed`. After success,
  `screenshots/native-final.png` and its JSON receipt retain an actual-window scene
  target with frame id, scene/state/resource revisions and GPU completion; missing
  capture evidence fails the run. The default stays functional headless and makes
  no visual claim.
- Operator fixtures use the vanilla `ops.json` beside the isolated server config
  (level 4, offline identities recorded in the receipt). Headless survival has no
  operator grant; windowed survival grants its one test identity for the
  separately labelled playthrough death fixture only, after ordinary
  mining/pickup/placement acceptance.
- The respawn path uses the bounded event-driven Play wait. An immediate
  `in_play=false` right after the Death-screen Respawn click is not itself a
  disconnect, but there is no protocol respawn fallback: a failed UI request, Play
  timeout or missing restored health fails the gate.
- Rendered name/text/colour/glow, piglin admiration, merchant offers and similar
  visual items need screenshot review; inventory debit or menu admission alone is
  not visual acceptance. Action/renderer gaps fail closed.
- Headless native scenario families are received-state evidence only;
  `--windowed` adds the shared final capture, not full pixel parity. `explain <id>`
  lists a scenario's exact fixtures and fences.
- `native-v2-chest-drag-disconnect` physically drags one log inside a chest
  menu, checks authoritative cursor custody, disconnects without closing the
  menu, and checks the player inventory and chest after rejoining. The fixture
  uses an operator grant and an empty inventory; it does not cover overflow.
- Singleplayer recovery:
  `run scenario native-world-first-start-interrupted --client native --windowed`
  interrupts the server only after both the first-start marker and database exist;
  `run scenario native-sp-relaunch-lease-race --client native --windowed` suspends
  the old server until the new one logs its lease wait. They create private client
  data and worlds, use a private Xvfb, and do not cover the lease timeout branch or
  Windows. For the first-start negative control only,
  `SOLARIS_FIRST_START_RECOVERY_BINARY` can select an older server for the
  recovery attempt after the fixed server created and interrupted the world.
- `java-v2-p5-parity`, `java-v2-p5-towns` and `loader-v2-settlements-smoke` run the
  real Java/Loader clients under private Xvfb against a frozen candidate
  (`SOLARIS_SERVER_BINARY` or `--server-bin`, plus `--plugins-dir`); they never
  build a server.

- `native-v2-ledger-painting` places one painting and observes its removal
  after its fixture support is removed.
- `native-v2-ledger-axe` uses a fixture axe to strip a log, scrape oxidized
  copper and unwax weathered copper, observing each received block state.
- `native-v2-ledger-sponge` places a sponge beside one water source and observes
  the wet sponge and absorbed source through the graphical client's received blocks.
- `native-v2-ledger-setblock-snbt` opens a fixture chest created with `/setblock`
  SNBT and withdraws the seven diamonds received in its first slot.
- `native-v2-ledger-crafting-pickup` opens a crafting table and checks that its
  menu remains open and 16 planks are on the cursor after the first pickup.
  This does not exercise disconnect custody.

## Scene shots

`python3 -m tools.harness run scene-shot --client native --windowed` runs a
scripted capture. Set `SOLARIS_SERVER_BINARY`, `SOLARIS_CLIENT_BINARY`, the native
env block above and `SCENE_SHOT_SCENARIO` to a JSON file. The JSON has `steps`,
each one of `cmd`, `tp` (five coordinates), `look`, `hotbar`, `wait_ticks`, `wait_play`,
`wait_respawn` (waits for live health and a closed death screen), `pause` (native only), `wait_terrain`, `shot`, `observe`, `use_item_on`, `click_screen_button`,
`scan_blocks`, `read_block`, `navigate_to_block`, `list_entities`, `require_entities`, `timed_entities`,
`mob_motion`, or `key` (`keys` and `ticks` MCP arguments; released when the step finishes). Optional
`seed` defaults to 100, `world` copies an existing world, `server_view_distance`
defaults to 4, `lod_enabled` optionally sets `[lod].enabled` in the private
server config, and `lod_share_terrain_keys` optionally sets
`[lod].share_terrain_keys` when LOD is enabled. Sharing reveals seed-derived
terrain parameters to the client; use only with an isolated test world.
`client_settings` accepts `graphics_preset`, `render_distance`
and `fov` (default 70: the owner's saved fov is never inherited). Set
`RUST_LOG=mc_edge::lod=debug` to retain a server log line for each successfully
queued LOD tile response. PNG paths are
recorded in `result.json`; settings and world are isolated per run.
`tools/harness/tool_clipping_scene.json` captures the diamond pickaxe against
obsidian from far and close positions, including held attack frames.
`tool_wall_visible` checks the received target, selected tool and eye distance,
then requires turquoise tool pixels over a dark wall in the fixed close-view
frame. Inspect the attack frames separately; the pixel check covers rest only.


`require_entities` takes `name`, `args` for `minecraft_list_entities`, and a nonempty
`types` list of namespaced entity IDs. Place it immediately before a shot: it
records the client-visible entity snapshot as `<name>.json` and fails if any
required type is absent. It works with native and Java MCP clients, but presence
in the snapshot does not establish that the entity appears in the captured frame.

`use_phases` (native only) takes `name` and `ticks` (1–100), holds use across
start and hold captures, releases it even on failure, and requires the client's
`using` state to transition idle → active → active → idle.

`resize` (native only) takes `name` and `size` in pixels, resizes the owned Xvfb
window, and captures a frame whose dimensions must match the requested size.

`wait_disconnect` (native only) takes `name` and `eof`; it waits for the
disconnected/error phase and records cleared use state, requiring the transport
EOF reason when `eof` is true. `reconnect` targets the same private server.
`server_eof` half-closes every client-facing socket in the harness-owned proxy,
producing clean EOF without killing the server or approximating EOF with a reset.

`walk_capture` samples one uniquely matching entity and waits for its reported raw
walk position and minimum speed. It retains the before/after observations and
the screenshot's render-frame gait inputs; the frame, not a later state sample,
must match the requested tolerance, and the observed entity identity must persist.
Position is not wrapped: renderers can apply a factor before calculating phase.
`tools/harness/gait_phase_scene.json` fixes the scene and slows both clients'
server clock to 2 TPS so extraction can be sampled without widening the phase
tolerance. For the Java lane copy the fixture and set `java_plain` to `true`.

`goat_side_horn` checks a previously named screenshot from the fixed goat side
pose against the measured Java horn edge; its four-row bracket shifts down one
pixel when rasterization moves the horn top below the first row. It is not a
general image comparator.
`assert_chat` (native only) records the current observation and checks `contains`
and `excludes` substrings against `recent_chat`. At least one expected substring
is required; use an ordered chat fence after the commands under test before
checking absence. `tools/harness/command_feedback_scene.json` exercises player
success suppression, failure visibility and the enabled feedback control.

`assert_same_game_time` compares integer `time.game_time` from two named native
observations and requires the specified daylight change, proving a fresh clock
update arrived. The registered `native-frozen-game-age` scene fences freeze,
teleport and time-command replies across held frames and local input.

`wait_chat` (native only) takes a nonempty substring and waits on client state
notifications until `recent_chat` contains it. Use a received acknowledgement
before observing a server-driven transition; waiting client ticks does not
acknowledge a command. This checks current chat history, not freshness: use a
unique ordered fence when the same reply could already be present.

`native-low-roof-house` certifies a two-block-high enclosed house with a paired
bed. An ordered command fence and block predicates check the bed and roof before
the native client asserts the certification reply. This checks certification,
not whether a villager sleeps or breeds.

`assert_entities` (native only) takes `name`, `args` (as `list_entities`), `entity_type`
and `min`, plus optional `max`, `box` (`[x0,y0,z0,x1,y1,z1]` on the received feet
position), `max_height` (a sleeping pose is 0.2 tall), `profession` (the received
VillagerData) and `baby` (the received DATA_BABY_ID; a row that never received it matches
neither `true` nor `false`). It counts the matching received entities and fails `<name>_min` or
`<name>_max`; the artifact keeps the same-thread `game_time` and every received row.
The village scenes judge settlement behaviour with it, in game ticks on a frozen clock:
`native-village-idle-stroll` (six residents dropped 40 blocks from the bell walk back
within 32 blocks in 3000 ticks; two villagers dropped into a 4-deep pond float up within 100 ticks),
`native-village-night-door` (a door the player left open reads shut after both
residents sleep behind it at night), `native-village-late-job` (at day tick 10000 an
unbound villager and a bound resident take a free lectern),
`native-village-distant-job` (a resident claims a certified workshop 72 blocks away),
`native-village-deep-water` (residents dropped onto the floor of 2- and 5-deep ponds, with no bank cell
beside them, leave the water and sleep at home), `native-village-mob-doors` (residents shut behind
poplar, copper and waxed oxidized copper doors walk to the bell and the doors read shut afterwards;
an iron door keeps its resident in), `native-village-after-hours-job` (at Idle day tick 11000 a bound
jobless resident walks 44 blocks to a new farmer workshop and takes the profession) and
`native-village-breeding` (two bound adults ration bread from a certified storage and, in a
house whose roof sits one block over its three beds, raise a baby villager the client receives
within 3000 game ticks; the fix patch of `ISSUE-1-13` is the headroom-free spare-bed scan).
Each runs on a sky platform with spawning and time frozen; none covers terrain routes.

`upgrade_certified` (native only) takes `{"kind": "<building kind>", "level": 2|3}`:
it waits for the newest `Certified <kind> #<id>` reply in `recent_chat`, sends
`settlement building upgrade <id>` (a building id is a settlement and sequence
pair, so a fixed scenario cannot spell it), waits 20 ticks and fails the named
assertion `<kind>_upgraded_to_level_<level>` unless `Upgraded building #<id> to
level <level>.` arrived.

`native-settlement-buildings-upgrade` hand-builds a stone wall, a tower, a
barracks with two beds and a farmer workshop with a composter inside a founded
settlement and certifies each, expands the wall to the next level's size and
adds a second composter, then upgrades both through `upgrade_certified`. It
covers ISSUE-1-09 on the command path; it does not press the Buildings dialog's
Upgrade button (`army-qa` does), build with Builders (`buildings-qa`), or cover
gates, level 3 or the other workshop kinds.

`native-settlement-radius-growth` founds a settlement of radius 8 around six
villagers, then binds a seventh. `/settlement info` must show `radius=24
members=6` after founding and `radius=28 members=7` after the bind (four blocks
per resident, docs/settlements/construction.md). It reads the command reply, not
the Settings dialog or the map territory.


- `timed_entities` takes the same `name` and `args` as `list_entities` and retains
  its same-client-thread `game_time` and entity positions. Missing timestamps fail.
- `mob_motion` reads three named `timed_entities` receipts and records horizontal
  displacement and blocks per client tick for one zombie and one spider. Both
  must retain their identities and move; observed game time must advance. These
  are client-observed rates, not authoritative server-clock speeds: client time
  can be corrected by server packets, and curved paths make net displacement a
  lower bound on traveled distance.
- Zombie-family chase uses a blueprint capability to send the aggressive-arms
  mob flag. It is cleared when the target is lost or on the first hostile
  evaluation after restart, because targets are runtime-only; the changed
  appearance is journaled. The focused metadata and WAL tests do not substitute
  for a matched animated visual comparison.

- With a client supporting held MCP input,
  `{"shot":"tab","hold_keys":["key.keyboard.tab"]}` captures the visible player
  list and releases the key even if capture fails (native only).
- Terrain fence (native only): after observing the relevant block states and
  before the screenshot, use
  `{"wait_terrain":{"name":"terrain-ready","args":{"sections":[[62,10,62],[62,9,62]],"timeout_seconds":30}}}`.
  Sections are unique section-coordinate integer triples (1-64); the timeout is
  0.1-120 s, default 30, also bounded by the scenario deadline. A missing tool,
  timeout or non-ready response fails the scenario. The receipt is saved as
  `<artifact>/terrain-ready.json` and in `scenario.terrain_waits`; requested and
  mesh revisions must match. The completed frame is a readiness fence, not proof
  of visible pixels, and capture is not atomic with the fence.
- `--client java --windowed` connects a NeoForge Java client with the Loader
  automation mod to the Solaris server (`:fabric-agent:runClientMcp`): `shot`
  uses `minecraft_screenshot` and matches the 1280x720 window, fov, GUI scale 3
  and render distance. It is an instrumented rendering oracle, not an unmodified
  vanilla client or a vanilla-server AI or movement oracle. Mojang's client is a
  test-time reference only, never a source for models.
- For a Java client **and vanilla server** oracle, set `vanilla_singleplayer: true`
  in the scenario and run `--client java --windowed`. No standalone server or
  EULA file is created. Begin the steps with `ui_click` (exact screen `class`,
  `title`, and 1280×720 window `x`/`y`), `ui_wait` (screen class), and `ui_shot`
  as needed to create a world and enable commands, followed by `wait_play` and
  normal scene actions. The harness checks the title overlay, window ownership,
  actual in-play state and integrated-server log; fixed UI coordinates are tied to
  this window size and must be checked when the Java UI changes.
- `settlement-map-qa` reuses this lifecycle with a scripted body and judges the
  client's own screenshots: it founds an empty town, opens its map, certifies a
  real Storage while the map stays open (stale shot: unchanged), refreshes and
  returns to the main page; palette pixel deltas between shots prove territory,
  viewer and storage cells. `--client native --windowed` presses the dialog
  buttons and drives the native pointer (`client_pointer`): a territory cell and
  a storage cell must show tooltips, and clicking the storage must bring the
  server's building page. `--client java --windowed` uses the pages' `/settlement
  menu` commands (Loader MCP cannot press buttons inside a dialog's scroll body)
  and has no pointer tool, so hovers and cell clicks are native-only.

## Distant Horizons gate (`dh-gate`)

`python3 -m tools.harness run dh-gate --platform fabric|neoforge` runs a real
Java Distant Horizons (DH) client against a candidate server. Prerequisite
discovery alone is never a pass. The profile checks both publisher hashes (URLs,
sizes, SHA-512/SHA-1) and the baseline loader pins in
[`tools/harness/dh-artifacts.json`](../../tools/harness/dh-artifacts.json), which
is the pin owner.

- Inputs: `SOLARIS_DH_JAR` (the pinned DH jar), `SOLARIS_SERVER_BINARY` (the
  candidate), Java 25, Xvfb, Python Pillow, and the real DH scenario driver in the
  sibling `solaris-loader` (`SOLARIS_LOADER_ROOT` selects the checkout). The driver
  uses a private copy under `.analysis/prism-26.1.2/minecraft` inside that
  checkout, never the owner's Prism, and mints fresh local MCP bearer tokens per
  client.
- Scenario: independently clean DH caches; server DH disabled versus enabled from
  the same camera; vanilla render distance <= 8, DH radius 128; a never-visited
  landmark 1024 blocks away.
- Required receipts: request/DTO/render completion, vanilla-versus-DH packet
  coordinates, screenshots, surface-edit/reconnect/dimension/reset checks and a
  bounded adversarial pass. `dh-scenario.json`, `dh-packet-evidence.json` and
  `result.json` keep the actual outcomes.
- Fixed-camera pixel check: no gold in the DH-off control, gold in the cold DH-on
  frame, emerald after the live edit. A decoded leaf or replacement VBO can precede
  parent aggregation and is not a rendered-edit pass.

## Flakes that are environment

- Load above ~25 on 12 cores: the client never enters Play, or
  `stale_dimension_epoch` after a respawn. Rerun at low load before debugging.
- `renderer_initialization_failed: Parent device is lost`: transient GPU/Xvfb
  flake; rerun once.
- Under Xvfb about 25-30 ms per frame goes to `xcb_put_image`; it exists only in
  harness runs and is not client FPS cost.
- A graphical settlement war gate once failed with `stale-revision` while a cargo
  matrix ran on the same host and passed on an idle host. Run long graphical gates
  on a quiet host and count such passes as host-idle evidence.
- A debug server runs at about 2-9 TPS while a joining player's chunks generate;
  timing-sensitive checks (bow draw, eating, cooldowns) need the release server
  (AGENTS.md).

## Promotion with a negative control

A bug row closes only with a registered `harness:<profile>:<scenario>` check that
fails closed at the row's own assertion on a negative control. Build the negative
control as the current candidate with only that row's fix reverse-applied. Commits
from before the 26.3 cutover speak protocol 775 and fail at login, which is not a
negative control; bundled multi-fix reversals do not count either. A unit or
synthetic test is a regression guard, never the closing evidence for a
client-visible bug.

## village-autonomy-qa

1. Build the release server under the runner into its own target.
2. Run the fast proxy first (about 8 min, deterministic):
   `cargo test -p mc-core --lib
   seed_100_generated_village_builds_storage_with_player_simulation_distance --
   --ignored --nocapture`.
3. `python3 -m tools.harness run village-autonomy-qa --client native --windowed`
   takes 40-55 min (cap 3600 s); run it async with `timeout: 0`. Two in parallel
   are fine on the 12-core host; stop stale `solaris-v2` processes from old runs by
   PID first.
4. Receipt: `.analysis/validation/<UTC>-village-autonomy-qa-*/village-autonomy-qa/scenario.json`:
   `assertions`, `autonomy_acceptance.storage_rounds` (chest totals and buildings
   about once a minute), `client_blocks` (blueprint matches; `wood_species`),
   `planner_status`, resident census; `transcript.json` has every MCP call. Replay
   recorded `minecraft_scan_blocks` results through the harness matcher offline to
   test matcher changes without a 50-minute rerun.
5. `all_behaviours_pass` requires every recorded verdict, including ones commented
   "non-gating". Never narrow it without an owner decision.

Known causes: builders only come from residents that reach storage (a bridge exit
over water once blocked them); builders and resource workers work only in day time
2000..9000, so harness waits must span a game day; each project uses one wood
species funded from stock, so an upgrade can differ from its base level; an
autonomous upgrade clears the previous level's removed cells before it places
anything, so a level-one building matches only until its upgrade starts and again
once the upgrade completes; natural zones must prove wild resources. A chat-reply
race (`mutations_refused_while_unowned`) fails on a CPU-contended host. Lumberjack
latency varied between runs; only the final checkpoint persists, so diagnosing it
needs a run that records state over time.

To trace villagers temporarily (never merge): a `tracing::debug!(target:
"villager_trace", ...)` gated every 400 ticks in the villager brain, run with
`RUST_LOG=info,villager_trace=debug` (the harness passes the environment to the
server), then grep `server.log` per entity id. `path=0` with a fixed target for
thousands of ticks means no route; the same position with `path>0` means a
navigation stall. Compare good and bad runs of the same binary.

## Settlement and army QA false alarms

Check these before blaming the product (all happened):

- `fill` over 32768 blocks is silently rejected; chunk fills and assert air.
- A one-block step under a two-high lintel traps builders.
- Summon villagers at cell centres (`x+.5`) of free cells, not inside walls.
- Dialogs are static snapshots; read progress from a fresh menu or the ledger.
- Over-funding (1.5x) lets the planner upgrade to the next level right after
  certification; fund 1.0x.
- An oracle variable never updated in its loop makes the verdict always FAIL.
- A target outside simulation distance does not tick; keep the player near both
  site and storage.
- The client log ends at `session startup: play` by design; server logins are not
  logged at INFO.
- Never read state with `strings state.redb` (old pages); use the recovery path.
- Guard-return oracle: track one soldier UUID that leaves its post by more than 4
  blocks and returns within 2.5 after the lure dies. The `army-qa` fixture waits
  for every live guard within 2.5 blocks of its post before spawning a zombie 11
  blocks away (inside the 12-block tether); the command-order check needs an
  `Ordered` reply newer than its chat fence and a dialog roster showing Patrol for
  every soldier; Return deliberately does not engage.

For product failures, copy the receipt world into a throwaway `#[ignore]` core
test, add a ticket at the player chunk, run the real tick and log per resident;
then write a minimal failing-first test without the world, fix, delete the probe.

## Headless join smoke against any server

Proves join, auth, one action and persistence; not a graphical gate.

1. Launch as a supervised service, ready on `automation descriptor`:
   `solaris-client play --headless --profile offline-test --server <host:port>
   --username <name<=16> --automation --mcp-http 127.0.0.1:0 --artifact-dir <dir>`.
   MCP: `initialize` with the descriptor's nonce, `notifications/initialized`,
   then `tools/call`.
2. `minecraft_wait_for_play` (90-120 s). "Connection refused" right after the
   server became ready means the client launched too early: relaunch rather than
   reuse the session.
3. Auth plugin servers: `/register <pw> <pw>` or `/login <pw>` via chat, confirmed
   through `recent_chat` and `minecraft_wait_for_state_change`. Keep test
   passwords in a 0600 file under ignored `.analysis/`.
4. One action with a server-side result: scan, navigate, `minecraft_break_block`
   expecting `became_air` and `pickup_confirmed`. Spawn protection (16 blocks when
   `ops.json` is non-empty) makes a non-op dig near spawn report `started` without
   `became_air`; move away. A plant above the target blocks the look ray; pick
   another column.
5. Persistence: disconnect, stop the server via `stop` on stdin and wait for the
   checkpoint line, restart, rejoin, `minecraft_wait_for_loaded_block`,
   `minecraft_read_block`.
6. Clients older than the server's Loader channel version fail in configuration
   with `unsupported Loader channel or phase`; that is version skew, not a server
   bug.

## Manual local playable launch (owner-visible play, not a gate)

An owner-visible session on a visible display, with a copied config and release
binaries. It produces no receipt and proves nothing; the automated
`playable`/`regression` profiles and agent QA run isolated, headless-display
clients instead and must never touch the owner's desktop.

1. Prepare the server cache once:
   `solaris-v2 content import --version 26.3 --download --cache data/vanilla`
   then `solaris-v2 content verify --cache data/vanilla`.
2. Copy the manual profile and edit the copy, never `playable.toml` itself:
   `cp playable.toml .analysis/playable-run.toml`; set `world_dir`, `port` and
   `seed` (the profile is offline, loopback `127.0.0.1:25565`, view distance 4).
   A reused `world_dir` keeps its old seed and generation contract; use a new
   directory for a fresh world.
3. Start the server: `target/release/solaris-v2 --config .analysis/playable-run.toml`.
   It is ready when it logs that the listener is bound. Type `stop` on its stdin
   (or send Ctrl+C) to write the final checkpoint.
4. Install and start the client from the client repository's release build:
   run `solaris-launcher --headless-bootstrap` once (or the launcher window), then
   `solaris-client` (Title screen: Multiplayer, Add Server or Direct Connect) or
   `solaris-client --server 127.0.0.1:25565 --username Steve`. Use the same
   `--data-dir` as the bootstrap if you did not use the default.

## Vanilla render oracle (native visual parity)

The owner requires a side-by-side comparison with the real Java client, not
memory or wiki images. Reproducible from this repository: the two captures.
**Not reproducible from this repository:** the panel step. The owner's runners
and panel composer live in the owner-local `<work>/vref-kit` (a throwaway
Pillow script that pastes the vanilla and Solaris PNGs of each row side by side
with labels); it is not tracked anywhere. To review, put the two PNGs of the
same scenario step next to each other with any image viewer or a few lines of
Pillow, and follow the review rules below.

- Capture both clients with the same scenario JSON: `python3 -m tools.harness run
  scene-shot --client java|native --windowed` (steps in [Scene shots](#scene-shots)).
  The Java oracle uses a Loader checkout for the server's Minecraft version
  (`SOLARIS_LOADER_ROOT`).
- Same window 1280x720, FOV 70, GUI scale 3, seed, server build, render distance
  and pose for both clients. Java shots change only when the server changes: reuse
  them and reshoot native only.
- Vanilla's default graphics preset resets render distance to 16 at startup: write
  `graphicsPreset:"custom"` and fail when the post-run `options.txt` distance
  differs.
- The native Medium preset is the HDR look; Low is the vanilla-exact path (default
  since RC2). Check the preset first for global colour gaps.
- Review the panels yourself; report each item as `matches` or `differs:` with RGB
  sampled over the same rectangles (no HUD, hands or text). Never generalise a
  per-sample error beyond the samples.
- Capture native scenes only after `geometry_ready` with zero pending sections.
- Provenance (owner, 2026-11-04): never copy Mojang code, numbers, model geometry or
  UVs into sources, including values read from decompiled or `javap` classes;
  textures load from game resources at runtime. Entity rigs are authored by eye
  from public visuals, with provenance notes kept outside Git. Tell agents not to
  open decompiled sources. (This rule is about client rendering assets. Packet ids
  and layouts for the server are read from local reports and `javap`, see
  [ADR 0002](../decisions/0002-vanilla-protocol-metadata-as-reference.md).)
- The owner workstation cannot reach minecraft.wiki; reference files were fetched
  through the test server with `ssh "$SOLARIS_TEST_HOST" "curl ..."` into
  `/tmp/wiki-ref` there and rsynced back. That needs the owner's SSH access.

## Combinatorial scenario matrix with bots

**Not runnable from `main`.** The matrix generator (`gen_matrix.py`), the
`matrix.yaml` and append-only `ledger.jsonl` it produced, and the shared harness
API (`tools/harness/matrix_api.py` with the `protocol-matrix`, `java-matrix` and
`native-matrix` profiles) lived on branches and in the owner's `<work>/scenarios/`;
none of them is in this repository, and `python3 -m tools.harness list` has no
such profile. The supported replacement is the registered harness scenarios
(`python3 -m tools.harness explain`) and [catalog load](catalog-load.md). Rules
from that matrix that still bind any new gameplay row:

- One runner: the canonical harness. No second runner or DSL.
- An adapter's oracle returns non-empty checks; a missing verb raises a named
  `MissingCapability` and never passes. A command's feedback is not a success
  oracle; check world state. Input verbs are submission-only; wait for fresh
  state. A forceload acknowledgement is not a loaded-chunk fence.
- kill -9 durability: wait until the published journal batch covers the mutation
  tick, then SIGKILL; the mutation must survive. A mid-action crash must recover
  consistent state; losing up to the documented WAL tail is allowed.
- Coverage classes: spec_only / stub / real_partial / complete / executed
  (pass|fail). A registration is not coverage.
- Disjoint port ranges per runner; plugin-sensitive rows record a plugin-set
  fingerprint. No bots on the live test-server world.
