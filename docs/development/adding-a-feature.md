# Adding a vanilla gameplay feature

Start with [architecture](../ARCHITECTURE.md), the native owner and
[DoD](../DEFINITION_OF_DONE.md). Establish intended vanilla behavior from the
appropriate local oracle; never infer packet IDs/layouts from memory.
This route is not parity evidence; [historical wording](README.md) remains in Git.

## Open transitions

[STATUS](../STATUS.md#open-transitions) owns decided transitions: replacement,
evidence and deletion boundary. Check the touched area's row—including tools
and harness—before editing.

- Fixes and changes required on supported surfaces are always allowed, even on
  an old side; 26.1.2 support is a standing contract, not a transition.
- No new feature/mechanism on the old side: advance the transition or add no
  old caller/case/copy. Do not open another transition there until finishing,
  dropping or obtaining the owner's decision.
- Opening a transition adds its STATUS row in the same change; finishing
  deletes old code/config/tests/docs and the row together.

### Mechanisms, not code

Unify parallel **authority/infrastructure** (pinning, launch, storage, observers,
config), not all repeated call-site code. Repetition beats a
[wrong abstraction](https://sandimetz.com/blog/2016/1/20/the-wrong-abstraction).
Shared semantic facts are legitimate parameters (`Vitals::tick(eye_in_water,
can_drown, ...)`); caller-specific flags/modes/branches are not. Inline such
shared code into callers and remove unused parts, regardless of sunk cost.
**Never duplicate gameplay authority**: split that function by behavior instead.

## New features are modules

Owner 2026-11-16 (`receipts/Stage3/OWNER-RULE-MODULAR-NEW-FEATURES-20261116.md`):
new features are modules now; fixes stay with their owner. Existing-code
cycle-cut moves follow [ADR 0021](../decisions/0021-build-time-and-crate-split.md)
byte-exactly: before the 21 November announcement only after current release
tasks close, otherwise afterwards (owner 2026-11-17).

| Boundary | Requirement |
|---|---|
| Placement | Coherent module under domain host, e.g. `crates/mc-core/src/settlements/migration.rs`; directory + `mod.rs` when needed. Without core-internal dependencies, use a separate crate below core |
| Ownership | State, tick systems, sibling `*_tests.rs` and feature-row codec inside; persist via host's existing path |
| Interface | Few named functions/small trait; may use existing `TickState`, entities, settlement records; no ad-hoc `pub(crate)` reach-ins |
| Registration | One host-owned point (module declaration plus one tick call/state field), not scattered edits in `tick.rs`/`registry.rs`. This limits conflicts; host→feature→TickState→host is still a cycle |
| Cycle ratchet | `crates/xtask/src/code_health/ratchet.rs` counts top-level modules in the largest SCC, not individual edges or submodule cycles; rejects above ceiling. Most core top-level modules share one SCC. Use a domain submodule or top-level module that nothing in the SCC refers to. A new node joins only when paths exist in both directions; submodule references can still pull other top-level nodes in. Inferred/type-method-resolved references are invisible to the checker |

Review checklist: name module+host, internal state/systems/tests, exported
interface and single registration point; `python3 -m tools.harness run code-health`
largest-cycle count must not increase
([ratchet](../decisions/0021-build-time-and-crate-split.md#cycle-ratchet-implemented-2026-11-07)).

## New dependencies and components

Existing layers: Rust product; Python stdlib harness/dev tools (Bash thin
wrappers); Java only mods/oracles. An external crate, toolchain/component
(nightly, `cargo-component`), language/runtime, persistent format, daemon/service
or required executable spends one of very few [innovation tokens](https://mcfunley.com/choose-boring-technology).
Before adding one, an ADR/owning-ADR section must answer:
1. How can existing tools solve it?
2. What makes that prohibitively hard/expensive?
3. What does it replace, with what migration commitment?
   Replacements also need a [transition row](../STATUS.md#open-transitions).

## Trace the actual path

Example: client block placement. Unprefixed paths here are relative to
`crates/mc-core/src/`; paths starting `crates/` are repository-relative.

| Stage | Owner and required boundary |
|---|---|
| Wire | `crates/mc-protocol/src/packets/`; `crates/mc-edge/src/decode.rs`: `ServerboundUseItemOn` → `PlayerInput::UseItemOn`, authenticated connection association. Encoding changes cover both decoder and supported-window `crates/mc-edge/src/translation/` |
| Dispatch | `boundary.rs` inputs/results; `tick.rs` → `tick/input/interact.rs::use_item_on`. Inspect other branches of shared actions; other routes: `tick/input/{session,movement,combat,inventory,social}.rs` |
| Plan | `tick/build.rs`, `placement.rs`; `intent.rs` owns `Intent`, `IntentKind`, `Footprint`. Atomic decision includes preconditions, blocks, debits and dependent effects; validate live reach/session/custody |
| Merge | `tick/merge.rs` orders/applies against accumulated state; stale footprint cannot partially debit/apply. Preserve plugin build hooks and approval fences |
| Durability | `core/journal.rs`, `journal_touched.rs`: proper absolute post-image/tombstone and checkpoint coverage; durable changes need restart tests. [Persistence](../architecture/persistence.md) owns ACK versus fsync and checkpoint-only motion |
| Projection | `crates/mc-edge/src/plan.rs`, `crates/mc-edge/src/plan/player_changes.rs`: success **and correction**. Prediction is not authority; socket delivery is not durable commit |

This is not a universal Intent pipeline: scheduled work may have no packet,
handlers may directly mutate tick-owned state, transient state need not create
WAL images. Trace real owner/commit/recovery boundaries
([mutation routes](../architecture/mutations.md)).

## Data and commands

- `crates/mc-core/src/registry.rs` loads `CoreRegistry` from `mc-data`/imported cache.
  Inspect existing table/loader first; no Mojang content in Git. Native startup
  contributions: `crates/mc-core/src/plugin_startup.rs`, e.g.
  `crates/mc-core/src/settlements/startup.rs`.
- Trace `crates/mc-core/src/{commands,command_execution}.rs` to the focused executor;
  update `crates/mc-edge/src/command_tree.rs`, subtrees and suggestions as applicable.
  Advertised permissions and executable paths must agree. Plugin commands use
  [their admission contract](../PLUGINS.md); edge presentation never bypasses
  native validation.

## Regression and acceptance

1. Focused behavioral regression through real tick/`PlayerInput`, resulting state
   **and corrections**, with rejection/staleness/custody where relevant. Fix-only
   reversal must fail; record observed fail-first, not a gate. Compilation failure
   or a helper asserting itself is not behavioral reproduction.
2. Substantial tests go in sibling `*_tests.rs` via existing registration.
   Durable changes need restart/recovery; wire boundaries need decode/codec tests.
   Name tests' non-client scope; use [L0–L2](../../CONTRIBUTING.md#validation)
   under the [owner-host runner](heavy-commands-and-memory.md).
3. Player-visible changes need the exact [actual-client scenario](real-client-gates.md).
   New/changed gates need feature-absent/broken negative controls. Prepared-only,
   TCP, unit and server observation are not graphical acceptance; failed owner
   cases stay blocked until the same or stricter client scenario passes.
4. Report commands, evidence and remaining oracle/storage/client gaps. Use DoD
   `draft` (default if evidence missing), `stabilization`, `release-ready`;
   claim no more than observed.
