# Contributing to Krasnokamen

Krasnokamen is an independent Minecraft: Java Edition server written from
scratch in Rust: parallel ticks, deterministic outcomes, WAL-backed saves and
sandboxed Wasm plugins. Players join with an unmodified client (releases
26.1.2–26.3, canonical 26.3). Read the [project README](README.md) first, then
the [documentation index](docs/README.md) for the guide that owns your area.
Coding agents should also read [docs/development/outside-agents.md](docs/development/outside-agents.md),
a distilled version of the in-repo agent guide without any private-host
tooling.

Contributions welcome: bug reports with a reproduction, focused fixes,
documentation improvements, and plugins (see [docs/PLUGINS.md](docs/PLUGINS.md)
and the `krasnokamen-default-plugins` repository). Keep changes focused;
discuss material scope or design choices in an issue before expanding.
Core production builds must not require sibling repositories.

## Prerequisites

On a clean Linux, macOS or Windows machine you need:

- **Rust** via rustup: the version pinned in [`rust-toolchain.toml`](rust-toolchain.toml)
  (channel 1.99, with the `rustfmt` and `clippy` components) is installed
  automatically on first use.
- **Clang and mold** on Linux x86-64 only: [`.cargo/config.toml`](.cargo/config.toml)
  selects them as the linker for that target. Other platforms and
  architectures use their default linker and need nothing extra.
- **JDK 25** (`java`/`javac` on `PATH`) — only for `content import` (deriving
  the game-data cache) and the Java-backed harness profiles. Plain builds and
  Rust tests do not need Java.
- **Python 3** — runs the validation harness (`python3 -m tools.harness`).
- **cargo-nextest** — the test runner behind `run test`/`correctness`:
  `cargo install cargo-nextest --locked`.
- **Xvfb** — only for the graphical real-client harness profiles, which also
  need JDK 25 and client jars; see
  [docs/AGENT_TOOLING.md](docs/AGENT_TOOLING.md#prerequisites). Ordinary
  development does not need it.

## Build

From the repository root:

```sh
cargo build -p mc-edge --bin krasnokamen
```

This produces the `krasnokamen` server binary (debug profile by default;
use `--release` only when a gate or release requires it).

## Content import (once per checkout)

Krasnokamen does not ship Mojang files. Derive the game-data cache from the
official server bundles into the checkout's `data/vanilla/` — either let the
server download them (needs JDK 25 and network access to Mojang):

```sh
cargo run -p mc-edge --bin krasnokamen -- content import --version 26.3 --download --cache data/vanilla
```

or from server jars you already own (licensed copies; `--download` cannot be
combined with `--from`):

```sh
cargo run -p mc-edge --bin krasnokamen -- content import --cache data/vanilla \
  --from /path/to/26.3/server.jar --gameplay-from /path/to/26.1.2/server.jar
```

The command installs the canonical 26.3 cache in `data/vanilla/` and, beside
it, the 26.1.2/26.2 endpoint caches and both adjacent mapping snapshots
(`data/vanilla-26.1.2`, `data/vanilla-26.2`, `data/vanilla-steps`). **The
tests read exactly these checkout paths**: not the per-user cache, and not
through a symlink (a symlinked `data/vanilla` is refused). The server also
searches the checkout, but a complete per-user cache takes precedence. The
Run command below selects the checkout cache explicitly; a configured
`[data].vanilla_data_dir` overrides that selection. Without `--cache` the
import goes to the operator default `$XDG_DATA_HOME/krasnokamen/content/26.3`
(otherwise `~/.local/share/krasnokamen/content/26.3`), which tests never read;
selection order and `--cache` are documented in
[the operating guide](docs/OPERATING.md#vanilla-content-cache), which also
covers `content verify`. **Mojang bytes never enter Git**: `data/vanilla/`,
`data/vanilla-*` and `.analysis/` are ignored — never stage them, and never
commit decompiled or extracted Mojang content. Packet IDs and layouts come
from local `wire-probe`/`javap` runs against your own downloaded jars, never
from memory ([protocol changes](docs/development/protocol-changes.md)).

## Run

```sh
KRASNOKAMEN_CONTENT_CACHE="$PWD/data/vanilla" cargo run -p mc-edge --bin krasnokamen -- --config example.toml
```

Then connect a Minecraft Java 26.3 client to `127.0.0.1:25565`.
`example.toml` listens on all interfaces with **offline authentication** — for
anything beyond local testing set `[auth].online_mode = true` and review
`[network].bind_address` (see the [operating guide](docs/OPERATING.md)).

## Validation

Run the canonical harness from the repository root:

```sh
python3 -m tools.harness list                                  # profiles and what they need
python3 -m tools.harness run test -p mc-core --lib command_summon   # L0: focused tests while editing
python3 -m tools.harness run fmt                               # L1
python3 -m tools.harness run code-health                       # L1
python3 -m tools.harness run correctness                       # L2: regressions, fmt, code-health, strict Clippy, all tests
```

- **L0** — the focused `run test -p <crate> --lib <filter>` for the scope you
  touched, plus diff/syntax checks of what you changed. Loop here while editing.
- **L1** — L0 plus `run fmt` and `run code-health`. Run when finishing a change.
- **L2** — `run correctness`: bug regressions, formatting, code-health,
  strict workspace Clippy and all nextest workspace/all-target tests. Run before
  opening a code PR. It needs `cargo-nextest`, about 12 GiB of RAM, and on a
  16-thread machine about 45 minutes with a warm target directory (a cold one
  adds the full workspace build). If your machine cannot run it, open the PR
  as a draft with your L1 receipts and say so: the PR's CI runs formatting,
  Clippy, code-health, the bug regressions and a shortened test suite, and the
  maintainer runs L2 on the merge candidate.
- Each run writes a receipt under
  `.analysis/validation/<UTC>-<profile>-<unique>/result.json` (`status`,
  `commands`, `exits`, `logs`, `details`, `error`). `prepared` or `--check`
  output is not a pass; unknown or failing commands fail closed. Quote the
  receipt path when you claim a check is green.
- Documentation-only changes need `python3 tools/doclinks.py` (markdown links,
  anchors and reachability), not a Cargo build.
- `run regressions` executes every registered bug-ledger check and is part of
  L2; run it alone only after an integration that skipped L2.
- Graphical, oracle and performance profiles prove only their named scope and
  have extra prerequisites (Xvfb, JDK 25, client jars, a release server binary
  as `KRASNOKAMEN_SERVER_BINARY`). Read
  [docs/AGENT_TOOLING.md](docs/AGENT_TOOLING.md) and
  [docs/development/real-client-gates.md](docs/development/real-client-gates.md)
  before attempting them. Do not widen a bound, add retries or replace
  real-client coverage with protocol bots to turn a failing gate green.

## Making a change

1. Find the owning page in the [documentation index](docs/README.md) and read
   it; check [open transitions](docs/STATUS.md#open-transitions) before
   touching an area that is being replaced.
2. Follow the project invariants: Mojang bytes never enter Git; the native
   store plus the tick WAL is the only save format and fails closed; gameplay
   authority stays in the native tick; no compatibility shims — migrate
   callers and delete the old path; debug builds by default.
3. Add focused regression coverage for behaviour changes: the test must go
   **red when the fix hunks are reverted** (negative control). A unit test
   cannot close a known client-visible failure — those need a real-client
   check. Substantial tests go in focused sibling `*_tests.rs` modules.
4. File or update a bug row in [`docs/bugs/ledger.toml`](docs/bugs/ledger.toml)
   for anything broken you observed — see
   [the bug ledger](docs/bugs/README.md#filing-a-bug). A fix adds `regression`
   refs. Leave `fix_commit` out of your PR: the maintainer records it after
   merging, when the commit id is final. **Never set `status = "fixed"` by
   hand**; the promotion tooling does that after running the negative control.

## Pull requests

- Fork the repository, work on a branch, open a PR. The owner reviews and
  merges; nothing lands on `main` without that review, and pushing, tagging and
  publishing releases are owner actions.
- Use one revertible **Conventional Commit** per completed checkpoint
  (`fix(scope): …`, `docs: …`). Leave Git hooks and signing alone. Change
  `Cargo.lock` only for a concrete dependency reason.
- Never stage ignored local state (`.analysis/`, `data/vanilla/`, runtime
  worlds, logs, tool scratch dirs). Preserve unrelated local changes.
- Review expects: L1/L2 green with harness receipts quoted, `doclinks.py` green
  for doc changes, the invariants above respected, and a regression test that
  fails without the fix. Docs-only and test-only changes still need doclinks
  green and a description of what a player observes differently.

## Licence

Dual-licensed under [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your
option. Krasnokamen is unofficial and not affiliated with Mojang Studios or
Microsoft.
