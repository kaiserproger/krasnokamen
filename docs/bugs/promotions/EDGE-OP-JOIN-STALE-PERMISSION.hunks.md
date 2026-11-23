# EDGE-OP-JOIN-STALE-PERMISSION — control

[Control patch](../fix-patches/EDGE-OP-JOIN-STALE-PERMISSION.patch) for
`0504a2800c3b43fc06a8959f8bd809b5be116478`.
`fix_commit = "patch:bf716a13d456ed6bbe9a34b4cf8956a57835d384aa4d9c1524c1f2ee34cae489"`
is the SHA-256 of the patch's non-test hunks. The patch reverse-applies on
`17b4abddc370d1681fa3dceea13607118fc4933b` with `git apply -R --check` rc 0.

## The fix

Before `0504a280`, `run_connection` read the level from the operator registry
(`admin.level(outcome.uuid)`), submitted the native Join with it, and registered
the session with `admin` only after the join burst — outside any operator fence.
An op/deop landing in that window changed the registry but not the already
submitted Join, so the admitted native session kept the stale permission. The fix
moved the level read, the registration and the Join under `operator_updates`
(`admit_native_session`), read the level from the fresh session
(`admin.session_level(id)`), captured the burst permission before the burst
(`burst_op_level`), fenced Play registration with the same lock and refreshed the
command tree after registration.

## Hunk map (original hunks → HEAD, patch hunks)

| original (`0504a280`) | HEAD equivalent | patch hunk (b-side) |
|---|---|---|
| `connection.rs @@ -3,6` — adds `admit_native_session` (fence, register, Join with `session_level`) and `refresh_join_permission_tree` | `crates/mc-edge/src/server/connection.rs:56-76` (admission body) and `:78-95` (refresh body) | `@@ -61,15` and `@@ -78,7` |
| `connection.rs @@ -222,18` — replaces the direct `hub.join_session(..., admin.level(outcome.uuid), ...)` with the `ip` capture and `admit_native_session` | `connection.rs:387-401` | `@@ -372,24` |
| `connection.rs @@ -273,6` — captures `burst_op_level` from `session_level` before the burst | `connection.rs:450-455` | `@@ -439,6` |
| `connection.rs @@ -284,11` — join burst uses `burst_op_level` instead of the inline `admin.level(outcome.uuid)` | `connection.rs:473` | `@@ -457,11` |
| `connection.rs @@ -307,6` — takes `operator_updates` around Play registration | `connection.rs:492` | `@@ -480,6` |
| `connection.rs @@ -333,11` — removes the late unfenced `admin.register` and refreshes the command tree under the fence | `connection.rs:517-519` | `@@ -504,11` |

Reverse of each patch hunk restores the pre-fix text: the admission body reads
`admin.level(uuid)` and enqueues the Join before registering (no fence, no
`remove` because nothing is registered on failure); the caller submits the Join
directly with that registry level and registers `admin` after the burst; the
burst level is read inline from `admin.level(outcome.uuid)`; Play registration is
unfenced; `refresh_join_permission_tree` is a stub.

## a/-side adaptation and test instrumentation

- `admit_native_session` and `refresh_join_permission_tree` keep their HEAD
  signatures: both are called from `admin_tests.rs` (category (b)). Only their
  bodies are the control.
- **The `refresh_join_permission_tree` stub is test-API adaptation, not a
  historical pre-image**: the function did not exist before `0504a280`. Keeping
  the name with `let _ = (server, id, burst_level);` lets the kept
  `join_burst_permission_change_refreshes_command_tree` compile and go red.
- **Rendezvous instrumentation** (`connection.rs:22-52`,
  `#[cfg(test)] pub(super) static ADMIT_RENDEZVOUS` and
  `admit_rendezvous(uuid)`; call line `:74`). A test arms it for one UUID; the
  admission reports arrival and waits for a oneshot release. It is identical on
  both sides of the patch (common context, not a hunk), carries no behaviour, is
  compiled out of release builds and is a no-op for every unarmed test. It exists
  only so a real command can run at the boundary the control already has.
- The fixed body drops the operator fence explicitly before parking
  (`connection.rs:72 drop(_guard)`): after the drop the only remaining statement
  is `session`, so this is behaviour-identical to dropping at return, and it lets
  the test's real `deop` take the fence without deadlock. The control body never
  takes the fence.
- The caller passes `mc_core::boundary::ClientSettings { view_distance, locale,
  chat_visibility }` where the 2026-11 pre-image passed positional
  `view_distance, locale`: `ClientSettings` is the later `join_session` API. The
  values are the ones HEAD's fixed path already sends.

## Kept out of the control

- `crates/mc-core/src/command_debug.rs @@ -63`, `command_execution.rs @@ -92/-389`
  (formatting) and `tick/entity_phase.rs @@ -1309` (a reader cleanup) — (a): not
  this row's behaviour.
- `*_tests.rs` hunks of `0504a280` — test hunks, not part of a production patch.
- The all-session operator fanout of `878f1535` belongs to
  `EDGE-REJOIN-OP-SESSION` (#83).

## Proving assertion

- `cargo:mc-edge:server::admin_tests::deop_between_join_submission_and_late_registration_keeps_native_level`
  (`crates/mc-edge/src/admin_tests.rs:533`). It arms the rendezvous for the unique
  subject `GateSubject` (so no concurrent test admits the same UUID), polls the
  admission exactly once, requires it to park, awaits the arrival oneshot, runs a
  **real** `deop GateSubject` command (resolved through the ops entry, so it works
  before registration), releases, and asserts the published native
  `PlayerChange::PermissionLevel` is 0 (helper `assert_published_levels` at
  `:456`, final assert `:488-492`, call `:582`).
  - Inverse: the admission has already snapshotted level 4 and submitted the
    Join, and has not registered the session, so the command's name lookup finds
    nothing and sends no native `PermissionLevel`. The Join's **4** is the only
    published level → red on the value ("wants 0, published Some(4)").
  - Restored: the admission registered before parking and released the fence; the
    command's fanout reaches the session and sends 0, so the last published level
    is **0** → green.
  - Synchronisation is oneshot-only: no sleeps, no polling, no timing races.
- Secondary refs: `joining_session_and_deop_keep_native_permission_order` reds in
  the inverse on the fence setup assertion `admin_tests.rs:411`; the stubbed
  refresh reds `join_burst_permission_change_refreshes_command_tree`. The shared
  `deop_reaches_rejoining_native_session` calls `deop` after the admission
  completes, so it never enters the window and is not evidence here.

## Limits

- Predicted, not observed: no test, build or formatter was run when this map was
  written; Main runs the red/restore.
- `docs/bugs` is excluded from the tree fingerprint; the sha256s here attest the
  bytes.
