# Adding a vanilla gameplay feature

This is a navigation path for a gameplay change, not proof of parity. Start
with [the architecture](../ARCHITECTURE.md), the relevant native owner and
[Definition of Done](../DEFINITION_OF_DONE.md). Confirm the intended behaviour
and vanilla facts from the appropriate local oracle before implementing it.
Do not infer packet IDs or field layouts from memory.

## Trace the actual path

For a client-triggered block placement, trace this example end to end:

1. `crates/mc-protocol/src/packets/` defines the wire packets.
   `crates/mc-edge/src/decode.rs` decodes `ServerboundUseItemOn` into
   `PlayerInput::UseItemOn`; the edge associates the authenticated connection.
   If the packet or its encoding changes, check protocol and edge decoding,
   including 26.1.2 translation in `crates/mc-edge/src/legacy/` when applicable.
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

1. Write a focused failing-first regression against the existing real tick
   path and the intended behaviour. A compile failure or a test that only
   asserts a helper's own output is not a behavioural reproduction.
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
