# EDGE-REJOIN-OP-SESSION — control

[Control patch](../fix-patches/EDGE-REJOIN-OP-SESSION.patch) for
`878f1535bbde9838775b755060fe6e8ca6f69278`.
`fix_commit = "patch:65640ea957d9408ec82970c28e93c8812fc392cc1420dd3287579ac04549ecf7"`
is the SHA-256 of the patch's non-test hunks. The patch reverse-applies on
`17b4abddc370d1681fa3dceea13607118fc4933b` with `git apply -R --check` rc 0.

## The fix

Before `878f1535`, an `op`/`deop` reserved two inputs, mutated the operator list,
and then updated the single session that `Admin::find(target)` (name match with a
UUID filter) returned after the mutation: one `PermissionLevel` and one command
tree. A player registered under more than one connection (rejoin before the old
connection leaves, a second registration for the same UUID) kept the stale native
level on every session the name lookup did not return. The fix enumerates
`sessions_for_uuid`, reserves capacity for all of them, and sends the native
`PermissionLevel` and a command tree to each, zeroing the tree for a locked
session.

## Hunk map (original hunks → HEAD, patch hunks)

| original (`878f1535`) | HEAD equivalent | patch hunk (b-side) |
|---|---|---|
| `admin.rs @@ -245,6` — adds `Admin::sessions_for_uuid` (all sessions for the UUID, sorted by connection id) | `crates/mc-edge/src/admin.rs:248-256` — kept in the patch (see (b)) | — |
| `server.rs @@ -1096,9` — replaces `reserve_inputs(2)` and the single-session mutation with the per-UUID session list, `reserve_inputs(1 + sessions.len())` and the post-wait re-check | `crates/mc-edge/src/server.rs:1031-1046` (selection `:1031-1041`, capacity `:1038-1046`) | `@@ -1028,30` |
| `server.rs @@ -1107,23` — replaces the single `find(target)`-selected `PermissionLevel` and tree with one `PermissionLevel` and one tree per session, the tree zeroed when the session is locked | `server.rs:1070-1089` (per-session `PermissionLevel` `:1070-1078`, per-session tree `:1079-1089`) | `@@ -1080,42` |

## Control shape and its boundary

The control splits the current block at the selection kind (`target.starts_with('@')`):

- **Name path (no `@`)** — historical: `reserve_inputs(2)` *before* the mutation
  (one `SpawnProtectionOperators`, one `PermissionLevel`, with the unused permit
  dropped when the lookup finds nothing, exactly as the original); `set_op`; then
  `Admin::find(target).filter(|entry| entry.1 == uuid)` *after* the mutation; one
  `PermissionLevel` with `admin.level(uuid)` and one command tree with the raw
  `admin.level(uuid)` (no locked-session zero).
- **Selector path (`@a`, `@s`, …)** — left as the current code: per-UUID
  `sessions_for_uuid` fanout, `reserve_inputs(1 + sessions.len())`, locked-session
  zeroed tree. This path postdates `878f1535` and is another owner's behaviour
  (category (a)); reverting it would turn this row's control red on a fanout the
  row does not describe.

The earlier map claimed the batch's `1 + selected.len()` was equivalent to the
old `reserve_inputs(2)`. That was wrong and is corrected here: with exactly one
free permit the two forms admit differently, and the old lookup happened after
the mutation, which is what the control now reproduces. The boundary between the
historical name path and the later selector path is the `selector` branch itself.

## a/-side adaptation

- `Admin::sessions_for_uuid` stays (category (b)): the fixed fanout, the selector
  path and `admin_tests.rs:312` use it; the control calls it only in the selector
  branch.
- The a-side is **not** empty. It introduces, on top of the pre-fix statements
  (`reserve_inputs(2)` before the mutation, `Admin::find(target).filter(|entry|
  entry.1 == uuid)` after it, one `PermissionLevel` with `admin.level(uuid)` and
  one raw-level command tree):
  - `let selector = target.starts_with('@');` and the four-line comment above it
    ("Historical name path: two permits reserved up front, … the later selector
    path keeps its per-UUID fanout") — the split that keeps the post-`878f1535`
    selector fanout (reason (a)) while the name path is historical;
  - three `if selector` branches: the session list
    (`if selector { sessions_for_uuid(uuid) } else { Vec::new() }`), the permit
    count (`if selector { 1 + sessions.len() … } else { 2 }`) and the update block
    (`if selector { … per-UUID fanout … } else if let Some(…) = self.admin.find(…)`).
  These are the selector/name boundary, not behaviour of this row: the selector
  path is another owner's fanout and stays HEAD's, and the name path is the
  pre-fix code. The control's red is read on the name path only.

## Kept out of the control

- `server.rs:1049-1059` post-wait re-authorization (the `EDGE-QUEUED-OP-AFTER-DEOP`
  hunk) — (a): another row's behaviour.
- `boundary.rs`, `player.rs`, `plan/player_changes.rs`, `packets/play.rs`,
  `packets/play/inventory.rs` — (a): hidden-pickup targeting owned by
  `CORE-PICKUP-HIDDEN-MENU-IMAGE`.
- `admin_tests.rs`/`plan_tests.rs`/`translation/adjacent/play_tests.rs` and the
  ledger/decision docs — test and documentation hunks.

## Proving assertion

- `cargo:mc-edge:server::admin_tests::deop_reaches_every_registered_session_for_one_uuid`
  (`crates/mc-edge/src/admin_tests.rs:328`). One connection finishes admission for
  the UUID (native level 4); a second registration for the same UUID is held under
  the distinct name `Alex2` before its own admission; the command names `Alex2`, so
  the name is unique and the historical `find` pick is deterministic — no
  HashMap-order dependence. Fixed: the UUID fanout reaches the admitted session and
  the core publishes native level **0** (value assertion through
  `assert_published_levels`, helper `:456`, final assert `:486-491`, call `:368`).
  Inverse: on the name path the lookup returns only the unadmitted session whose
  `PermissionLevel` input is a no-op (no entity), so the admitted session's last
  published level stays **4** — red on the value, not on a timeout.
- The original same-name scenario `deop_reaches_rejoining_native_session`
  (`admin_tests.rs:291`) is preserved unchanged (two sessions both named `Alex`).
  It is green on the fixed candidate; under the inverse the historical name lookup
  returns an arbitrary one of the two same-name sessions, so its outcome is
  unordered and is **not** evidence for this control. Its native proof (chat-spam
  disconnect) is not used as the red.
- Edge levels are never evidence: `set_op` updates every session of the UUID
  (`crates/mc-edge/src/admin.rs:418-423`), so `server.admin.session_level(..) == 0`
  holds in both forms.
