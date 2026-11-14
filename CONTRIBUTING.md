# Contributing to Krasnokamen

Krasnokamen is developed by its owner and a fleet of coding agents. Read
[AGENTS.md](AGENTS.md) for the working contract, project invariants and
validation tiers, and use the [documentation index](docs/README.md) to find the
guide for the area you change. Keep changes focused; discuss material scope or
design choices before expanding an issue. Core production builds must not
require sibling repositories.

## Contributors and the owner

Outside contributors fork the repository, work on a branch and open a pull
request. The repository owner reviews and merges; nothing lands on `main`
without that review, and pushing, tagging and publishing releases are owner
actions. Make each commit a revertible Conventional Commit and leave hooks and
signing alone. Coding agents running on the owner's host follow
[AGENTS.md](AGENTS.md) instead, including Main's standing commit/push
authorization, worker-branch limits and the host's memory-runner and
target-directory rules. Tags and releases still need explicit owner authorization.

## Prerequisites

- **Rust** at the version pinned in [`rust-toolchain.toml`](rust-toolchain.toml)
  (rustup installs it on first use), with `rustfmt` and `clippy`.
- **Clang and mold** on Linux x86-64; the [Cargo config](.cargo/config.toml)
  links with them and sets the debug test stack size. Other platforms use their
  default linker.
- **JDK 25** (`java` and `javac`) for content import and the Java-backed
  harness profiles.
- **cargo-nextest** (`cargo install cargo-nextest --locked`): the test runner
  behind `correctness`.
- **Python 3** for `python3 -m tools.harness`.
- **Xvfb**, only for the graphical real-client gates.

Downloaded or extracted Mojang content must never enter Git.

## Development setup

Follow [Build from source](README.md#build-from-source) in the README and use
debug builds for normal development. Content import needs JDK 25 and the
licensed server bundles; see the [content cache](docs/OPERATING.md#vanilla-content-cache).

## Validation

Run the canonical harness from the repository root:

```sh
python3 -m tools.harness list
python3 -m tools.harness run test -p mc-edge --lib <filter>
python3 -m tools.harness run correctness
```

Use focused tests for the edit loop. `correctness` is the L2 baseline: formatting,
code-health, strict workspace Clippy and workspace/all-target nextest tests.
Nextest policy lives in [`.config/nextest.toml`](.config/nextest.toml). Runs leave
receipts under `.analysis/validation/`; preparation-only output is not a pass.
On the shared owner workstation, use the memory runner described in AGENTS.md.
Documentation-only changes need `python3 tools/doclinks.py` (links, anchors and
reachability from AGENTS.md), not a Cargo build.

Graphical, oracle and performance gates prove only their named scope. Consult
[agent tooling](docs/AGENT_TOOLING.md) and the [break-it QA protocol](docs/QA_AGENT_PROTOCOL.md):
player-visible changes and releases need a separate breaker, not just their own
happy-path pass. Do not widen a bound, add retries or replace real-client
coverage with protocol bots to turn a failing gate green.

## Changes and commits

- Add focused regression coverage for behavior changes and update the owning
  operator guide or ADR in the same change. Remove superseded paths rather than
  keeping duplicate authorities or compatibility shims.
- Preserve unrelated local changes. Do not stage ignored agent state, runtime
  worlds or Mojang content; change `Cargo.lock` only for a concrete dependency reason.
- Use one revertible Conventional Commit per completed checkpoint.

## Documentation

The [documentation index](docs/README.md) separates operator/developer guides,
ADRs, harness inputs and release notes. Historical process logs are local archives,
not current specifications. Architecture, authority, threading or persistence
policy changes must update the owning ADR; do not treat a planned migration as
current runtime behavior.
