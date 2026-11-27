# Real-client gates in practice

What counts as evidence is fixed elsewhere: the [evidence decisions](../../DECISIONS.md#evidence)
(actual-client gates, release server, breaker), the
[bug regression contract](../bugs/README.md), the
[QA protocol](../QA_AGENT_PROTOCOL.md) (gate hygiene, negative controls,
variance) and [agent tooling](../AGENT_TOOLING.md) (harness commands, receipts,
Java/Loader client variables). This page is the working recipe and the traps
for graphical and native-client gates.

Per-scenario facts are not copied here: `python3 -m tools.harness explain <id>`
prints each registered scenario's fixture, sources, what it proves and what it
does not, and the check rationale sits in the docstrings of
`tools/harness/backends/scene_shot_checks.py` and the owning backend.

## Prerequisites

Everything below is reproducible from public sources except lines marked
**owner-local**. Sibling repositories, checked out beside this one unless an
environment variable says otherwise: the native client
([krasnokamen-client](https://github.com/kaiserproger/krasnokamen-client)), the
Loader ([krasnokamen-loader](https://github.com/kaiserproger/krasnokamen-loader))
and the first-party plugins
([krasnokamen-default-plugins](https://github.com/kaiserproger/krasnokamen-default-plugins)).

### 1. Build the release server and the native client

Gates use release binaries: a debug server drops to 2–9 TPS while a joining
player's chunks generate, so timing-sensitive checks (bow draw, eating,
cooldowns) fail on it.

- **Owner host:** pin, never build ad hoc (the runner refuses a bare
  `cargo build --release`, owner 2026-11-15).
  `<work>/solaris/tools/dev/pin-release <tree>` builds once per tree (HEAD plus
  the hash of uncommitted changes), copies the binary to
  `<work>/pins/<sha12>[-<diff8>]/` with a manifest, prints
  `KRASNOKAMEN_SERVER_BINARY=<pin>/krasnokamen` and reuses an existing pin for
  the same tree and build parameters; `--client` does the same for the client
  checkout (`KRASNOKAMEN_CLIENT_BINARY=<pin>/krasnokamen-client`). Pins are
  immutable; builds are incremental unless `--no-incremental` (performance
  windows and `tools/battle/run-battle.py` need non-incremental). The harness
  refuses binaries inside pooled `targets/slot-*`. A negative-control tree needs
  its own pin. `regressions-client`, `ledger probe` and `ledger promote` check
  the pin's manifest, checksum and ledger registration against the tested tree
  and print the exact `pin-release <tree>` command when it is missing.
- **Elsewhere:** `cargo build --release --locked -p mc-edge --bin krasnokamen`;
  the client builds in its own repository.
- The client links ALSA on Linux and needs the distro development packages its
  release workflow installs (`pkg-config libasound2-dev libudev-dev
  libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxi-dev
  libxrandr-dev libgl1-mesa-dev libvulkan-dev cmake`). Windowed gates need a
  Vulkan 1.1+ driver; Mesa `lavapipe` works, slowly.
- **ALSA sysroot (owner-local workaround).** Without root, unpack the distro's
  `libasound2-dev` (and `libxkbcommon-dev`) into a directory and export
  `PKG_CONFIG_PATH=$B/pkgconfig LIBRARY_PATH=$B` with
  `B=<sysroot>/usr/lib/x86_64-linux-gnu` before building or pinning; the owner's
  copy is the client checkout's ignored `.analysis/build-support/`. Missing
  `PKG_CONFIG_PATH` makes `alsa-sys` panic, missing `LIBRARY_PATH` fails linking
  `-lasound`. Check the binary's mtime afterwards: an early-exiting wrapped build
  leaves the old binary.

### 2. Build the client data root from public sources

The client refuses to start without a complete data root. A release build of
the launcher produces it:

```sh
krasnokamen-launcher --headless-bootstrap --data-dir <root>      # needs internet access
```

`--headless-bootstrap` is the whole first-run install without a window;
`--force` reinstalls, `--manifest URL` overrides the baked-in content manifest.
It downloads the Krasnokamen facts bundle and, from Mojang's endpoints, the 26.3
`client.jar` and asset objects, and unpacks textures, models and fonts; no
Mojang account is involved and no Mojang bytes enter Git. Layout
(`launcher/src/paths.rs` in the client repository):

```text
<root>/facts/                     Krasnokamen facts tables
<root>/data/vanilla/reports/      blocks.json, block_mining.json, block_light.json, registries.json, ...
<root>/resources/assets/          textures/models/fonts unpacked from client.jar
<root>/assets/asset-index.json    Mojang asset index (+ objects/)
<root>/mojang/versions/26.3/      version json and client.jar
<root>/cache/  installed.json     launcher state
```

`<root>` defaults to `$XDG_DATA_HOME/krasnokamen-client`. Offline, the client's
alternative is `krasnokamen-client setup --from-dist DIST --assets ASSETS`. The
owner's `<work>/qa-armada/template-data` is only a kept launcher output. The
singleplayer content bundle and facts publishing belong to the client
repository, not to gate prerequisites.

### 3. Environment block

Keep one env file per gate wave and source it; never retype paths.

```sh
export KRASNOKAMEN_SERVER_BINARY=<pin>/krasnokamen
export KRASNOKAMEN_CLIENT_BINARY=<pin>/krasnokamen-client
export KRASNOKAMEN_CLIENT_ROOT=<client checkout>                    # default: beside the main checkout
export KRASNOKAMEN_CLIENT_DATA_DIR=<root>/data/vanilla              # the vanilla metadata dir, not <root>
export KRASNOKAMEN_CLIENT_RESOURCE_ROOT=<root>/resources
export KRASNOKAMEN_CLIENT_ASSET_ROOT=<root>/assets
export KRASNOKAMEN_CLIENT_ASSET_INDEX=<root>/assets/asset-index.json
export KRASNOKAMEN_SERVER_VANILLA_DATA_DIR=<content-root>/26.3      # Java probes/survival: canonical content with adjacent caches
export KRASNOKAMEN_LOADER_ROOT=<krasnokamen-loader checkout>        # only to select another checkout
export KRASNOKAMEN_DEFAULT_PLUGINS_ROOT=<krasnokamen-default-plugins> # standard-pack and plugin gates; same default
```

Sibling discovery (`tools/harness/runtime.py::sibling_root`) resolves beside the
main checkout, also from worktrees.

- A headless native run needs `KRASNOKAMEN_CLIENT_DATA_DIR` with its block and
  registry reports; `--windowed` also needs the resource root, asset root and a
  nonempty asset index with objects. Missing or incomplete paths fail before
  launch. Native profiles not on the allow-list are rejected
  ([harness entrypoint](../AGENT_TOOLING.md#harness-entrypoint)).
- Native profiles, and every client run of `ledger probe`, `ledger promote` and
  `run regressions`, create a temporary private XDG root whose
  `krasnokamen-client/` links (never copies) the template's content (`facts`,
  `assets`, `data`, `resources`, `mojang`), with no settings or caches, and
  remove it afterwards. Do the same by hand for ad-hoc runs: copying cost 650 MB
  per run and filled the disk, and a shared root leaks `client-settings.json`
  between gates (an FOV assertion broke that way). Without it the client looks
  under `~/.local/share/krasnokamen-client` and exits before MCP is ready.
- One wrong path (an item properties file under the wrong repository) leaves the
  client on a white "loading content" screen forever without naming the file:
  diff the env against the last green gate line by line.
- The singleplayer scenarios (`native-world-first-start-interrupted`,
  `native-sp-relaunch-lease-race`) also need an imported 26.3 cache at
  `<core>/data/vanilla` ([OPERATING](../OPERATING.md#vanilla-content-cache)),
  which they copy into the private client root as `servers/content/26.3`.

**Modded Java endpoints.** Set both `KRASNOKAMEN_JAVA_VERSION` and
`KRASNOKAMEN_JAVA_PLATFORM`; supported pairs are 26.1.2/Fabric (API 0.155.2),
26.1.2/NeoForge, 26.2/NeoForge and 26.3/Fabric or NeoForge. The launch picks the
matching `:loader-<platform>:runClientMcp` task and pinned Gradle properties;
missing, conflicting or unsupported selectors fail before Gradle starts, with no
neighbour-version fallback, and never switch the caller's task (use the matching
Loader profile/platform). `KRASNOKAMEN_CLIENT_JAR` must be the matching local
vanilla client jar. Plain vanilla `vanilla_command` accepts exact 26.1.2, 26.2
and 26.3 metadata. Java matrix groups use the same launch after their private
Xvfb starts.

**Gradle.** Java launches keep private game homes but reuse `GRADLE_USER_HOME`
(owner host: the sibling `loader-gradle-home`; an explicit home is kept). Init
scripts separate build outputs, and a matching `--project-cache-dir` separates
task history and Loom metadata, per checkout, Minecraft version and platform
(owner host `<work>/targets/loader-builds`, elsewhere the checkout's ignored
`.analysis/harness-builds`); separate output paths alone let Gradle delete the
previous endpoint's live classes as stale. Build completion is not launch
readiness: the Loader's cold build follows the
[event-only readiness contract](../AGENT_TOOLING.md#harness-entrypoint).

### 4. Which gates need owner-held access

No gate needs a Mojang or Microsoft account: the native client uses the
`offline-test` profile and Java clients use offline identities; MCP tokens and
bridge secrets are per-run random values you generate. Only the owner holds the
public test server (`KRASNOKAMEN_TEST_HOST`, SSH, `krasnokamen-auth` passwords,
`KRASNOKAMEN_SOAK_AUTH`; [test server operations](test-server-ops.md)), the
Windows VM login ([Windows and macOS client QA](windows-and-macos-client-qa.md))
and release publication ([releases](release-and-tags.md)). Never write these
values, addresses or paths into tracked files.

## Evidence classes

Keep them separate in every report:

- **Functional:** headless native client, protocol bots, unit/TCP tests. A
  headless native run records `details.headless=true` and proves received state
  only.
- **Graphical:** native `--windowed` under the harness Xvfb with retained frames
  and pixel predicates, plus a human look at at least one screenshot. Comparing
  light arrays or other internal state does not prove rendering;
  `present_requested` does not prove monitor scanout; changed pixels do not prove
  model parity.
- **Java client:** proves vanilla-client compatibility for the path it ran and is
  diagnostic for native rendering claims. Each retained release uses its own
  imported endpoint cache and adjacent translation route.

Match the original actor, candidate and failed scenario: an operator or an
independent observer does not prove a non-operator's private inventory or UI
path. A read-only instrumentation extension discloses its diff and never changes
input, rendering or network behaviour. Crowd/FPS claims retain received counts,
submitted bodies and drawn-frame evidence under the
[performance methodology](performance-windows-and-profiling.md#comparing-builds);
headless counts or a closer diagnostic fixture do not replace the failed scene.

Setup is not the assertion: await a fresh ordered command reply, then the
loaded blocks, authoritative inventory or reached position the action needs. A
forceload/teleport acknowledgement is not resident terrain or arrival.
Distinguish requested, server-accepted, collision-resolved and client-received
positions before calling corrections rubber-banding; a correction count alone
does not establish it. Native `minecraft_click_container_slot` bypasses
hit-testing: GUI-scale or layout proofs need a physical XTest click (pattern:
`_scroll` in `tools/harness/backends/native_p4.py`).

## Isolation (owner rules, 2026-10-23/24)

- **Never the owner's desktop.** Every agent client runs on its own Xvfb display
  (never `:0`/`:1`) with `WAYLAND_DISPLAY` unset; the harness forces X11
  (`SDL_VIDEODRIVER`/`GDK_BACKEND=x11`) and scene-shot fails if `client.log` says
  `video driver: wayland`. Browser/noVNC viewers need their own proven headless
  isolation ([breaker procedure](../QA_AGENT_PROTOCOL.md#breaker-procedure)).
- **Never the owner's microphone or speakers.** Every agent-launched Java client
  gets the harness's null audio ([Java launch](../AGENT_TOOLING.md#harness-entrypoint)).
  Bot fleets and QA hunts use the same: a private null ALSA config
  (`pcm.!default { type null }`, `ctl.!default { type null }`) through
  `ALSA_CONFIG_PATH`, `PIPEWIRE_REMOTE=/nonexistent`,
  `PULSE_SERVER=unix:/nonexistent`, a private `XDG_RUNTIME_DIR`; verify with
  `wpctl status` that no stream appeared. A null output sink alone isolates
  output, not capture. Audio gates record a virtual null-sink monitor, never the
  microphone (the desktop mic indicator during such runs is expected).
- **Private audio graph** (`tools/harness/screen_audio_graph.sh`): waits for
  PipeWire's socket event and readable registry before WirePlumber, then for its
  `sm-settings` metadata before any client; readiness failures abort preflight
  with daemon logs in the audio receipt directory. Runtime directories default
  to `/tmp` regardless of `TMPDIR`; an explicit `SA_SCRATCH` must keep the
  runtime path within 80 bytes (Unix socket limit). An explicitly requested
  `sound_sink` is audible only after the same preflight verifies the private
  Pulse socket, ALSA config, silent input and named sink; it cannot select a host
  device and is recorded in the client receipt. Native audible runs need the ALSA
  Pulse plugin (`libasound2-plugins`, or `ALSA_PLUGIN_DIR` for a user-local
  extraction); a missing `libasound_module_pcm_pulse.so` is not evidence of
  silent playback.
- Private ports, data roots and worlds per run; kill only processes you started,
  by PID.

## Running a gate

- On the owner host run each graphical gate through the scene slot, which holds
  one of two shared slots and raises the gate's CPU weight above tests and
  builds: `<work>/solaris/tools/dev/scene-slot.sh <work>/solaris/tools/dev/mem-run.sh -m 6G python3 -m tools.harness run <profile> --client native --windowed`.
  A CPU-rendered native client under Xvfb takes 140–170 % CPU and screen, voice
  and Java battle scenes run two or three clients: a gate that times out or
  misses frames on an overloaded host is not evidence; rerun it at normal load.
  One graphical run per agent at a time; a `-x` window goes inside the slot
  (`scene-slot.sh mem-run.sh -x …`), never the reverse.
- **Environment flakes are hypotheses, not exceptions.** Keep the first failed
  receipt, record load, cgroup/CPU pressure, renderer/backend and failure phase,
  and rerun once in a corrected environment only when that adds evidence.
  `Parent device is lost`, a missing frame or `stale_dimension_epoch` is not
  exonerated by its wording. Xvfb/software rendering and capture overhead belong
  to the recorded backend: compare like with like and never subtract a historical
  `xcb_put_image` estimate to manufacture physical-display FPS.
- **Promotion.** A client-visible row closes only through `ledger promote` with
  a registered `harness:<profile>:<scenario>` check that fails at the row's own
  assertion on a control tree with only that fix reversed; controls, client
  fixes (`client:<sha>` with control/restore receipts of pinned clients) and
  backfills follow the [ledger contract](../bugs/README.md#the-fixed-rule-single-home).
  Commits from before the 26.3 cutover speak protocol 775 and fail at login,
  which is not a control; a unit or synthetic test is a regression guard, never
  the closing evidence for a client-visible bug.

## Native scenario rules

- Native runs the client headless (`play --headless`) against the
  harness-started server over an ephemeral loopback MCP endpoint (no Java, no
  Xvfb). Descriptor readiness uses inotify complete-write events and the process
  pidfd, then authenticated MCP initialization; scenario readiness uses MCP state
  events, never sleeps. Each run's descriptor/token directory lives under the
  client's `.analysis/harness-runs/`; the 0600 bearer token and nonce are not
  receipted.
- `--windowed` owns Xvfb, launches `play --windowed` and, after success, retains
  `screenshots/native-final.png` with a JSON receipt (frame id,
  scene/state/resource revisions, GPU completion); missing capture evidence
  fails the run. Headless runs make no visual claim; `--windowed` adds the final
  capture, not pixel parity.
- `survival` uses an isolated seed-81 world with no plugins, no operators and
  hostile spawning disabled. The generic fallback builds a debug server, which
  does not satisfy real-client gates: pass the pinned release server through
  `KRASNOKAMEN_SERVER_BINARY` (path and SHA-256 are receipted). `--plugins-dir`
  becomes `[plugins].directory` and supplies no plugin-specific scenarios.
- Operator fixtures use vanilla `ops.json` beside the isolated config (level 4,
  offline identities recorded). Headless survival grants no operator; windowed
  survival grants its one identity only for the labelled death fixture, after
  ordinary mining/pickup/placement acceptance.
- Respawn uses the bounded event-driven Play wait. `in_play=false` right after
  the Respawn click is not a disconnect, but there is no protocol respawn
  fallback: a failed UI request, Play timeout or missing restored health fails.
- Rendered name/text/colour/glow, piglin admiration, merchant offers and similar
  visuals need screenshot review; inventory debit or menu admission is not
  visual acceptance. Action or renderer gaps fail closed.
- Java/Loader v2 gates (`java-v2-p5-parity`, `java-v2-p5-towns`,
  `loader-v2-settlements-smoke`) run real clients under private Xvfb against a
  frozen candidate (`KRASNOKAMEN_SERVER_BINARY` or `--server-bin`, plus
  `--plugins-dir`) and never build a server.

## Scene shots

`python3 -m tools.harness run scene-shot --client native|java --windowed` runs a
scripted capture from `SCENE_SHOT_SCENARIO=<json>` (registered scenes:
`run scenario <id>`, mapped in `tools/harness/backends/scene_shot_scenario.py`),
with the server/client binaries and the environment block above. Settings and
world are isolated per run; PNG paths land in `result.json`. Step schemas and
validation live in `tools/harness/backends/scene_shot_steps.py`.

Scenario options: `seed` (default 100), `world` (copy an existing world),
`server_view_distance` (default 4), `lod_enabled` / `lod_share_terrain_keys`
(the latter reveals seed-derived terrain parameters: isolated worlds only),
`client_settings` (`graphics_preset`, `render_distance`, `fov` default 70, never
the owner's saved fov; `language`), `server_console` (stdin on
`RUN_DIR/console.fifo` for console fixtures), `operator_level`,
`vanilla_singleplayer`, `java_plain`, `java_hide_chat`.
`RUST_LOG=mc_edge::lod=debug` logs each queued LOD tile response.

| Steps | Purpose |
|---|---|
| `cmd`, `tp` (five coordinates), `look`, `hotbar`, `key` (`keys`, `ticks`; released when the step ends), `hold_keys` (released even on failure), `use_item_on`, `attack_entity`, `click_screen_button`, `open_chat` (`command` bool), `navigate_to_block`, `pause` (native) | drive the client through ordinary input and commands |
| `wait_ticks`, `wait_play`, `wait_respawn` (live health, death screen closed), `wait_screen`, `wait_chat`, `wait_terrain`, `wait_loader_block`, `wait_disconnect` (`eof`), `reconnect`, `server_eof` | wait on client state events |
| `shot`, `observe`, `scan_blocks`, `read_block`, `list_entities`, `timed_entities`, `require_entities`, `toggle_debug_overlay`, `resize` (native), `render_draw_sample` | capture frames and state |
| `assert_block`, `assert_chat` (native), `assert_chat_components`, `hover_chat_component`, `assert_entities` (native), `assert_observation`, `assert_player_sleeping`, `assert_render_bounded`, `assert_same_game_time`, `assert_screen`, `pixel_window`, `use_phases`, `upgrade_certified` | generic assertions |
| `walk_capture`, `mob_motion`, `mob_chase`, `death_tilt`, `dropped_clock_capture`, `falling_block_capture`, `goat_side_horn`, `goat_horns_upright`, `tool_hand_visible`, `tool_wall_visible` | fixture-specific oracles; not general comparators |
| `ui_click`, `ui_wait`, `ui_shot` | vanilla-singleplayer Java oracle UI (exact screen class/title, 1280×720 coordinates) |

Rules that bite:

- A frame-diff box must start below the horizon: sky and clouds drift between
  frames even under `tick freeze`; keep the box on the hand or terrain
  (`issue1-02-held-tool`, 2026-11-24).
- `wait_chat` sees current chat history, not freshness: use a unique ordered
  fence when the same reply could already be present, and fence after the
  commands under test before checking absence with `assert_chat`. Waiting client
  ticks never acknowledges a command.
- `require_entities` proves presence in the client snapshot, not in the frame.
  `assert_entities` counts only received entities in the radius/box and refuses
  snapshots whose `truncated` is true or missing.
- `assert_player_sleeping` and `assert_entities` `sleeping` read received pose
  metadata (wire index 6, pose 2), never the predicted pose; missing metadata is
  an error.
- `wait_terrain` (sections as unique section-coordinate triples, timeout
  0.1–120 s) is a readiness fence on matching requested and mesh revisions, not
  proof of visible pixels; capture is not atomic with it.
- `render_draw_sample` requires the reported view to match and a newer
  `submitted_frame_id`; `assert_render_bounded` needs at least three samples
  over two views with one repeat (a view is yaw **and** pitch).
- `assert_chat_components` checks the component payload; only
  `hover_chat_component` drives the pointer and requires the shown tooltip.
- `walk_capture` resamples overlapping ticks until the render frame matches
  instead of widening tolerance, and keeps every attempt; a phase miss still
  fails within `max_ticks`.
- `upgrade_certified` takes the newest `Certified <kind> #<id>` reply and fails
  `<kind>_upgraded_to_level_<level>` without the upgrade reply.
- Wait/teardown progress files (`lifecycle-progress.json`,
  `scene-progress.json`, `scene-shot/mcp-progress.json`,
  `teardown-progress.json`) are written before each operation, so a forced stop
  keeps the last phase; proxy start, EOF and teardown are each bounded to 8 s.

**Java lanes.** `--client java --windowed` runs the NeoForge Loader automation
client (`:fabric-agent:runClientMcp`) at 1280×720, fov 70, GUI scale 3 and the
scene's render distance: an instrumented rendering oracle, not an unmodified
vanilla client or a vanilla-server AI oracle. Mojang's client is a test-time
reference only, never a source for models. With `vanilla_singleplayer: true` it
is a Java client **and vanilla server** oracle: no standalone server or EULA
file; begin with `ui_click`/`ui_wait`/`ui_shot` to create a world and enable
commands, then `wait_play`. The harness checks the title overlay, window
ownership, in-play state and the integrated-server log; fixed UI coordinates
must be rechecked when the Java UI changes.

**Scenario knobs** (beyond `explain`):

| Scenario | Knob |
|---|---|
| `native-dynamic-context` | `KRASNOKAMEN_CLOCK_TEST_FRAMES=<screenshot dir>` reruns the clock-face rejection test on retained frames (`test_scene_shot.py`); graphical receipts remain required |
| `native-world-first-start-interrupted` | `KRASNOKAMEN_FIRST_START_RECOVERY_BINARY` selects an older server for the recovery attempt (negative control only) |
| `native-death-tilt` | needs a server that keeps the dead body for its 20 dead ticks; recalibrate the Java frames with the scene minus its `death_tilt` step and `--client java` (oracle table `backends/death_tilt_oracle.py`) |
| `native-player-head-registry` | scene generated by `tools/harness/gen_player_head_registry_scene.py`; fetches a public texture from `textures.minecraft.net` |
| `java-gait-sidecar` | copy of `gait_phase_scene.json` with `java_plain` and `java_hide_chat` true |
| `native-unloaded-eye` | registered as `harness:scene-shot:native-unloaded-eye`; teardown releases the held proxy stream even on failure |

## Profiles with their own procedure

### Civilization gate

`civilization-seed-qa` prepares an operator entry (level 4) for its primary
player, because its founding fixture certifies storage with explicit bounds
(`settlement building certify storage <min> <max>`, operator-only); it does not
exercise the ordinary "Проверить здесь" path. `spawn_protection = 0` keeps the op
entry from changing spawn protection; the claim still runs as a player command.
It needs `KRASNOKAMEN_SNAPSHOT_BINARY` (the `player-scale-bots` pin) for the
birth checkpoint. Street stands are searched around each street cell's own level
(`layout::stand`), because street work can lie more than `SITE_BAND` below the
bell. Sprint, fixture-distance and house-milestone rules are in
[receipts and failure diagnosis](../AGENT_TOOLING.md#receipts-and-failure-diagnosis).

### Battle crash recovery

`run scenario battle-kit-census | battle-crash-handoff | battle-auth-hydration`
(profile `battle-crash-recovery`, ledger refs
`harness:battle-crash-recovery:<id>`): a field-1k 1500/side battle with PovRed
holding a rank, CamSpec spectating and an actual SIGKILL; census rules are in
[crash census evidence](../QA_AGENT_PROTOCOL.md#crash-census-evidence). Inputs
fail closed: `KRASNOKAMEN_SERVER_BINARY` (a pin directory with `manifest.json`;
the two runner scenarios need `pin-release --no-incremental`, also under
`ledger probe --client`), `KRASNOKAMEN_BATTLE_WORLD` and
`KRASNOKAMEN_BATTLE_WORLD_SEED` (a stopped native battle world),
`KRASNOKAMEN_BATTLE_HORIZON_CLIENT`, `_CTL`, `_RESOURCES` (the Horizon pin) and,
for the runner scenarios, `KRASNOKAMEN_CLIENT_BINARY` plus
`KRASNOKAMEN_BATTLE_RECORDER_DATA`. The auth scenario deploys `krasnokamen-auth`
from the plugins sibling and has no recorder. A failed runner scenario or cleanup
writes `battle-failure-diagnostics.json` (original traceback, process states
before/after cleanup, secondary errors); a broken control pipe during cleanup
never replaces the original failure. `ledger promote` needs a receipt from the
promoted tree's own server pin. `java-battle-end-camera`
(`java-battle-functional --client java --platform neoforge`, seed 46, 150 s)
uses the same battle inputs plus `KRASNOKAMEN_BATTLE_PACK_ROOT` and
`KRASNOKAMEN_BATTLE_HIT_COUNTER`; a battle pass without its late-capture
assertion is not that regression's proof.

### Mixed-version Loader gates

- `loader-fabric-mixed-interaction` (`loader-live` lifecycle, explicit release
  server and plugin directory): Fabric 26.3 and 26.1.2 clients in one fresh
  world; server-log admission records must bind the clients' names and distinct
  connection IDs to protocols 777 and 775. `A8_NEGATIVE_CONTROL=no-drop` must
  fail the recipient inventory oracle; `A8_NEGATIVE_CONTROL=same-version`
  launches both as 26.3 and must fail the protocol admission assertion. No 26.2
  gameplay claim; not the freeze-pin recheck.
- `loader-fabric-mixed-version-build` (`loader-live`, `KRASNOKAMEN_JAVA_VERSION=26.3`,
  `--platform fabric`, exact `--server-bin`, empty `--plugins-dir`): after the
  26.3 client enters Play it builds and starts Fabric/SVC 26.1.2; the first
  client must stay in Play and keep every path and SHA-256 of its Loader class
  tree. Registered regression for `HARNESS-LOADER-SHARED-BUILD-DIR-RACE`; a
  same-version voice pair is no substitute. Provision the SVC artifact as for
  `screens-voice-qa`.

### Screens and voice (`screens-voice-qa`)

`--client native --windowed --parts 2` picks endpoints per leg: screens on
26.3/Fabric; voice on the SVC artifact's baseline (26.1.2/Fabric, the owner's
PrismLauncher case). A conflicting explicit Java selector is rejected. The SVC
jar comes from the harness's pinned artifact path or `KRASNOKAMEN_SVC_JAR`;
publisher size and both hashes are checked before staging. The fixture binds
SVC's group action to `G` in each `options.txt`. The native reconnect check
records cleared voice registration, server support, listening and microphone
HUD after disconnect and requires them to stay off on the next non-voice server;
a fresh `registered=true` capability advertisement in Play is not stale state.
The screen check waits for completed Loader acknowledgement and then records
whether `computer:monitor` was admitted: missing content after that handshake is
a false verdict, not a wait.

### `settlement-map-qa`

Privileged fixture commands found and certify a town; then only physical input
drives the owned window: bell → hub, dashboard, map (a house cell found by its
palette pixels), building page, L2 order review and receipt, shortage icons and
material detail, Storage (a shift-click moves and conserves one real stack),
construction progress against independently observed blocks, Residents,
Household, Resources, and a production order on a certified crafting-table
workshop whose bound resident must craft for it. Native uses the renderer's UI
nodes; Java locates labels by the running version's bitmap font and takes
`-- --gui-scale 2|3 --language en_us|ru_ru --size 1280x720|1920x1080` (native
always sends `en_us`). `--wrong-click` is the negative control.

`details.verdicts` (`bell_opens_hub`, `dashboard`, `map`, `building_order`,
`shortage_icons`, `storage_transfer`, `project_progress`, `residents`,
`resources`, `production_order`, `localized_text` for `ru_ru` runs, `all`) are
published whether the run passes or stops: `false` when a check failed (or an
earlier one did), `null` when the run stopped without a failed check; the ledger
reads `false` as red and `null` as no evidence. A Java client that cannot load
the requested language fails before connect (`check_java_language`; usual cause:
missing `minecraft/lang/<locale>.json` objects in the asset store, repair it).

| Scenario (`-- --scenario`) | What it judges |
|---|---|
| default | the walk above; most verdicts run native with `--timeout-seconds 3600`; `localized_text` runs Java `--language ru_ru` (owner host: source `<work>/receipts/BuildHost/java-prereqs.env` for the Java prerequisites) |
| `quickwins --language ru_ru` (native) | the D63 quick wins, written from the decision text by an agent that did not see the implementation (criteria and sources in `backends/settlement_quickwins_qa.py`); stops at the first unmet `QW <id>` (`details.client_evidence.quickwins.first_unmet`); `--survey` records every unmet criterion and is never acceptance; control = the same run on the tree without the quick wins, with its own pin |
| `quickwins-c1-race [--founding-clocks 1..6] [--found-after-ticks 1..1200]` | `JAVA-CLIENT-MISSES-FOUNDING-STARTER-CHEST-BLOCK`: observer clocks churn the anchor chunk, then only C1 is judged; `starter_chest_race` is `true` (seen), `false` (seen only after a chunk resend), `null` otherwise |
| `stage2 --language ru_ru` (native, Java) | stage-2 certification (standing inside, «Проверить здесь») by a non-operator; fixtures through `server_console` (`settlement_quickwins_console.ConsoleFixtures`, replies between two unique `say` fences) |
| `stage2op` | the same player at ops.json level 2 (`operator_level: 2`), judging only the operator branch of the coordinate form of `/settlement building certify` |
| `blueprints --language ru_ru\|en_us` | the Construction blueprint list read page by page: no raw ids or «Unknown», every name localized, 26 or more distinct |

Tab-completion denial has no client tool; `mc-edge` `command_tree` tests cover it.

### `settlement-quickwins-horizon`

`python3 -m tools.harness run settlement-quickwins-horizon --timeout-seconds 3600`
runs the quick-wins walk on the pinned Horizon 26.3 client (`--offscreen` under
the harness Xvfb; `KRASNOKAMEN_HORIZON_CLIENT|CTL|RESOURCES` or
`--horizon-client|--horizon-ctl|--horizon-resources`); `-- --scenario stage2|stage2op`
runs the stage-2 walks (Tab completion recorded, not judged). The harness owns
server, Xvfb and client in one `ExitStack` and receipts the tree fingerprint and
server path/sha256. Waits block on the client's
`wait-for state-change --since REV` (a read-only extension in the QA kit at
`targets/horizon-qa-kit`). A `ru_ru` run refuses before connect when the client
resources lack `assets/minecraft/lang/ru_ru.json`.

### Distant Horizons gate (`dh-gate`)

`python3 -m tools.harness run dh-gate --platform fabric|neoforge` runs a real Java
Distant Horizons client against a candidate; prerequisite discovery is never a
pass. [`tools/harness/dh-artifacts.json`](../../tools/harness/dh-artifacts.json)
owns the publisher hashes and loader pins the profile checks. Inputs:
`KRASNOKAMEN_DH_JAR`, `KRASNOKAMEN_SERVER_BINARY`, Java 25, Xvfb, Pillow and the
Loader's DH scenario driver (private Prism copy under the Loader checkout's
`.analysis/prism-26.1.2/minecraft`, fresh MCP tokens per client). The scenario
compares server DH off and on from one camera with clean DH caches (vanilla
render distance ≤ 8, DH radius 128, a never-visited landmark 1024 blocks away)
and keeps request/DTO/render completion, packet coordinates, screenshots,
surface-edit/reconnect/dimension/reset checks and a bounded adversarial pass
(`dh-scenario.json`, `dh-packet-evidence.json`). Pixel check: no gold in the
DH-off control, gold in the cold DH-on frame, emerald after the live edit; a
decoded leaf or replacement VBO before parent aggregation is not a rendered edit.

### village-autonomy-qa

1. Pin the release server.
2. Run the fast deterministic proxy first (about 8 min):
   `cargo test -p mc-core --lib seed_100_generated_village_builds_storage_with_player_simulation_distance -- --ignored --nocapture`.
3. `python3 -m tools.harness run village-autonomy-qa --client native --windowed`
   takes 40–55 min (cap 3600 s): launch it as a background job; stop stale
   `krasnokamen` processes from old runs by PID first.
4. Receipt `.analysis/validation/<UTC>-village-autonomy-qa-*/village-autonomy-qa/scenario.json`:
   `assertions`, `autonomy_acceptance.storage_rounds` (chest totals and
   buildings about once a minute), `client_blocks` (blueprint matches,
   `wood_species`), `planner_status`, resident census; `transcript.json` has every
   MCP call. Replay recorded `minecraft_scan_blocks` results through the matcher
   offline to test matcher changes without a rerun.
5. `all_behaviours_pass` requires every recorded verdict, including ones
   commented "non-gating"; never narrow it without an owner decision.

Known causes: builders come only from residents that reach storage (a bridge
exit over water once blocked them); builders and resource workers work only in
day time 2000–9000, so waits must span a game day; each project uses one wood
species funded from stock; an autonomous upgrade first clears the previous
level's removed cells, so a level-one building matches only until its upgrade
starts and again once it completes; natural zones must prove wild resources. A
chat-reply race (`mutations_refused_while_unowned`) fails on a CPU-contended host.
Only the final checkpoint persists, so lumberjack latency needs a run that records
state over time. To trace villagers temporarily (never merge): a
`tracing::debug!(target: "villager_trace", ...)` every 400 ticks in the villager
brain, `RUST_LOG=info,villager_trace=debug`, then grep `server.log` per entity id;
`path=0` with a fixed target for thousands of ticks means no route, the same
position with `path>0` a navigation stall.

### Edge pump lag (`join-lag-gate`)

`python3 -m tools.harness run join-lag-gate --client native --windowed` covers
`EDGE-SLOW-PUMP-HOLDS-JOIN-BEHIND-STALE-OUTPUTS` and
`EDGE-OUTBOX-STALL-WATCH-HALTS-LIVE-LAGGING-PUMP`. The test/dev-only
`KRASNOKAMEN_PUMP_LAG_TICKS=K` makes the pump wait for K core tick events before
each call to the unchanged `next_publication`, which then meets a backlog and
applies its own skip/drain: the production path stays the only path, and no
clock is read (unset or 0 keeps the shipped pump; an invalid value fails startup;
see `crates/mc-edge/src/server/pump.rs`). The gate runs `/tick rate 3` with K=8
(about 2.7 s per publication: past the 1.6 s publish budget, under the 10 s
outbox watch), opens its verdicts only on the server's `pump lag enabled` warn
line (inotify) and reads the join interval from the server's `login admitted` →
`join landed` pair.

- `join_behind_slow_pump`: a fresh client's join lands inside 10 s.
- `live_pump_does_not_halt`: `/stats` keeps advancing `simulation_tick` and the
  log has no `edge pump not draining`.

A verdict whose condition never occurred is `null` (the run fails as
inconclusive), never `false`. Options: `--tick-rate N` (1–20, default 3),
`--pump-lag-ticks N` (1–64, default 8). Control: the same command on a tree with
only the fix hunks reverted, whose server misses the join deadline and halts.
`ANTI-XRAY-BORDER-REVEAL-LOST-ACROSS-SKIPPED-PUBLISH` is not covered: a real
client receives the reveal through plain block updates in both trees, so that
row keeps its frame-level unit tests.

## Settlement and army QA false alarms

Check these before blaming the product (all happened):

- `fill` over 32768 blocks is silently rejected; fill per chunk and assert air.
- A one-block step under a two-high lintel traps builders.
- Summon villagers at cell centres (`x+.5`) of free cells, not inside walls.
- Dialogs are static snapshots; read progress from a fresh menu or the ledger.
- Over-funding (1.5×) lets the planner upgrade right after certification; fund 1.0×.
- An oracle variable never updated in its loop makes the verdict always fail.
- A target outside simulation distance does not tick; keep the player near both
  site and storage.
- The client log ends at `session startup: play` by design; server logins are
  not logged at INFO.
- Never read state with `strings state.redb` (old pages); use the recovery path.
- Guard return: track one soldier UUID that leaves its post by more than 4
  blocks and returns within 2.5 after the lure dies. `army-qa` waits for every
  live guard within 2.5 blocks of its post before spawning a zombie 11 blocks
  away (inside the 12-block tether); the command-order check needs an `Ordered`
  reply newer than its chat fence and a roster showing Patrol for every soldier;
  Return deliberately does not engage.

For product failures, copy the receipt world into a throwaway `#[ignore]` core
test, add a ticket at the player chunk, run the real tick and log per resident;
then write a minimal failing-first test without the world, fix, delete the probe.

## Headless join smoke against any server

Proves join, auth, one action and persistence; not a graphical gate.

1. Launch as a supervised service, ready on `automation descriptor`:
   `krasnokamen-client play --headless --profile offline-test --server <host:port> --username <name<=16> --automation --mcp-http 127.0.0.1:0 --artifact-dir <dir>`.
   MCP: `initialize` with the descriptor's nonce, `notifications/initialized`,
   then `tools/call`.
2. `minecraft_wait_for_play` (90–120 s). "Connection refused" right after the
   server became ready means the client launched too early: relaunch.
3. Auth plugin servers: `/register <pw> <pw>` or `/login <pw>` via chat, confirmed
   through `recent_chat` and `minecraft_wait_for_state_change`; keep test
   passwords in a 0600 file under ignored `.analysis/`.
4. One action with a server-side result, e.g. `minecraft_break_block` expecting
   `became_air` and `pickup_confirmed`. Spawn protection (16 blocks when
   `ops.json` is non-empty) makes a non-op dig near spawn report `started`
   without `became_air`; move away. A plant above the target blocks the look ray.
5. Persistence: disconnect, `stop` on server stdin, wait for the checkpoint line,
   restart, rejoin, `minecraft_wait_for_loaded_block`, `minecraft_read_block`.
6. A client older than the server's Loader channel version fails in
   configuration with `unsupported Loader channel or phase`: version skew, not a
   server bug.

## Manual local playable launch (owner-visible play, not a gate)

An owner-visible session with a copied config and release binaries; it produces
no receipt and proves nothing. Automated `playable`/`regression` profiles and
agent QA use isolated displays and never touch the owner's desktop.

1. Prepare the cache once: `krasnokamen content import --version 26.3 --download --cache data/vanilla`,
   then `krasnokamen content verify --cache data/vanilla`.
2. Copy the profile and edit the copy, never `playable.toml`:
   `cp playable.toml .analysis/playable-run.toml`; set `world_dir`, `port`,
   `seed` (offline, loopback `127.0.0.1:25565`, view distance 4). A reused
   `world_dir` keeps its old seed and generation contract.
3. `target/release/krasnokamen --config .analysis/playable-run.toml`; ready when
   the listener is bound; `stop` on stdin (or Ctrl+C) writes the final checkpoint.
4. From the client repository's release build: `krasnokamen-launcher --headless-bootstrap`
   once (or the launcher window), then `krasnokamen-client` (Multiplayer → Add
   Server or Direct Connect) or `krasnokamen-client --server 127.0.0.1:25565 --username Steve`,
   with the same `--data-dir` as the bootstrap if not default.

## Vanilla render oracle (native visual parity)

The owner requires a side-by-side comparison with the real Java client, not
memory or wiki images. Both captures are reproducible here; the owner's panel
composer (owner-local `<work>/vref-kit`) is not tracked: put the two PNGs of the
same step side by side with any viewer or a few lines of Pillow.

- Capture both clients from the same scenario JSON with
  `run scene-shot --client java|native --windowed` ([scene shots](#scene-shots));
  the Java oracle uses a Loader checkout for the server's Minecraft version.
- Same window 1280×720, FOV 70, GUI scale 3, seed, server build, render distance
  and pose. Java shots change only when the server changes: reuse them, reshoot
  native.
- Vanilla's default graphics preset resets render distance to 16 at startup:
  write `graphicsPreset:"custom"` and fail when the post-run `options.txt`
  distance differs. The native Medium preset is the HDR look; Low is the
  vanilla-exact path (default since RC2): check the preset first for global
  colour gaps. Capture native scenes only after `geometry_ready` with zero
  pending sections.
- Review the panels yourself; report each item as `matches` or `differs:` with
  RGB sampled over the same rectangles (no HUD, hands or text); never generalise
  a per-sample error.
- Provenance (owner, 2026-11-04): never copy Mojang code, numbers, model geometry
  or UVs into sources, including values read from decompiled or `javap` classes;
  textures load from game resources at runtime; entity rigs are authored by eye
  from public visuals with provenance notes outside Git; agents do not open
  decompiled sources. (Server packet ids and layouts come from local reports and
  `javap`: [ADR 0002](../decisions/0002-vanilla-protocol-metadata-as-reference.md).)
- The owner workstation cannot reach minecraft.wiki; reference files are fetched
  through the test server (`ssh "$KRASNOKAMEN_TEST_HOST" "curl ..."` into
  `/tmp/wiki-ref`, rsynced back), which needs the owner's SSH access.

## Combinatorial scenario matrix with bots

`protocol-matrix`, `java-matrix` and `native-matrix` share one adapter contract:
[bot scenario matrix](../AGENT_TOOLING.md#bot-scenario-matrix) and
[matrix runner status](../AGENT_TOOLING.md#matrix-runner-status). A protocol bot
pass is not graphical proof.
