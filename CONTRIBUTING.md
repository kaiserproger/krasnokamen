# Contributing to Krasnokamen

Krasnokamen is a Minecraft: Java Edition server written from scratch in Rust:
parallel ticks, deterministic outcomes, WAL-backed saves and sandboxed Wasm
plugins. Players join with an unmodified client (releases 26.1.2–26.3, canonical
26.3). Read the [project README](README.md), then the
[documentation index](docs/README.md) for the page that owns your area.

Welcome: bug reports with a reproduction, focused fixes, documentation and
[plugins](docs/PLUGINS.md) (see the `krasnokamen-default-plugins` repository).
Discuss material scope or design changes in an issue first. Core production
builds must not require sibling repositories.

## Prerequisites

| Need | Used for |
|---|---|
| rustup | installs the toolchain pinned in [`rust-toolchain.toml`](rust-toolchain.toml) (1.99 with rustfmt and Clippy) on first use |
| Clang and mold | Linux x86-64 only: [`.cargo/config.toml`](.cargo/config.toml) selects them as linker; other targets use the default linker |
| JDK 25 (`java`, `javac`) | `content import` and the Java-backed harness profiles; plain builds and Rust tests do not need it |
| Python 3 | the validation harness, `python3 -m tools.harness` |
| cargo-nextest | the test runner behind `run test` and `correctness`: `cargo install cargo-nextest --locked` |
| Xvfb, client jars | graphical real-client profiles only; see [harness prerequisites](docs/AGENT_TOOLING.md#prerequisites) |

## Build

From the repository root on Linux, macOS or Windows:

```sh
cargo build -p mc-edge --bin krasnokamen
```

Debug is the default; build `--release` only when a gate or a release needs it.

## Content import (once per checkout)

Krasnokamen ships no Mojang files. Tests read the derived game-data cache at
these **checkout paths, never through a symlink**: `data/vanilla/` (canonical
26.3) plus `data/vanilla-26.1.2`, `data/vanilla-26.2` and `data/vanilla-steps`.
One import installs all of them:

```sh
# downloads the official server bundles (needs JDK 25 and network access to Mojang)
cargo run -p mc-edge --bin krasnokamen -- content import --version 26.3 --download --cache data/vanilla
# or from server jars you own; --from and --download cannot be combined
cargo run -p mc-edge --bin krasnokamen -- content import --cache data/vanilla \
  --from /path/to/26.3/server.jar --gameplay-from /path/to/26.1.2/server.jar
```

Without `--cache` the import goes to the operator default
`$XDG_DATA_HOME/krasnokamen/content/26.3` (else
`~/.local/share/krasnokamen/content/26.3`), which tests never read. The server
prefers a complete per-user cache over the checkout; the run command below
selects the checkout explicitly, and a configured `[data].vanilla_data_dir`
overrides both ([cache selection and `content verify`](docs/OPERATING.md#vanilla-content-cache)).

**Mojang bytes never enter Git**: `data/vanilla*` and `.analysis/` are ignored;
never stage them or any decompiled or extracted content. Packet IDs and layouts
come from local `wire-probe`/`javap` runs against your own jars, never from
memory ([protocol changes](docs/development/protocol-changes.md)).

## Run

```sh
KRASNOKAMEN_CONTENT_CACHE="$PWD/data/vanilla" cargo run -p mc-edge --bin krasnokamen -- --config example.toml
```

Connect a Java 26.3 client to `127.0.0.1:25565`. `example.toml` listens on all
interfaces with **offline authentication**: beyond local testing set
`[auth].online_mode = true` and review `[network].bind_address`
([operating guide](docs/OPERATING.md)).

## Validation

Run the harness from the repository root; `python3 -m tools.harness list` names
every profile and what it needs.

| Level | Command | When |
|---|---|---|
| L0 | `python3 -m tools.harness run test -p <crate> --lib <filter>` for the touched scope, plus diff/syntax checks of what you changed | while editing |
| L1 | L0 + `python3 -m tools.harness run fmt` + `python3 -m tools.harness run code-health` | finishing a change |
| L2 | `python3 -m tools.harness run correctness`: fmt, code-health, strict workspace Clippy, every nextest workspace/all-target test, then the registered bug regressions | before a code PR |

- L2 needs cargo-nextest, about 12 GiB of RAM and a
  [Krasnokamen Loader](https://github.com/kaiserproger/krasnokamen-loader)
  checkout for its real-client manifest tests: clone it beside this one as
  `../solaris-loader` or set `KRASNOKAMEN_LOADER_ROOT`. A warm 16-thread machine
  needs roughly 45 minutes; a cold target adds the full build. If you cannot run
  it, open a draft PR with your L1 receipts and say so: PR CI runs fmt, Clippy,
  code-health, the bug regressions and a sharded short test suite, and the
  maintainer runs L2 on the merge candidate.
- Every run writes `.analysis/validation/<UTC>-<profile>-<unique>/result.json`
  (`status`, `commands`, `exits`, `logs`, `details`, `error`). Quote that path
  when you call a check green. `prepared` or `--check` output is not a pass;
  unknown, unlaunchable or failing commands fail closed.
- Documentation-only changes need `python3 tools/doclinks.py` (links, anchors,
  reachability from `AGENTS.md`), not Cargo.
- `run regressions` alone is only for an integration that skipped L2.
- Graphical, oracle and performance profiles prove only their named scope and
  need more (Xvfb, JDK 25, client jars, a release server as
  `KRASNOKAMEN_SERVER_BINARY`): read [agent tooling](docs/AGENT_TOOLING.md) and
  [real-client gates](docs/development/real-client-gates.md) first. Never widen
  a bound, add retries or substitute protocol bots to turn a failing
  real-client gate green.

## Making a change

1. Read the owning page from the [documentation index](docs/README.md), the
   [decisions](DECISIONS.md) for the area and its
   [open transitions](docs/STATUS.md#open-transitions). A change that goes
   against a decision changes that row in the same commit.
2. **Every fix comes with a test that fails without it.** Run it once against
   the old code to see it fail. A bug fix's regression goes red when only the
   fix hunks are reverted (the negative control `ledger promote` records). A
   unit test cannot close a known client-visible failure: that needs a
   real-client check ([evidence rules](DECISIONS.md#evidence)).
3. **Fix the implementation, never the oracle.** A mismatch with vanilla is a
   Krasnokamen bug: do not edit the recorded vanilla result, the scenario's
   expectation or the fixture to make it pass. **Never widen a tolerance,
   bound, timeout or retry count** to make a mismatch go away; a difference
   beyond the bound is a bug.
4. **Intended differences from vanilla are named per case with a reason**
   (`known_divergence` in the scenario, [decision P7](DECISIONS.md#product)).
   The marker fails the gate once Krasnokamen matches; remove it then. An
   unnamed difference is a bug.
5. **Three changes without a gained case: stop and ask.** When three edits in
   a row leave the failing case as it was, stop editing, write down the
   remaining mismatch and the premise you worked from, and ask the maintainer
   (Main on the owner host): they may know the rule you are missing.
6. A green run with no test of your change proves nothing: a behaviour change
   adds its own test or scenario where the [table below](#where-tests-go) puts
   it.
7. To silence a lint, write `#[expect(lint, reason = "…")]`, not `#[allow]`,
   so it is flagged once it no longer applies; convert an old `#[allow]` when
   you touch its code.
8. Comments say only what the code cannot (an invariant, a vanilla reference,
   why the obvious way is wrong); the reasoning for a change goes in the
   commit message.
9. Every published number is measured on a quiet machine, with the command,
   the hardware and core count, and the slow cases beside the mean
   ([performance windows](docs/development/performance-windows-and-profiling.md)).
10. Check a command's own exit status (`set -o pipefail`, `${PIPESTATUS[0]}`,
    the harness `result.json`), and break the fix once to see the check fail
    before you trust it green.
11. Record anything broken you observed as `docs/bugs/rows/<ID>.toml`
    ([filing a bug](docs/bugs/README.md#filing-a-bug));
    `python3 -m tools.harness ledger show <ID>` reads one row and
    `ledger list --status open` lists the open ones. A fix adds `regression`
    references. Leave `fix_commit` out of a PR (the maintainer records it once
    the merged commit id is final) and **never set `status = "fixed"` by
    hand**: only `ledger promote` does, after running the negative control.

Mistakes already made, with the check for each, are in
[pitfalls](docs/PITFALLS.md).

## Where tests go

| Behaviour | Where | Run |
|---|---|---|
| Vanilla mechanics, redstone quirks, farm rates | a scenario in `tools/harness/quirk_parity/scenarios/`, run on a real vanilla server and on Krasnokamen ([quirk parity](docs/QUIRK_PARITY.md)) | `python3 -m tools.harness run quirk-parity -- --family <family>` |
| Movement and physics | local vanilla captures ([physics oracles](docs/physics-oracles.md)) | as that page says |
| What a player sees or does in a client | a harness real-client scenario ([native scenario rules](docs/development/real-client-gates.md#native-scenario-rules)) | `python3 -m tools.harness run <profile>` |
| A bug fix | the test or scenario the row's `regression` names ([ledger](docs/bugs/README.md#filing-a-bug)) | `python3 -m tools.harness ledger show <ID>` |
| Logic inside one module | a focused sibling `<module>_tests.rs` beside the module | `python3 -m tools.harness run test -p <crate> --lib <filter>` |
| Saves, WAL and recovery | `crates/mc-core` tests through checkpoint, action and restart with replay ([persistence](docs/architecture/persistence.md)) | same |
| Packets and codecs | `crates/mc-protocol/src/**/*_tests.rs` from wire-probe bytes; version steps in `crates/mc-edge/src/translation/adjacent/*_tests.rs` | same |
| World import and export | `crates/mc-world-conversion/src/*_tests.rs`, on worlds vanilla wrote | same |
| Content data (recipes, loot) | `crates/mc-data/tests/` | same |
| Plugin host and package contract | `crates/mc-plugin-host/tests/` | same |
| The harness itself | `tools/harness/test_*.py`, for a failure the harness really had | `python3 -m unittest tools.harness.test_<name>` |
| Performance | a profile, not a test ([performance profiles](docs/performance/PROFILES.md)) | as that page says |
| Native client, Loader, plugins | their own repositories | their READMEs |

## Pull requests

- Fork, work on a branch, open a PR. The owner reviews and merges; pushing,
  tagging and publishing are owner actions.
- One change per commit and per PR: two fixes are two revertible
  **Conventional Commits** (`fix(scope): …`, `docs: …`). Say what was wrong,
  how to reproduce it, how the change fixes it, and **what you could not
  test** (another OS, a client, real hardware).
- Leave Git hooks and signing alone; change `Cargo.lock` only for a concrete
  dependency reason. Never stage ignored local state (`.analysis/`,
  `data/vanilla*`, worlds, logs, tool scratch directories); preserve
  unrelated local changes.
- Review expects quoted L1/L2 receipts (or the draft note above), doclinks
  green for documentation and a regression that fails without the fix.
  Docs- and test-only PRs still say what a player observes differently.

## Coding agents on a fork

[AGENTS.md](AGENTS.md), with the [owner rules](docs/RULES.md) and
[pitfalls](docs/PITFALLS.md) it includes, applies to agents too. Rules about
the owner host (the `mem-run.sh` runner, `pin-release`, `wt-new`, `<work>`
paths) and Main's integration, queue and publication duties do not: run
commands directly, work on a branch in your fork and leave upstream `main`,
tags and releases to the maintainer. Missing harness inputs are not passing
evidence; only `ledger promote` closes a bug row.

## Licence

Dual-licensed under [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your
option. Krasnokamen is unofficial and not affiliated with Mojang Studios or
Microsoft.
