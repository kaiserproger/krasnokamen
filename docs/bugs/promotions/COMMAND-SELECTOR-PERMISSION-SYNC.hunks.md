# COMMAND-SELECTOR-PERMISSION-SYNC — hunk map

Row: `COMMAND-SELECTOR-PERMISSION-SYNC` (client-surface, minor) — the operator selector `@s` is
rejected because the core never sends the player their own permission status on join or on an operator
change. Refs: `cargo:mc-core:tick::auth_gate_tests::permission_projection_follows_join_updates_and_authenticated_unlock`,
`cargo:mc-edge:server::admin_tests::operator_changes_reach_own_entity_permission_consumer`,
`cargo:mc-edge:plan::plan_tests::permission_updates_target_joined_self_reliably_in_order` (structural, not
proving), `harness:scene-shot:command-selector-permission` (`op-permission`).

## Real fix

**The fix is not a commit of this history** (HANDOFF-11 records the same for the permission projection):
the behaviour arrived as a port of the edge/core permission path, and the three call sites are what the
client needs. The recorded identity is
`fix_commit = patch:a94bbc63e602a74055c29005435bb2b4c324de4ae09a9f8c6450415dca845ba9`,
the non-test hunks of `docs/bugs/fix-patches/COMMAND-SELECTOR-PERMISSION-SYNC.patch`.

| file | a-side (pre-image) | b-side (fixed) |
|---|---|---|
| `crates/mc-core/src/tick/input/session.rs` | no `sync_permission` call in `permission_level` or after `join applied` | `sync_permission(state, *connection)` at both sites |
| `crates/mc-core/src/tick/auth.rs` | no call in the join hook | `super::input::session::sync_permission(state, connection)` |

## Adaptation, stated

The patch removes exactly the three calls and keeps the helper
`pub(in crate::tick) fn sync_permission` (`session.rs:23`) in place. That is the README class "helper
whose every *call site* is inside the ported hunks": no line is invented on the a-side, no production
panic, and the helper alone changes nothing. The patch was re-authored from the current tree because the
stored copy no longer reverse-applied after the file grew; the hunk set is the same as the one recorded
before the re-authoring, only the offsets moved.

## Proving locations

- `crates/mc-core/src/tick/auth_gate_tests.rs:77` — `assert_eq!` on
  `PlayerChange::PermissionLevel` right after the joined event: the direct symptom, deterministic.
- `crates/mc-edge/src/admin_tests.rs:714` — `.expect(…)` on the 30-second `EntityEvent` join wait: the
  packet never arrives. It is an `expect`, not an assert macro, so the row names it in
  `proving_assertions`; it also catches any join stall, which is why the scene carries the symptom too.
- `harness:scene-shot:command-selector-permission` `op-permission` — the client's own observation:
  control `command.permission_level = null`, restore `4` (and `0` within 40 ticks of `deop @s`).

## Why `plan_tests::permission_updates_target_joined_self_reliably_in_order` is not proving

README:450 — a red outside the reversed hunks is not evidence about the fix, and this one is green on
both sides: the edge's *packet plan* is independent of the core choosing to emit the packet, so the
patch cannot make it fail (its control state is `green`, measured in the promotion).

## Gate

- head `05c8f435f`, server pins `05c8f435f6b9-server` (restore) and `05c8f435f6b9-8a20f244-server`
  (control, the same tree with the patch reversed), instrumented client pin `d3b5eedf470c` on **both**
  sides, so a red can only come from the server.
- control (`.analysis/validation/20261125T014648-ledger-promote-yuyyldfd`): `auth_gate` FAIL,
  `admin_tests` FAIL, `op-permission` red with `permission_level: null`; `plan_tests` green (structural).
- restore: all four green, `command.permission_level` = 4, then 0 after `deop @s`.
- what it does not prove: the suggestion list itself (the client has no command-suggestion machinery);
  the scene's `limits` says so.
