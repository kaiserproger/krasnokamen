# Horizon `horizon:lod` acceptance (owner D64)

Two harness profiles accept the third far-terrain service (Horizon's own
`horizon:lod` channel, protocol VERSION 1, next to the native LOD and Distant
Horizons services). They were written from the owner decision, Horizon's
source at the pin (`bd431b38`, MIT: `crates/horizon-client/src/lod_net.rs`,
`lod_net/client.rs`, `net.rs`, `docs/graphics.md`, `docs/hosting.md`) and this
repository's harness, by an agent that never saw the implementation (owner
D60). Packet layouts come from that source, never from memory; the byte
layouts are pinned in `tools/harness/test_horizon_lod_wire.py`.

Contract they assume (from the owner decision and Main's amendments):

- `[horizon_lod]` in the server TOML: `enabled` (default false) and
  `distance_chunks`, validated; with the section absent or `enabled = false`
  the server sends no Offer. Enabled, the server logs one startup WARN that
  names `horizon_lod` and the seed (far tiles help recover the world seed).
- Our server brand stays as today. Enabled, the Offer (`horizon:lod`, version
  1, distance = `distance_chunks`, 8 in flight, 64 tiles/s) goes in the
  configuration phase to every connection; tiles go to any connection that
  sends valid Requests. A vanilla-like client that ignores the unknown payload
  stays connected.
- Tiles come from STORED chunks only; never-generated columns are not drawn
  (empty columns or a refusal, never invented terrain). Both profiles therefore
  pregenerate the world with `krasnokamen world pregenerate` first.

## Profile `horizon-lod` (protocol level, no client)

```sh
tools/dev/mem-run.sh -m 6G env KRASNOKAMEN_SERVER_BINARY=<release pin> \
  python3 -m tools.harness run horizon-lod
# options after `--`: --seed 42 --distance-chunks 16 --view-distance 4 --pregenerate-radius 176
```

A Horizon-like client (`tools/harness/horizon_lod_wire.py`: protocol 777,
brand `vanilla`, ClientInformation, Horizon's reply checks, at most 8 requests
in flight) against real servers on one pregenerated world. Every check runs and
is listed in `result.json` under `client_evidence.horizon_lod.checks`; the
profile fails when any fails. Expected PASS evidence:

| check | meaning |
|---|---|
| `world_pregenerated` | stored area around spawn made by the operator tool |
| `invalid_distance_0_rejected`, `invalid_distance_2000000_rejected` | server refuses to start (non-zero exit) and the log names `distance_chunks` or `horizon_lod` |
| `startup_warns_about_the_seed` | exactly one WARN line naming `horizon_lod` and the seed |
| `offer_in_configuration`, `offer_matches_configuration` | exactly one `horizon:lod` payload before FinishConfiguration; decodes as `Offer(distance = 16, in_flight = 8, per_second = 64)` |
| `tiles_are_beyond_the_view_distance` | construction guard: all compared tiles lie more than 4 chunks from the player's chunk |
| `tiles_answered_and_decode` | 8 tiles (six 1-block-stride, one 2, one 4) answered with data, each decoding under Horizon's own `decode_reply` limits |
| `stored_tiles_have_terrain` | at least 99% of the stride-1 columns carry a surface |
| `stored_top_block_matches_the_world` | at least 95% of stride-1 columns: the block under the reported top face has exactly the reported state in the stored chunk, as a second plain protocol-777 connection (view distance 8, same world, no service) receives it |
| `stored_surface_height_matches_the_world` | at least 80% within 2 blocks of the highest non-air, non-water block (foliage and plants account for the slack) |
| `stored_water_matches_the_world` | at least 90% of reported water columns have that water state at the reported water top |
| `stored_terrain_is_not_flat` | at least 4 distinct heights over the tiles (a constant fake fails) |
| `coarse_tiles_stay_inside_the_footprint` | stride 2 and 4: at least 90% of columns within one block of the footprint's lowest and highest top |
| `oracle_spawn_matches` | probe and oracle spawned at the same position (same stored world) |
| `unstored_tile_carries_no_invented_terrain` | a tile 15 chunks out (inside `distance_chunks`, outside the stored area) is refused or all 256 columns are empty |
| `tile_past_distance_not_served`, `misaligned_tile_not_served`, `non_power_of_two_stride_not_served` | refused or ignored; a valid request sent after each is still served, so silence is distinguishable from a dead connection |
| `vanilla_like_client_stays_connected` | a protocol-775 client that ignores the Offer reaches the play loop |
| `off-absent_*`, `off-explicit_*` | with the section absent and with `enabled = false`: no seed WARN, no Offer, a request gets no reply before the next KeepAlive |
| `on_server_clean_exit` | the ON server stops with exit 0 |

Negative control (today's main, tree `6b21e15b4`): the checks fail on the
missing service. See "Recorded negative control" below.

## Profile `horizon-lod-live` (real Horizon client)

```sh
tools/dev/mem-run.sh -m 12G env KRASNOKAMEN_SERVER_BINARY=<release pin> \
  KRASNOKAMEN_HORIZON_CLIENT=/run/media/kaiserroman/games/solaris-work/targets/horizon-lod-kit/horizon-client \
  KRASNOKAMEN_HORIZON_CTL=/run/media/kaiserroman/games/solaris-work/targets/horizon-lod-kit/horizon-ctl \
  KRASNOKAMEN_HORIZON_RESOURCES=/run/media/kaiserroman/games/solaris-work/targets/hzbattleprep-hz/.local/horizon-client/26.3 \
  python3 -m tools.harness run horizon-lod-live
# `-- --only off` runs the control alone; other options: --seed --view-distance 4 --lod-distance 64
#   --pregenerate-radius 400 --min-quad-ratio 1.5 --min-changed-fraction 0.15
```

The kit's `horizon-client` is the pin's build (`manifest.json`, sha256
`ac184266…`) with the read-only agent-control extension; it runs offscreen under
the harness's own Xvfb, never on a person's desktop (owner D56). Horizon joins
with render distance 4 and LOD Distance 64, once per mode, each with a clean game
folder (`server-terrain/` and BlueMap would otherwise carry far terrain over;
`blueMapAutoDetect:false`), the same seed, the same pregenerated world, and the
same pose: spectator, noon, clear weather, clouds off, 120 blocks above the spawn
column (absolute, because the world remembers the player between the runs),
looking 20 degrees down.

- **OFF control** (no `[horizon_lod]`): `lod.source == "server-terrain"` (the
  client only has what the server sent). This is the baseline. Recorded with the
  Horizon kit on a server without the service: `target_tiles` 2400,
  `sampled_target_tiles` 2400, `resident_quads` about 27 000 and the frame shows the
  9x9 sent chunks and then sky: Horizon reports every target tile sampled in OFF
  too, so `sampled_target_tiles` alone proves nothing and is only recorded.
- **ON** passes only when all hold: `lod.source == "server"` (the Offer was
  accepted), `lod.effective_distance` above four times the view distance,
  `lod.resident_quads` at least `--min-quad-ratio` (1.5) times the OFF value, no
  "far terrain from the server" decode warning in the client log, and the ON
  screenshot differing from the OFF one over at least `--min-changed-fraction`
  (15%) of the pixels of the horizon band (28%..62% of the height: below the
  toasts, above the chat), i.e. terrain drawn past the sent chunks. Both
  screenshots are kept; the visual review of `<artifact>/on/far-terrain.png`
  against `<artifact>/off/far-terrain.png` is still a human step.
- The stored area is 400 blocks (25 chunks) around spawn, so far terrain stops
  there although the offer allows 64; a pass does not claim more.
- `--min-quad-ratio` and `--min-changed-fraction` are first guesses made without
  the implementation (two sessions on servers without the service measured quad
  ratios of 0.94 and 1.14, band changes of 0.05% to 2.5%); Main may tune them from
  the first ON run's `samples` and screenshots in `result.json` but must keep
  `lod.source == "server"` and the OFF-vs-ON pixel difference.

## Recorded negative control

Both profiles were run on 2026-11-19 against a server pin of today's main
(`6b21e15b4`, `pins/6b21e15b41f3-9e669d8c-server-x86_64-unknown-linux-gnu-incr`),
which has no `horizon:lod` service (an unknown `[horizon_lod]` section is only
warned about). Both fail, for the stated reason:

- `horizon-lod` (artifact `.analysis/validation/20261119T142311-horizon-lod-e_a1_kie`,
  seed 42, 139 s): world pregenerated and every OFF check green (no Offer, no seed
  WARN, requests unanswered, the vanilla-like client stays connected), but
  `invalid_distance_0_rejected` and `invalid_distance_2000000_rejected` (the server
  started), `startup_warns_about_the_seed` (no WARN), `offer_in_configuration` (0
  `horizon:lod` payloads; the configuration channels are `minecraft:register`,
  `krasnokamen:loader/probe`, `minecraft:brand`) and, unreached, all tile checks.
- `horizon-lod-live` (artifact `.analysis/validation/20261119T142532-horizon-lod-live-2tpkhuru`,
  57 s, Horizon kit offscreen under the harness Xvfb): both sessions report
  `lod.source == "server-terrain"`, `target_tiles` 2400, `sampled_target_tiles`
  2400; resident quads 24322 (OFF) and 27712 (ON); band change 0.05%. Failed:
  `on_client_accepted_the_offer`, `on_resident_far_geometry_exceeds_off`,
  `on_screenshot_shows_far_terrain_off_does_not`.

Nothing here was run against the implementation; the pass evidence above is
what Main should expect from it, and the thresholds noted as guesses are the
first thing to tune against its first ON run.
