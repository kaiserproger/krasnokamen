# ISSUE-1-14 — hunk map

Row: `ISSUE-1-14` (client, major) — "Inventory crafting drag makes item disappear." Refs:
`harness:entities:native-v2-craft-drag` (`initial_drag_keeps_server_cursor`),
`harness:entities:native-v2-craft-drag-paced` (`native_drag_matches_vanilla_cursor`),
`harness:entities:native-v2-chest-drag-disconnect` (`chest_rejoin_item_exactly_once`).

## Real fix and its pre-image

Client `b7c48e1c7de46c34bc6611d438edfec3f37039c5` ("fix(client): keep the cursor across menu drags; goat,
spider and sheep model parity", 2026-11-08) predates the client rename `f0a6abf6`, so the route is the
committed-reversal pair. Its production hunks (7 in `src/ui/state.rs`) no longer reverse-apply at the
tip (later commits rewrote the pointer handler), so the a/-side was **hand-ported verbatim** to HEAD
(README: the same deletion hand-ported with the original lines named by `git blame -C -C`):

- reversal `5f01c22101016a14c5ea8eaf141ab2278ec21927` — re-adds `drag_pickup` (field, `new`,
  `release_all`, pointer-down reset, pointer-up reset), the pickup prediction block inside the
  pointer-down slot arm, the `|| self.drag_pickup` release branch and the `drag_origin` destination
  filter. `cargo check -p krasnokamen-client --lib` passes (`negctl/craft-reversal-check.log`).
- re-land `f843981bd320edfac6af221640d82481165abde0` — removes them again; the row's `fix_commit`.
- `git diff 5f01c22101 f843981bd3` is exactly the re-land's own non-test patch (3 insertions, 32
  deletions), which the ledger re-checks.

The hand-ported a/-side lines are the commit's own (shown by the `-` side of `git show b7c48e1c --
src/ui/state.rs`), re-fitted into the current handler: field/init/reset lines are literal; the
prediction block is the commit's block with the current call shapes (`inventory.menu().slots`,
`inventory.menu().kind.virtual_result`, which both still exist); the release branch and destination
filter are literal. No line was invented.

## Every other non-test hunk of b7c48e1c and why it is not in this pair

| file | hunks | what they do | reason |
|---|---|---|---|
| `assets/solaris/models/entity/goat.json`, `spider.json`, `src/minecraft/entities.rs`, `src/entities/solaris_models/tests.rs`, `tools/entity_models.py`, `tools/test_entity_models.py`, `docs/ENTITY_MODELS.md` | model parity | goat/spider rigs | (b) `ISSUE-1-18` (sheep/goat models) and the model rows; not the drag cursor |
| `src/session.rs` | `@@ -297`, `@@ -1304` | `game_time()` helper and an entity-listing test | (b) another feature of the same bulk commit |
| `src/session/observation.rs` | `@@ -54`, `@@ -76`, `@@ -1213`, `@@ -1231` | loading-readiness and singleplayer observation + tests | (b) other rows in the same bulk commit |
| `src/automation/minecraft_catalog.json` | two one-line edits | automation catalog labels | (c) tooling |
| `src/ui/drag_tests.rs` | six hunks | the drag unit tests updated to the new (server-cursor) behaviour | test hunks; kept unchanged on both sides |
| `docs/AGENT_TESTING.md`, `README.md` | documentation | (c) |

## Gate

Measured 2026-11-24 on the committed pair (server pin `pins/70d1d0679b56`, core `70d1d0679b`):

- `native-v2-craft-drag` (`initial_drag_keeps_server_cursor`) and `native-v2-craft-drag-paced`
  (`native_drag_matches_vanilla_cursor`) PASS on the control: their server-echo probes read the
  authoritative cursor, which the local prediction does not override for a release over the drag's
  origin.
- `native-v2-chest-drag-disconnect` FAILS on the control at `chest_drag_cursor_retained` (control
  `20261124T133513`, restore `20261124T133604` passed): the pre-fix prediction drops the retained
  cursor across the chest drag, which is the row's symptom ("the item disappears"). This is the ref
  the promotion uses; `chest_drag_cursor_retained` is the row's cursor-retention path, and it is the
  first failure in that scene on the control.
