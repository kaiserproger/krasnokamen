# Adding a vanilla gameplay feature

This is a navigation path for a gameplay change, not proof of parity. Start
with [the architecture](../ARCHITECTURE.md), the relevant native owner and
[Definition of Done](../DEFINITION_OF_DONE.md). Confirm the intended behaviour
and vanilla facts from the appropriate local oracle before implementing it.
Do not infer packet IDs or field layouts from memory.

## Open transitions

[STATUS](../STATUS.md#open-transitions) lists every transition in flight whose
target the owner has decided: what replaces what, the evidence, and when the
old side is deleted. Before changing an area, look for its row. This covers
every area of the repository, the harness and tools included, not only
gameplay.

- Fixes and changes required on a currently supported surface are always
  allowed, including on the old side of a transition (26.1.2 support, for
  example, is a standing contract, not a transition).
- What the row stops is building new features or new mechanisms on the old
  side of a decided transition: a change in that area moves it forward, or
  at least adds no new caller, case or copy of the old mechanism.
- No new transition starts in an area that already has one open. Finish it,
  drop it, or get the owner's decision first.
- A change that opens a transition adds its row to STATUS in the same change.
- Finishing a transition deletes the old side in the same change (code, config,
  tests, docs) and deletes its row.

### Mechanisms, not code

The rules above are about *mechanisms*: parallel infrastructure or authority
for one responsibility (pinning, server launch, persistence paths, observers,
config). Unify those. Duplicated *code* at call sites is fine, and preferred
to the wrong abstraction ([Metz](https://sandimetz.com/blog/2016/1/20/the-wrong-abstraction)).
A parameter that carries a fact of the shared semantics (environment state, a
game rule, input data, as in `Vitals::tick(eye_in_water, can_drown, ...)`) is
fine. If a new caller needs a mode, flag or boolean that makes the shared
function do something different for that caller, or a caller-specific branch,
the abstraction is wrong: inline the shared code into its callers and delete
the part each caller does not use. The fastest way forward is back, and how
much the shared code cost or how complex it has grown is no reason to keep it.
Never do that to gameplay authority: a rule implemented in two places is
worse, so split the function by behaviour instead.

## New dependencies and components

A new external crate, toolchain or component (nightly Rust, `cargo-component`),
language or runtime (each layer already has one: Rust for the product, Python
stdlib for the harness and `tools/dev` with bash only for thin wrappers, Java
only for client mods and vanilla oracles), persistent format, daemon or
service, or new required
prerequisite binary spends one of very few innovation tokens
([McKinley](https://mcfunley.com/choose-boring-technology)). Before adding one,
write a short decision record (a new ADR or a section of the owning one) that
answers:

1. How would we solve this with what we already have?
2. What exactly makes the current stack prohibitively hard or expensive?
3. What existing thing does it replace, and what is the commitment and plan to
   migrate? A replacement is a row in
   [open transitions](../STATUS.md#open-transitions).

## Trace the actual path

For a client-triggered block placement, trace this example end to end:

1. `crates/mc-protocol/src/packets/` defines the wire packets.
   `crates/mc-edge/src/decode.rs` decodes `ServerboundUseItemOn` into
   `PlayerInput::UseItemOn`; the edge associates the authenticated connection.
   If the packet or its encoding changes, check protocol and edge decoding,
   including the supported-window chain in `crates/mc-edge/src/translation/`.
2. `crates/mc-core/src/boundary.rs` defines inputs and results. `tick.rs`
   dispatches into `tick/input/interact.rs` and its `use_item_on` handler.
   Inspect the handler's other branches before changing a shared action.
   Other inputs route through `tick/input/{session,movement,combat,inventory,social}.rs`.
3. Placement planning in `tick/build.rs` and `placement.rs` builds the native
   decision. `intent.rs` defines `Intent`, `IntentKind` and `Footprint`.
   Include every precondition, block, item debit and dependent effect in the
   appropriate atomic decision; check live reach, session and custody.
4. `tick/merge.rs` sorts and applies intents against accumulated state. A
   stale footprint must not partially consume resources or apply effects.
   Plugin build hooks and approval fences, where applicable, must remain intact.
5. The tick change set feeds native journal construction in `core/journal.rs`
   and `journal_touched.rs`. Durable changes require the appropriate absolute
   post-image or tombstone and checkpoint coverage; test restart when storage
   changes. [Persistence](../architecture/persistence.md) explains the ACK
   versus fsync distinction and the checkpoint-only motion exception.
6. `crates/mc-edge/src/plan.rs` and `plan/player_changes.rs` project native
   changes into client packets. Check correction as well as success: client
   prediction is not authority, and socket delivery is not durable commit.

This chain is an example, not a universal pipeline. Scheduled work need not
start with a client packet; some input handlers mutate tick-owned state directly,
and transient state need not create a WAL post-image. Trace the actual owner,
commit and recovery boundary instead of forcing each change into an `Intent`.
See [mutation routes](../architecture/mutations.md).

## Data and commands

`crates/mc-core/src/registry.rs` loads `CoreRegistry`; content and vanilla
registries are supplied by `mc-data` and the imported content cache. Inspect
the relevant existing table and loader before adding new data; don't embed
Mojang content in Git. If startup contributions are needed, inspect
`crates/mc-core/src/plugin_startup.rs` and existing native contributions,
for example `settlements/startup.rs`.

For a command, trace `crates/mc-core/src/commands.rs` and
`command_execution.rs` to the focused native executor. Update the client
command tree in `crates/mc-edge/src/command_tree.rs` and its subtrees and
suggestions where appropriate; keep advertised permissions and executable
paths consistent. Plugin-declared commands have their own admission contract
in [PLUGINS.md](../PLUGINS.md). An edge tree is presentation, not permission
to bypass native validation.

## Regression and acceptance

1. Write a focused regression against the existing real tick path and the
   intended behaviour. It must go red when only the fix hunks are reverted
   (negative control); record whether you also saw it fail first. A compile
   failure or a test that only asserts a helper's own output is not a
   behavioural reproduction.
2. Put substantial tests in a focused sibling `*_tests.rs`, following the
   owning module's existing test registration. For an input action, exercise
   the actual `PlayerInput` through the tick, and assert resulting state and
   outgoing corrections. Cover rejected/stale inputs and custody when relevant.
3. Add restart/recovery coverage for durable changes and protocol/decode tests
   when those boundaries change. State clearly which tests do not exercise a
   real client. Run the relevant focused tests and required cargo baseline via
   the [resource runner](heavy-commands-and-memory.md).
4. For player-visible changes, run the exact scenario with an actual client
   under the [real-client gate playbook](real-client-gates.md). A new or changed
   gate requires a negative control that fails when the feature is absent or
   broken. Prepared-only, TCP, unit and server-side observation are not a
   graphical client pass. A previously failed owner scenario remains blocked
   until the same or stricter real-client scenario passes.
5. Report the exact commands, evidence and gaps, including oracle, persistence
   and client scope. Label the result `draft`, `stabilization` or `release-ready`
   under the [DoD evidence matrix](../DEFINITION_OF_DONE.md); default to
   `draft` when evidence is missing. Never claim broader parity than the gate
   actually observed.
