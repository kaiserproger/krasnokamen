# Physics oracles (wire captures)

`wire-probe` (`crates/mc-test-harness/src/bin/wire_probe.rs`) logs in offline,
walks Handshake → Login → Configuration → Play and writes every clientbound
frame, so the same scripted scenario can be captured from a vanilla server and
from Krasnokamen and compared packet by packet. Captures are local evidence:
keep them under `.analysis/physics-oracles/` (gitignored) and never commit them.
Packet ids and layouts follow [decision C2](../DECISIONS.md#content-and-protocol)
and [protocol changes](development/protocol-changes.md).

## Servers

The probe speaks the canonical protocol (`mc_protocol::PROTOCOL_VERSION`, 777 =
26.3), so the oracle is a vanilla 26.3 server jar (Java 25) in its own
`.analysis/` directory, offline mode, the probe's player opped (the vanilla
setup uses ordinary commands):

```sh
mkdir -p .analysis/vanilla-physics-server .analysis/physics-oracles
printf 'eula=true\n' > .analysis/vanilla-physics-server/eula.txt
(cd .analysis/vanilla-physics-server && java -Xms1G -Xmx1G -jar ../server.jar --nogui)
```

Krasnokamen runs from a
[`pin-release`](development/real-client-gates.md#1-build-the-release-server-and-the-native-client)
binary or a debug build with `--config example.toml` (offline; set
`[data].vanilla_data_dir` if the content cache is not discovered) on another
port. Use the same deterministic setup on both and never commit their worlds.

## Running a capture

```sh
cargo run -p mc-test-harness --bin wire-probe -- \
  --addr 127.0.0.1:25565 --server-kind vanilla \
  --scenario player-shallow-water-entry --play-seconds 5 \
  --out .analysis/physics-oracles/player-shallow-water-entry.vanilla.capture
```

Repeat with `--server-kind krasnokamen` and the other address. On the owner host
the build goes through `mem-run.sh` ([heavy commands](development/heavy-commands-and-memory.md)).
Options: `--name` (login name, default `probe`), `--play-seconds` (wait per
command-fenced window), `--hold-open` (keep a passive session until SIGINT),
`--protocol 775|776|777` (only for the `chunk-batch` scenarios; every other
scenario refuses a non-canonical protocol).

## Scenarios

| Scenario | What the probe does |
|---|---|
| any other name (default `spawn-burst`) | passive: logs Play frames for `--play-seconds` |
| `player-shallow-water-entry`, `player-deep-water-swim`, `player-water-surface-exit` | confirms the spawn teleport, prepares the water fixture on vanilla, sends movement into the named cells, sprint and forward+jump+sprint input, records the post-input frames |
| `entity-land-passive-motion`, `entity-hostile-motion`, `entity-aquatic-motion` | on vanilla builds a small pool, disables random ticks and natural spawning, clears non-player entities and summons one target; the target window starts at its post-`summon` `AddEntity` |
| `item-water-drop-window` | survival mining of a dirt target at `(0, 64, 1)`, then moves into pickup range: target `BlockUpdate` to air, item `AddEntity`, item metadata, delayed `TakeItemEntity`, slot update, `RemoveEntities` |
| `collision-wall-step-fall` | teleports into each sub-window (flat floor, wall/full-block overlap, landing, water entry) and records corrections and `SetHealth` |
| `sand-gravel-fall-start` | breaks the stone supports under sand, gravel and an anvil: support/source `BlockUpdate`, `BlockChangedAck`, falling-block `AddEntity`, landing frames |
| `water-lava-replacement` | builds adjacent water/lava sources and a spread lane, breaks a dirt target next to water |
| `chunk-batch`, `chunk-batch-no-ack` | chunk batches at the exact `--protocol` endpoint, with or without the client's batch ACK |

The vanilla water fixture is the same as Krasnokamen's deterministic one:

```text
fill -8 63 -4 12 70 12 air
fill -8 63 -4 12 63 12 stone
fill -5 64 -2 -1 64 1 water
fill -5 64 3 -1 66 6 water
```

## Capture format

```text
# krasnokamen-wire-capture-v1
scenario=<name>
server_kind=<vanilla|krasnokamen|local>
addr=<host:port>
player=<login name>
protocol_version=<number>
play_seconds=<seconds>
captured_at_unix_ms=<timestamp>
packet_source=.analysis/protocol-dump.txt or javap net.minecraft.network.protocol.*
```

Frame lines are `[PLAY] id=0xNN body_len=<bytes> body=<hex prefix>`; the probe's
own serverbound packets appear as `[PLAY->SERVER:<name>]`. Selected packets get a
decoded `↳` line underneath (`AddEntity`, `SetEntityMotion`, `SetEntityData`,
`SynchronizePlayerPosition`, `BlockUpdate`, `BlockChangedAck`, container slot,
`TakeItemEntity`, `RemoveEntities`, `SetHealth`), so movement, metadata and
corrections compare without reading hex. The serverbound setup and movement lines
are part of the oracle: cite them with the clientbound frames they explain.

## Reading a comparison

- Label each compared signal `exact` (must match), `tolerance` (explicit bound on
  timing or position) or `deferred` (captured, not asserted).
- A local vanilla world can surface unrelated persisted entities around the
  summoned target; only the target's ids are oracle signals.
- A player-water capture is a wire-level correction probe: matching absence of
  post-input `SetEntityMotion` or extra `SynchronizePlayerPosition` means the
  server accepted the window, not that a client renders the same swimming. Change
  gameplay code from it only with a real-client gate or a runtime-side assertion.
- A committed test that relies on a capture cites the capture path or the `javap`
  class that established the packet; the capture itself stays local.
