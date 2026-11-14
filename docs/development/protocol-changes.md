# Changing the Minecraft protocol

Solaris speaks canonical Java 26.3 (protocol 777) and serves every release from
26.1.2 through 26.3 (775/776/777) through adjacent per-session steps. Import
installs all caches; a bump adds a step and keeps old versions. Read
[ADR 0015](../decisions/0015-multiversion-protocol-translation.md) before
changing a step. Never infer packet IDs or layouts from memory.

## Reference boundary

[ADR 0002](../decisions/0002-vanilla-protocol-metadata-as-reference.md) permits
reading official unobfuscated jar **metadata** as protocol reference: packet
registrations, class and field names and types, record order and constants.
It does not permit translating Java source or copying algorithms; keep Solaris
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
Play, and records clientbound frames. Point it at a local server of the matching
canonical protocol; it uses `mc_protocol::PROTOCOL_VERSION`, not an arbitrary
version selector. Start the intended server separately, then for example:

```sh
mkdir -p .analysis/protocol
cargo run -p mc-test-harness --bin wire-probe -- \
  --addr 127.0.0.1:25565 --server-kind vanilla \
  --scenario spawn-burst --out .analysis/protocol/vanilla.capture
```

Use `--server-kind solaris` and a separate output for the Solaris comparison.
`--play-seconds` bounds each command-fenced Play capture. Captures only show
packets actually observed; absence in a short capture does not establish an ID.
A probe is not a graphical-client acceptance test. Scenario details and paired
capture examples are in [physics oracles](../physics-oracles.md).

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
