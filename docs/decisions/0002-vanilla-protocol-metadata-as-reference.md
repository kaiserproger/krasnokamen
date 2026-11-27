# ADR 0002 — Vanilla protocol metadata as a reference

**Date:** 2026-05-12  
**Status:** Accepted; in force. Extends [ADR 0001](0001-vanilla-data-as-runtime-input.md).

## Decision

Minecraft Java 26.1 and later ship unobfuscated. The structural metadata of the
official jars is therefore public documentation of the same identifiers that
minecraft.wiki names, and reading it is allowed as a reference for protocol code:

- running `javap`, `unzip`, `strings` and similar tools on a local
  `client.jar`/`server.jar` to read class and field names, field types, record
  component order, enum members, static `int`/`String` constants and packet
  registration tables; `tools/dump-vanilla-protocol.sh` writes such a summary
  to the ignored `.analysis/protocol-dump.txt`;
- comparing those ids and layouts with the codecs in
  `crates/mc-protocol/src/packets/`.

Still forbidden:

- translating decompiled source into Rust, or copying algorithms (lighting,
  RNG, worldgen formulas) out of decompiled classes (ADR 0001);
- Mojang class names as Rust identifiers: vanilla `ClientboundLoginPacket` is
  our `packets::play::LoginPlay`; cross-references go in comments;
- naming constants after internals that are not part of the wire format;
- committing extracted classes, dumps or captures; they stay in ignored
  `.analysis/` or receipts outside Git.

A packet id or layout derived this way names the vanilla class and the local
`javap` capture in the codec's comment or commit, so the next maintainer can
re-derive it. The layout fact itself lives at the codec and its test (for
example `UpdateEnabledFeatures`, `crates/mc-protocol/src/packets/configuration.rs:140`
and its layout test at `:579`; the chunk-section fluid count,
`crates/mc-world/src/wire.rs:15-21`), not in this ADR.

## Why

A short wire capture cannot observe rare packets, Play alone has about 180, and
one wrong id disconnects a real client. Metadata cross-checks every packet at
once. The owner asked for this during the M1.g.4 wire validation.
How to apply it: [protocol changes](../development/protocol-changes.md).
