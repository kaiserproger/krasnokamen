# Decoder fuzzing (cargo-fuzz)

Coverage-guided fuzzing of the bytes a client can make the server decode. This crate has its
own `[workspace]` and is not a member of the root workspace, so CI/L2/code-health never build it.
Needs the nightly toolchain and `cargo install cargo-fuzz`.

Property for every target: malformed or truncated input yields a decode error, never a panic,
abort, OOM or unbounded allocation.

| Target | Entry point | Reaches |
| --- | --- | --- |
| `frame_stream` | `mc_protocol::frame::try_decode_frame` | length VarInt21, compression thresholds, zlib inflate; also checks chunked == whole-stream decode and encode/decode round-trip |
| `packets_pre_play` | `Packet::decode` of every serverbound Handshake / Status / Login / Configuration packet, `CustomPayload::decode_serverbound`, `ClientInformation` | strings, identifiers, byte arrays, UUIDs, known packs |
| `packets_play` | `Packet::decode` of every serverbound Play packet | chat/command signatures, books, signs, container clicks (hashed stacks), creative item stacks, data components and the NBT they embed |
| `loader_wire` | `mc_loader::LoaderClientAck::decode`, `LoaderArtifactRequest::decode`, `decode_loader_view_client_request` | the JSON Loader messages a client sends in Configuration and Play custom payloads |
| `nbt` | `mc_nbt::read_network`, `read_network_value`, `read_named`, `read_named_document` | all NBT tags, depth/size budgets; re-encode idempotency |
| `edge_play` (feature `edge`) | `mc_edge::decode::decode_play_frame` | the edge's frame → `PlayerInput` mapping (chat validation, slot/window checks, …) |

`packets_pre_play`, `packets_play`, `edge_play` and `loader_wire` take one selector byte (index into the tables in
`src/lib.rs`) followed by the packet body.

## Build and run

From `fuzz/`; wrap in `tools/dev/mem-run.sh` on the shared machine.

```sh
cargo +nightly run --release --example seed_corpus -- corpus   # structure-aware seeds from the real encoders
cargo +nightly fuzz build                                      # every target except `edge_play`

cargo +nightly fuzz run frame_stream     -- -max_total_time=720 -rss_limit_mb=2048 -timeout=20 -max_len=16384
cargo +nightly fuzz run packets_pre_play -- -max_total_time=720 -rss_limit_mb=2048 -timeout=20 -max_len=16384
cargo +nightly fuzz run packets_play     -- -max_total_time=720 -rss_limit_mb=2048 -timeout=20 -max_len=16384
cargo +nightly fuzz run loader_wire      -- -max_total_time=720 -rss_limit_mb=2048 -timeout=20 -max_len=16384
cargo +nightly fuzz run nbt              -- -max_total_time=720 -rss_limit_mb=2048 -timeout=20 -max_len=16384

# edge_play drags in mc-core/wasmtime: rustc needs >10 GB there with coverage instrumentation at
# opt-level 3, so build it with the dev profile (mc-protocol/mc-nbt/mc-data stay at opt-level 3).
cargo +nightly fuzz run --dev --features edge edge_play -- -max_total_time=720 -rss_limit_mb=2048 -timeout=20 -max_len=16384
```

`-malloc_limit_mb=32` additionally turns any single decode allocation over 32 MB into a finding.

## Findings

A crash lands in `fuzz/artifacts/<target>/`. Minimise with `cargo +nightly fuzz tmin <target>
<artifact>`, then add a regression test in the owning crate (`crates/mc-protocol`, `crates/mc-nbt`,
`crates/mc-edge`) that decodes the minimised bytes and expects an error, and fix the decoder.
Corpus and artifacts are git-ignored.
