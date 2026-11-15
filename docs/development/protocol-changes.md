# Changing the Minecraft protocol

Krasnokamen speaks canonical Java 26.3 (protocol 777) and serves every release from
26.1.2 through 26.3 (775/776/777) through adjacent per-session steps. Import
installs all caches; a bump adds a step and keeps old versions. Read
[ADR 0015](../decisions/0015-multiversion-protocol-translation.md) before
changing a step. Never infer packet IDs or layouts from memory.

## Reference boundary

[ADR 0002](../decisions/0002-vanilla-protocol-metadata-as-reference.md) permits
reading official unobfuscated jar **metadata** as protocol reference: packet
registrations, class and field names and types, record order and constants.
It does not permit translating Java source or copying algorithms; keep Krasnokamen
identifiers independent. Never commit Mojang classes, jars or extracted data.
Record the vanilla version, class and `javap` evidence when changing wire facts.

This is not permission to copy client-rendering values: for native client visual
work, the [render oracle rules](real-client-gates.md#vanilla-render-oracle-native-visual-parity)
prohibit copying Mojang code, numbers, geometry or UVs, including from `javap`.
Protocol IDs and layouts are the narrow metadata exception, not a rendering
exception. Entity rigs are authored by eye from public visuals.

## Inspect local metadata

Use a locally obtained, unobfuscated client jar for the exact version being
investigated and a compatible JDK. The script requires the path to `java` in
`JAVA25`, and derives `javap` from the same directory:

```sh
JAVA25="$(command -v java)" tools/dump-vanilla-protocol.sh /path/to/client.jar
```

The output is `.analysis/protocol-dump.txt`, local and ignored. The script
extracts protocol classes into a temporary directory, prints registration order
and declared packet fields, then deletes the extracted classes. Packet IDs are
zero-based within each state and direction. Its printed version label is
hard-coded to 26.1.2: identify the jar yourself, especially for 26.3. It writes
the same output path on each run; save separate local copies before comparing
versions. Declared fields alone do not establish every encoding detail; compare
against actual wire captures and relevant protocol metadata.

For an individual class, a compatible JDK can inspect the jar directly:

```sh
javap -classpath /path/to/client.jar -p net.minecraft.network.protocol.game.GameProtocols
javap -classpath /path/to/client.jar -p net.minecraft.network.protocol.game.ClientboundLoginPacket
```

## Capture the wire

`wire-probe` connects in offline mode, walks Handshake, Login, Configuration and
Play, and records clientbound frames. Ordinary scenarios use the canonical
`mc_protocol::PROTOCOL_VERSION`; `--scenario chunk-batch --protocol 775|776|777`
also supports exact endpoint batch captures, completing after three validated
batches and ACKs. Start the intended server separately, then for example:

```sh
mkdir -p .analysis/protocol
cargo run -p mc-test-harness --bin wire-probe -- \
  --addr 127.0.0.1:25565 --server-kind vanilla \
  --scenario spawn-burst --out .analysis/protocol/vanilla.capture
```

Use `--server-kind krasnokamen` and a separate output for the Krasnokamen comparison.
`--play-seconds` bounds each command-fenced Play capture. Captures only show
packets actually observed; absence in a short capture does not establish an ID.
A probe is not a graphical-client acceptance test. Scenario details and paired
capture examples are in [physics oracles](../physics-oracles.md).

## Equippable item components

The 775, 776 and 777 `Equippable` stream layouts have the same field order:
slot, equip sound holder, optional asset ID, optional camera overlay, optional
entity holder set, five booleans (dispensable, swappable, damage on hurt,
equip on interact, can be sheared), then shearing sound holder. Slot IDs are
wire IDs, not enum ordinals. Adjacent translation maps registered sound and
entity IDs; inline sounds and named entity sets retain their identifiers.
Inspect `net.minecraft.world.item.equipment.Equippable`,
`net.minecraft.world.entity.EquipmentSlot`, and `net.minecraft.sounds.SoundEvent`
in each version's jar when changing this codec.

## Spectator camera

`ClientboundSetCamera` carries one signed VarInt entity ID. Local `javap -p -c`
inspection of `GameProtocols` and `ClientboundSetCameraPacket` in the official
26.1.2, 26.2 and 26.3 client jars establishes Play clientbound IDs `0x5d`,
`0x5d` and `0x5f`, respectively. Count common registrations and the bundle
delimiter as well as game packets. Metadata and exact commands are retained
outside Git under `receipts/SpectateFix/metadata-<version>.txt`; the counted
registrations are in `ids-<version>.txt`.

The canonical packet uses `0x5f`; the existing adjacent name-based translation
maps older IDs without changing its body. Camera frames share the entity world
FIFO, never the reliable queue: an entity must be paired before binding, and
self-camera must precede target removal. Codec and adjacent-chain tests do not
substitute for actual-client camera observation.

## Implement and test

Packet types and IDs are under `crates/mc-protocol/src/packets/`; shared codecs
and framing are in `codec.rs` and `frame.rs`. Translation and session handling
belong to `mc-edge`, not native gameplay authority. Add focused packet/codec
round-trip and malformed-input tests alongside the affected code, and exercise
both versions when translation changes. The relevant crate tests can be run with:

```sh
tools/dev/mem-run.sh -m 6G cargo test -p mc-protocol
```

Use the shared-machine runner described in [heavy commands](heavy-commands-and-memory.md)
and the [real-client gates](real-client-gates.md) for changed client-visible
behavior. Passing codec tests or a wire capture alone does not prove rendering,
gameplay, or real-client compatibility.

## Chunk batches and client-paced quota

Every nonempty publication's full chunk packets use `chunk_batch_start` (empty,
0x0c), then the chunks, then `chunk_batch_finished` (VarInt count, 0x0b).
The layouts and IDs are identical in 775/776/777 and traverse the normal adjacent
translation chain. Local metadata commands and wire receipts belong in
`receipts/ChunkBatchFix/` outside Git.

Connection-local edge state follows the measured 26.3 `PlayerChunkSender`:
initial rate 9, initial outstanding-batch limit 1; any ACK raises the limit to 10.
ACK rates clamp to [0.01, 64], with NaN selecting 0.01. While below the limit,
each publication adds the rate to quota, capped at max(1, rate); whole chunks
spend quota only after queue admission. An ACK leaving no outstanding batches
resets quota to 1. The existing 16/publication cap and world slot/byte credit
remain upper bounds. Batch framing reserves two additional slots; queue admission
is atomic and dimension fences discard whole groups, returning their ACK debt.
Unloads, light-only updates and other packets do not spend chunk quota.

Protocol drivers must ACK completed batches. The shared raw test client does so
once at network receipt, at 8 chunks/tick; callers must not ACK it a second time.
The native sibling ACKs at 4 chunks/tick, so its join terrain now follows that
rate instead of the former 16/publication maximum; vanilla/Loader clients adapt
their desired rate themselves.

## Timed battle swings (26.3)

The core `EntityEvent::Swing` carries a hand, `SwingKind` and `duration_ticks`;
edge projects all three to the existing `ClientboundSwingAnimation` codec,
not its six-tick default. Ordinary settlement swings retain that default.
Battle packets start at windup, with full cycle durations 21/23/25 and contact
offsets 10/11/12 for sword/spear/mace. The native clip owner uses the exact
contact fraction, not a generic midpoint. Bow draw is 30 ticks in a 40-tick
clip (crossbow presentation 30/75); off-hand shield cover uses 10 ticks and
parried attacks emit type NONE for 12/18 ticks of stagger.

Metadata was inspected locally with `javap -p -c` against the 26.3 server jar:
`ClientboundSwingAnimationPacket` composes entity VAR_INT, `InteractionHand`
id mapper, then `world.item.component.SwingAnimation`; the latter composes
`SwingAnimationType` id mapper (NONE=0, WHACK=1, STAB=2) and duration VAR_INT.
No new packet ID or layout is introduced. The local command receipt is
`receipts/BvCombatActions/protocol-evidence.txt` in the work root, outside Git.
Adjacent 26.1.2/26.2 clients retain their existing generic-swing downgrade;
only 26.3 carries these timing/type semantics. NONE is omitted for 26.1.2/26.2
because their generic arm animation cannot represent a no-swing stagger.

The 26.3 `SwingCommand` and `ArgumentTypeInfos` registrations were inspected
with `javap` (`receipts/PortB4/REPORT.md` in the work root): `/swing` takes
optional targets, `mainhand`/`offhand`, animation (`none`, `whack`, `stab`;
default `whack`) and duration (default 6 ticks); 26.1.2 and 26.2 have no
animation or duration children. Its `minecraft:swing_animation` argument is
registry ID 60 and is translated to a Brigadier word for older clients that
lack that parser. Real-client G2 is separate from these wire tests.
