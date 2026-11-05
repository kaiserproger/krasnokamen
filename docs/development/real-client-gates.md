# Real-client gates in practice

What counts as evidence is fixed elsewhere: [AGENTS.md](../../AGENTS.md)
(actual-client gates, release server binary, breaker), the
[bug regression contract](../bugs/README.md), the
[break-it QA protocol](../QA_AGENT_PROTOCOL.md) (gate hygiene, negative
controls, variance) and [AGENT_TOOLING.md](../AGENT_TOOLING.md) (profiles,
client variables, receipts). This page is the working recipe and the traps.

## Evidence classes

Keep them separate in every report:

- **Functional:** headless native client, protocol bots, unit/TCP tests.
  A headless native run records `details.headless=true` and proves received
  state only.
- **Graphical:** native `--windowed` under the harness Xvfb with retained frames
  and pixel predicates, plus a human look at least one screenshot. Comparing
  light arrays or other internal state does not prove rendering.
- **Java client:** proves vanilla-client compatibility for the path it ran;
  it is diagnostic for native-client rendering claims.

Native `minecraft_click_container_slot` bypasses hit-testing; GUI-scale or
layout proofs need a physical XTest click (pattern: `_scroll` in
`tools/harness/backends/native_p4.py`).

## Isolation (owner rules, 2026-10-23/24)

- **Never the owner's desktop.** Every agent client runs on its own Xvfb display
  (never `:0`/`:1`) with `WAYLAND_DISPLAY` unset; the harness forces X11
  (`SDL_VIDEODRIVER`/`GDK_BACKEND=x11`) and scene-shot fails if `client.log`
  says `video driver: wayland`. Browser/noVNC viewers need their own proven
  headless isolation (QA protocol, breaker procedure).
- **Never the owner's microphone or speakers** (he is often in a call).
  Minimum: `ALSOFT_DRIVERS=null` for every agent-launched Java/Minecraft client
  (QA protocol). Stronger isolation used for bot fleets and QA hunts: a private
  null ALSA config (`pcm.!default { type null }`, `ctl.!default { type null }`)
  via `ALSA_CONFIG_PATH`, `PIPEWIRE_REMOTE=/nonexistent`,
  `PULSE_SERVER=unix:/nonexistent`, private `XDG_RUNTIME_DIR`; verify with
  `wpctl status` that no new stream appeared. A null output sink alone isolates
  only output, not capture. Harness audio gates record a virtual null-sink
  monitor, never the microphone; the desktop mic indicator during such runs is
  expected.
- Private ports, data roots and worlds per run; kill only processes you
  started, by PID.

## Native client environment

The harness needs a built native client and its content:

- **Build** (client repo; vendored ALSA sysroot):
  `B=<client>/.analysis/build-support/usr/lib/x86_64-linux-gnu;
  PKG_CONFIG_PATH=$B/pkgconfig LIBRARY_PATH=$B <work>/solaris/tools/dev/mem-run.sh -m 10G env
  RUSTC_WRAPPER= cargo build --release --bin solaris-client`. Missing
  `PKG_CONFIG_PATH` makes `alsa-sys` panic; missing `LIBRARY_PATH` fails linking
  `-lasound`. Check the binary's mtime afterwards: an early-exiting wrapped
  build leaves the old binary.
- **Content root.** A complete client data root lives at
  `<work>/qa-armada/template-data` (facts, assets, data, resources, mojang,
  cache, `installed.json`). Point `SOLARIS_CLIENT_DATA_DIR`,
  `SOLARIS_CLIENT_ASSET_INDEX` (`assets/asset-index.json`),
  `SOLARIS_CLIENT_ASSET_ROOT` and `SOLARIS_CLIENT_RESOURCE_ROOT` into it.
- **Fresh `XDG_DATA_HOME` per run** whose `solaris-client/` symlinks (never
  copies; 650 MB per run filled the disk) `facts assets data resources mojang
  cache`. A shared XDG leaks `client-settings.json` between gates (an FOV
  assertion broke that way). Without it the client looks under
  `~/.local/share/solaris-client` (a dangling `facts` symlink there) and exits
  before MCP is ready.
- Also `SOLARIS_CLIENT_ROOT`, `SOLARIS_CLIENT_BINARY` (release build),
  `SOLARIS_SERVER_BINARY` (release build), `SOLARIS_LOADER_ROOT`,
  `SOLARIS_DEFAULT_PLUGINS_ROOT`. Keep these in one env file per gate wave and
  source it; never retype paths. A single wrong path (for example an item
  properties file under the wrong repo) leaves the client on a white
  "loading content" screen forever without naming the file; diff the env
  against the last green gate line by line.
- Run each graphical gate through the scene slot so at most three run on the
  host: `<work>/solaris/tools/dev/scene-slot.sh <work>/solaris/tools/dev/mem-run.sh -m 6G env RUSTC_WRAPPER=
  python3 -m tools.harness run <profile> --client native --windowed`.

## Flakes that are environment

- Load above ~25 on 12 cores: the client never enters Play, or
  `stale_dimension_epoch` after a respawn. Rerun at low load before debugging.
- `renderer_initialization_failed: Parent device is lost`: transient
  GPU/Xvfb flake; rerun once.
- Under Xvfb about 25-30 ms per frame goes to `xcb_put_image`; it exists only in
  harness runs and is not client FPS cost.
- The graphical settlement war gate failed with `stale-revision` while a cargo
  matrix ran on the same host and passed on an idle host (2026-10-07). Run long
  graphical gates on a quiet host and count such passes as host-idle evidence.
- A debug server runs at about 2-9 TPS while a joining player's chunks generate;
  timing-sensitive checks (bow draw, eating, cooldowns) need the release server
  (AGENTS.md).

## Promotion with a negative control (2026-11-04/05)

A bug row closes only with a registered `harness:<profile>:<scenario>` check
that fails closed at the row's own assertion on a negative control. Build the
negative control as the current candidate with only that row's fix
reverse-applied. Commits from before the 26.3 cutover speak protocol 775 and
fail at login, which is not a negative control; bundled multi-fix reversals do
not count either. A unit or synthetic test is a regression guard, never the
closing evidence for a client-visible bug.

## village-autonomy-qa

1. Build the release server under the runner into its own target.
2. Run the fast proxy first (about 8 min, deterministic):
   `cargo test -p mc-core --lib
   seed_100_generated_village_builds_storage_with_player_simulation_distance --
   --ignored --nocapture`.
3. `python3 -m tools.harness run village-autonomy-qa --client native --windowed`
   takes 40-55 min (cap 3600 s); run it async with `timeout: 0`. Two in
   parallel are fine on the 12-core host; stop stale `solaris-v2` processes from
   old runs by PID first.
4. Receipt: `.analysis/validation/<UTC>-village-autonomy-qa-*/village-autonomy-qa/scenario.json`:
   `assertions`, `autonomy_acceptance.storage_rounds` (chest totals and
   buildings about once a minute), `client_blocks` (blueprint matches;
   `wood_species`), `planner_status`, resident census; `transcript.json` has
   every MCP call. Replay recorded `minecraft_scan_blocks` results through the
   harness matcher offline to test matcher changes without a 50-minute rerun.
5. `all_behaviours_pass` requires every recorded verdict, including ones
   commented "non-gating". Never narrow it without an owner decision.

Known causes (2026-10): builders only come from residents that reach storage
(a bridge exit over water once blocked them); builders and resource workers
work only in day time 2000..9000, so harness waits must span a game day;
construction uses the settlement's wood species; natural zones must prove wild
resources. A chat-reply race (`mutations_refused_while_unowned`) fails on a
CPU-contended host. Lumberjack latency varied between runs; only the final
checkpoint persists, so diagnosing it needs a run that records state over time.

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
- A target outside simulation distance does not tick; keep the player near
  both site and storage.
- The client log ends at `session startup: play` by design; server logins are
  not logged at INFO.
- Never read state with `strings state.redb` (old pages); use the recovery path.
- Guard-return oracle: track one soldier UUID that leaves its post by more than
  4 blocks and returns within 2.5 after the lure dies.

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
   server became ready means the client launched too early: relaunch rather
   than reuse the session.
3. Auth plugin servers: `/register <pw> <pw>` or `/login <pw>` via chat, confirmed
   through `recent_chat` and `minecraft_wait_for_state_change`. Keep test
   passwords in a 0600 file under ignored `.analysis/`.
4. One action with a server-side result: scan, navigate, `minecraft_break_block`
   expecting `became_air` and `pickup_confirmed`. Spawn protection (16 blocks
   when `ops.json` is non-empty) makes a non-op dig near spawn report
   `started` without `became_air`; move away. A plant above the target blocks
   the look ray; pick another column.
5. Persistence: disconnect, stop the server via `stop` on stdin and wait for the
   checkpoint line, restart, rejoin, `minecraft_wait_for_loaded_block`,
   `minecraft_read_block`.
6. Clients older than the server's Loader channel version fail in configuration
   with `unsupported Loader channel or phase`; that is version skew, not a
   server bug.

## Vanilla render oracle (native visual parity)

The owner requires a side-by-side comparison with the real Java client, not
memory or wiki images:

- `python3 -m tools.harness run scene-shot --client java|native --windowed`
  with a scenario JSON (`SCENE_SHOT_SCENARIO`; steps `cmd`, `tp`, `look`,
  `wait_ticks`, `shot`, `observe`, `hold_keys`). The Java oracle uses a Loader
  checkout for the server's Minecraft version (`SOLARIS_LOADER_ROOT`).
- Same window 1280x720, FOV 70, GUI scale 3, seed, server build, render
  distance and pose for both clients. Java shots change only when the server
  changes: reuse them and reshoot native only. Kit (panels, runners):
  `<work>/vref-kit/`.
- Vanilla's default graphics preset resets render distance to 16 at startup:
  write `graphicsPreset:"custom"` and fail when the post-run `options.txt`
  distance differs.
- The native Medium preset is the HDR look; Low is the vanilla-exact path
  (default since RC2). Check the preset first for global colour gaps.
- Review panels yourself; report each item as `matches` or `differs:` with
  RGB sampled over the same rectangles (no HUD, hands or text). Never generalise
  a per-sample error beyond the samples.
- Capture native scenes only after `geometry_ready` with zero pending sections.
- Provenance (owner, 2026-11-04): never copy Mojang code, numbers, model
  geometry or UVs into sources, including values read from decompiled or
  `javap` classes; textures load from game resources at runtime. Entity rigs are
  authored by eye from public visuals, with provenance notes kept outside Git.
  Tell agents not to open decompiled sources.
- The owner workstation cannot reach minecraft.wiki; fetch reference files
  through the test server (`ssh "$SOLARIS_TEST_HOST" "curl -sSL -o
  /tmp/wiki-ref/X.png 'https://minecraft.wiki/w/Special:FilePath/<name>'"`) and
  rsync them back; touch only `/tmp/wiki-ref` there.

## Combinatorial scenario matrix with bots (2026-10-24)

A matrix of gameplay rows run by protocol, Java and native bots against a
release server lived under `<work>/scenarios/` (`matrix.yaml` from
`gen_matrix.py`, append-only `ledger.jsonl`, last line per id wins, written only
through `gen_matrix.py --record`). Its shared harness API
(`tools/harness/matrix_api.py`, `protocol-matrix`/`java-matrix`/`native-matrix`
profiles) stayed on branches and is not in `main`. Rules that carry over:

- One runner: the canonical harness. No second runner or DSL.
- An adapter's oracle returns non-empty checks; a missing verb raises a named
  `MissingCapability` and never passes. A command's feedback is not a success
  oracle; check world state. Input verbs are submission-only; wait for fresh
  state. A forceload acknowledgement is not a loaded-chunk fence.
- kill -9 durability: wait until the published journal batch covers the
  mutation tick, then SIGKILL; the mutation must survive. A mid-action crash
  must recover consistent state; losing up to the documented WAL tail is
  allowed.
- Coverage classes: spec_only / stub / real_partial / complete / executed
  (pass|fail). A registration is not coverage.
- Disjoint port ranges per runner; plugin-sensitive rows record a plugin-set
  fingerprint. No bots on the live test-server world.
- The bots found real defects on a live build: an End restart failure on a
  dangling dragon-fight UUID, an entity-store stale-slot panic, and kill -9 WAL
  replay losing scoreboard and player position.
