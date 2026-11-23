# Decoder fuzzing (cargo-fuzz)

Coverage-guided fuzzing of the bytes a client can make the server decode. This crate has its
own `[workspace]` and is not a member of the root workspace, so L2 and code-health never build
it. CI and nightly use the same six legs; their execution contract is below.
Install `cargo-fuzz` (CI 0.13.2) and nightly (CI `nightly-2026-11-09`);
the server and other gates retain their pinned 1.99 toolchain.

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

Run from `fuzz/` using the pinned CI nightly for reproducibility:

```sh
cargo +nightly-2026-11-09 run --release --example seed_corpus -- corpus
cargo +nightly-2026-11-09 fuzz run <target> -- \
  -max_total_time=720 -rss_limit_mb=2048 -timeout=20 -max_len=16384 -print_final_stats=1
```

On CI's x86_64 Linux runner, add `--target x86_64-unknown-linux-gnu` because
binstall's musl-built cargo-fuzz otherwise selects a target ASan rejects.
Local `cargo install cargo-fuzz` uses the host target; do not force CI's triple
on macOS or aarch64.
`edge_play` adds `--dev --features edge`: optimized core/Wasmtime coverage
compilation exceeds the CI runner's memory. `cargo fuzz build` builds the other
targets. `-malloc_limit_mb=32` optionally makes a >32 MB single allocation a finding.

Owner host: Main's absolute `tools/dev/mem-run.sh -m 6G` wraps each heavy command;
use `--dev` for fuzz and omit `--release` for the seed example under the
owner-host release-build policy. The guard rejects ad-hoc release
`cargo build|test|run`; it does not mechanically reject `cargo fuzz run` without
`--dev`, so that policy still applies. Dev executes fewer inputs/second: local
clean smoke is not the CI release window and never proves absence of decoder bugs.

## CI and corpus

`.github/workflows/ci.yml` is the canonical leg definition:
push to main/tags/PR = 60 s per target, `fail-fast: false`; nightly
(`fuzz-nightly.yml`, cron `17 3 * * *`) = 900 s, dispatch `seconds` overrides.
Keep the workflows in step. `release-build` depends on fuzz.
Cargo-fuzz supplies sanitizer/coverage RUSTFLAGS, not workspace `-D warnings`.
Every leg uses the bounds above: crash/OOM/20-second input/hang fails the run;
`fuzz-<target>-failure` retains artifacts and log for 14 days.

Corpus cache keys are `fuzz-corpus-<target>-<sha>` with prefix restore: immutable
cache entries accumulate and newest matching corpus is restored. Cold corpora
are seeded by the real packet/NBT encoders through `seed_corpus`.

## Findings

A crash lands in ignored `fuzz/artifacts/<target>/`. Replay with
`cargo +nightly-2026-11-09 fuzz run <target> <artifact>`, minimize with
`cargo +nightly-2026-11-09 fuzz tmin <target> <artifact>` (edge adds
`--dev --features edge` to both; owner-host rules above still apply).
Add the minimized bytes to an error-expecting regression in the owning
protocol/NBT/Loader/edge crate, fix the decoder, and [file the bug](../docs/bugs/README.md);
only `ledger promote` closes it. Parser fixes are separate from fuzz-tooling changes.
