# COMMAND-FILL-SUCCESS-FEEDBACK — hunk map

Row: `COMMAND-FILL-SUCCESS-FEEDBACK` (client, minor) — "Successful fill replies with generic Changed
feedback instead of vanilla fill success." Ref: `harness:scene-shot:native-command-feedback`
(assertion `fill-success`).

## Origin commit and the row's hunk

The real origin is `3d4893773c482713c36c5564f66106227270249c` ("fix(compat): sync operator permissions
and preserve open copper chests", 2026-11-15): its `crates/mc-core/src/command_world.rs`
`@@ -432,7 +432,16 @@ pub(crate) fn execute(` hunk routes a successful `/fill` through
`crate::command_execution::success_feedback(state, connection, "commands.fill.success", …)` instead of
pushing the raw "Changed N block(s)" text. The fixed text itself comes from the client's vanilla
`commands.fill.success` translation ("Successfully filled %s block(s)").

Patch: `docs/bugs/promotions/COMMAND-FILL-SUCCESS-FEEDBACK.patch` — a git-format diff of exactly that
hunk (`crates/mc-core/src/command_world.rs`), regenerated from `3d4893773`, and it reverse-applies at
HEAD. `fix_commit = patch:67b9f371…` is the split patch's own hash. The a/-side line
`if result == Failed || crate::command_execution::accepts_success(state, connection) {` is the verbatim
pre-fix line (also present in the pre-fix `wt/BattleLiveCrash` tree at `command_world.rs:435`), so this
is a real pre-image, not a hand re-break.

## Every other hunk of 3d4893773 and why it is not in this patch

| file | hunk | what it does | reason |
|---|---|---|---|
| `crates/mc-core/src/boundary.rs` | `@@ -1022,6 +1022,10` | adds the `PlayerChange` variant that carries the own-entity permission | (a) COMMAND-SELECTOR-PERMISSION-SYNC's path |
| `crates/mc-core/src/tick/input/session.rs` | `@@ -16,9 +16,22`, `@@ -330,6 +343,7` | `sync_permission` on permission changes and on join | (a) COMMAND-SELECTOR-PERMISSION-SYNC |
| `crates/mc-core/src/tick/auth.rs` | `@@ -214,6 +214,7` | `sync_permission` on authenticated unlock | (a) COMMAND-SELECTOR-PERMISSION-SYNC |
| `crates/mc-edge/src/plan/player_changes.rs` | `@@ -193,6 +193,15` | plans the permission change to the viewer | (a) COMMAND-SELECTOR-PERMISSION-SYNC |
| `crates/mc-core/src/copper.rs` | `@@ -127,6 +127,21` | keeps an open copper chest from weathering | (a) the COPPER row (`COPPER-CHEST-OPEN-WEATHERING`) |
| `crates/mc-core/src/auth_gate_tests.rs` | `@@ -46,6 +46,64` | permission-projection tests | test hunk (excluded by `non_test_patch`), COMMAND-SELECTOR |
| `crates/mc-core/src/copper_tests.rs` | `@@ -400,6 +400,127` | copper chest tests | test hunk, COPPER |
| `crates/mc-edge/src/admin_tests.rs` | `@@ -515,6 +515,79` | operator-change tests | test hunk, COMMAND-SELECTOR |
| `crates/mc-edge/src/plan_tests.rs` | `@@ -114,6 +114,46` | permission-plan tests | test hunk, COMMAND-SELECTOR |
| `docs/COMPATIBILITY.md`, `docs/development/protocol-changes.md`, `docs/bugs/ledger.toml`, `docs/bugs/fix-patches/*.patch` | documentation, ledger and the per-row patch files | (c) no behaviour |

No hunk is (b): the fill hunk does not need API kept by the tests, and the fill tests are unaffected.

## Gate

`scene-shot`, scenario `native-command-feedback`: after the gamerule section the scene sends
`fill ~2 ~2 ~2 ~3 ~2 ~3 minecraft:stone` (four blocks), waits for the chat reply by event
(`wait_chat: "block(s)"`), then asserts `fill-success_contains_0` — the reply must read
`Successfully filled 4 block(s)` and must not read `Changed 4 block(s)`. On the control the reply keeps
the raw "Changed 4 block(s)" and `fill-success_contains_0` is the red
(`20261124T131618-ledger-promote-wggcurgg`, `regressions-client-0` exit 1).
