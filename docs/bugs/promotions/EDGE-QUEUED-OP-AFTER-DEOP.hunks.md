# EDGE-QUEUED-OP-AFTER-DEOP — control

[Control patch](../fix-patches/EDGE-QUEUED-OP-AFTER-DEOP.patch) for
`878f1535bbde9838775b755060fe6e8ca6f69278`.
`fix_commit = "patch:6ec10e90d8d8499a29c99abf09bca4fb653940b30418dcd6256084156cfd09f7"`
unchanged: the independent review approved these bytes and the Main dry-run
already observed the red and the restored green, so nothing here was re-derived or
rerun. The patch reverse-applies on `17b4abddc370d1681fa3dceea13607118fc4933b`
with `git apply -R --check` rc 0.

## The fix

`op`/`deop` first authorizes the command, then waits for authoritative input
capacity and for the operator fence. A deop of the initiating session that lands
during that wait was not re-checked, so the queued `op` executed with the level
read before the wait. `878f1535` re-authorizes the complete selected batch after
both awaited steps.

## Hunk map (original hunks → HEAD, patch hunks)

| original (`878f1535`) | HEAD equivalent | patch hunk (b-side) |
|---|---|---|
| `server.rs @@ -1096,9` — reserves the batch capacity, then re-checks `session_locked`/`command_tree::permitted` for the initiator | `crates/mc-edge/src/server.rs:1042-1059` (reservation `:1042-1046`, re-check `:1049-1059`) | `@@ -1046,6` (removes only `:1049-1059`) |

The reverse deletes only the post-wait re-check. The pre-wait check
(`server.rs:938`) and the mutation (`set_op`, `server.rs:1061`) stay, so the
control reproduces exactly the approved scenario: a queued `op` whose initiator is
deopped while it waits still executes.

## a/-side adaptation

None. The patch removes the re-check block and adds nothing on the a-side.

## Kept out of the control

- `admin.rs @@ -245` (`sessions_for_uuid`), `server.rs @@ -1096` (all-session
  reservation and fanout) and `server.rs @@ -1107` (per-session
  `PermissionLevel` and command tree) — (a): that is
  `EDGE-REJOIN-OP-SESSION` (#83); reversing it here would turn this row's red on
  #83's symptom.
- `boundary.rs`, `player.rs`, `plan/player_changes.rs`,
  `packets/play.rs` and `packets/play/inventory.rs` — (a): the hidden-pickup
  targeting change carried by the same commit, owned by
  `CORE-PICKUP-HIDDEN-MENU-IMAGE`.
- `admin_tests.rs`/`plan_tests.rs`/`translation/adjacent/play_tests.rs` and the
  ledger/decision docs — test and documentation hunks, not a production patch.

## Proving assertion

- `cargo:mc-edge:server::admin_tests::queued_op_is_rejected_after_initiator_is_deopped`
  — `crates/mc-edge/src/admin_tests.rs:188`, assertion at `:207-211`
  ("demoted operator must not regain rights"). A full permit hold parks both
  commands; after release the deopped initiator's `op` must not restore level 4.
  Observed red in `.analysis/validation/20261123T074112-ledger-promote-o087__2e`
  (`ledger-1-run.log:247-248`) and green after restore in `ledger-3-run.log:238-242`
  (Main dry-run receipt `MAIN-EDGE-QUEUED-DRY-RUN.md`); not rerun here.
- Limit: that observed red/restore belongs to the previous candidate fingerprint
  (`admin_tests.rs` has since changed); it is not a run on the current tree. The
  bytes and ref are unchanged, so a fresh red/restore is a rerun of the same
  control, not new evidence requirements.
- Scope: proves the registered capacity-wait scenario, not every selector
  resolution path.
