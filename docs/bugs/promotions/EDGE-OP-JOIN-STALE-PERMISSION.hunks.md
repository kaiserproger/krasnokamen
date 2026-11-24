# EDGE-OP-JOIN-STALE-PERMISSION — fix-hunk map

The row's fix is `0504a2800c3b43fc06a8959f8bd809b5be116478` (`fix: fence operator permissions during
session admission`). This patch is the forward-ported fix at HEAD,
`docs/bugs/fix-patches/EDGE-OP-JOIN-STALE-PERMISSION.patch`, SHA-256 of its non-test hunks
`patch:7a30a926ec93da2eac398a6aafba067c0810796d57468ef82c7842c2f7786c60`; it reverse-applies on the
candidate.

Regenerated after the final review rejected the first version: the first a/-side bodies were models of
functions the control's production path did not call (`run_connection` called `join_session` directly and
`admit_native_session` became dead test-only code), so the reds came from invented bodies and the emptied
kept API. In this version the caller keeps calling the kept `admit_native_session` and only that function's
body, the burst's op level and the post-burst registration are the pre-fix code.

## a/-side (the pre-fix code at HEAD)

- `admit_native_session` body: `let level = server.admin.level(uuid); let session =
  server.hub.join_session(id, uuid, name, level, settings);` plus the kept test rendezvous
  (`#[cfg(test)] admit_rendezvous(uuid).await`). No operator fence, no `admin.register` — the level is read
  outside the fence, so an op/deop can overtake the Join it targets. The kept signature (including `ip`) is
  unchanged; `ip` is unused by the pre-fix body (`let _ = ip;`).
- the burst level: `op_level: if session_locked(id) { 0 } else { admin.level(outcome.uuid) }` (pre-fix),
  not the registered `session_level(id)`.
- the registration after the join burst: `if let Some(ip) = outcome.remote_address.or(stream_peer_ip(&peer))
  { server.admin.register(id, outcome.uuid, outcome.name.clone(), ip); }` — the pre-fix post-burst
  registration, with no `operator_updates` guard and no `refresh_join_permission_tree` call.

## Retained hunks and why

| Retained | Reason |
|---|---|
| `admit_native_session` signature (incl. `ip`) and the caller's `admit_native_session(…)` call at `connection.rs:393` | (b) the kept refs call `admit_native_session` directly (`admin_tests.rs:298`, `:335`, `:385`, `:514`, `:553` — `:58`/`:536` are definitions); a control that bypassed it would make the reds come from dead code (the rejected first version). |
| `refresh_join_permission_tree` body | (b) the kept ref calls it directly (`admin_tests.rs:599`, `:613`); keeping it green is required by the restore, and it is listed below as not evidence. |
| `server.operator_updates` field/plumbing | (b) the kept fence test takes that lock (`joining_session_and_deop_keep_native_permission_order`). The **burst guard itself is not retained**: the a/-side removes `let operator_guard = server.operator_updates.lock().await;` and its `drop` (hunk `@@ -484`), exactly the pre-fix code, which held no guard around registration. |

## Kept caller behaviour on the a/-side

Pre-fix, `server/connection.rs` had **no** IP check at all: `server.hub.join_session(…)` ran for every
connection and registration sat behind the join result, not behind an IP (`0504a280^` lines 219-230 and
330-334 of `server/connection.rs`; the commit `0504a280` changes `server/connection.rs`, not
`server.rs`). The `remote IP unavailable` rejection at `connection.rs:389-392` is a **later** caller
change (the pre-join `admit_native_session(…, ip, …)` call needs a remote address), not part of the
0504a280 fix, and it sits on both sides of the patch. The a/-side keeps it because the retained
`admit_native_session` signature takes `ip` (rule (b)); the function's body ignores the address
(`let _ = ip;`) and does not register — pre-fix registration behaviour. The kept refs call
`admit_native_session` directly, so they are unaffected by the caller's IP handling.

## Control reds and which are evidence

| ref | control | evidence? |
|---|---|---|
| `deop_between_join_submission_and_late_registration_keeps_native_level` | **red** at `admin_tests.rs:491` — `native permission level publication: connection 2 wants 0, published Some(4)` | **Yes — the row's symptom**: a real `deop` between Join submission and late registration never reaches the native Join, and the authoritative published level stays 4. The edge-level assert at `:586` (`session_level(id) == 0`) is green; the red is the core publication. |
| `deop_reaches_rejoining_native_session` | red at `admin_tests.rs:312` (`sessions_for_uuid(…).len()` 1 vs 2) | No — an edge-registry count: in the pre-fix order the second session is unregistered until the burst, which this test never reaches. Edge state is not evidence. |
| `joining_session_and_deop_keep_native_permission_order` | red at `admin_tests.rs:401` (`Join must wait on operator fence`) | No — a precondition of the kept fence test; the pre-fix admission has no fence to wait on. |
| `join_burst_permission_change_refreshes_command_tree` | green (calls `refresh_join_permission_tree` directly) | No — the function is kept as (b); it does not exercise admission ordering. |
| `deop_between_snapshot_and_registration_reaches_native_join` | green | — (registered; restore green) |

The restore is green on all five refs. The earlier row text's ":411 predicted, not observed" is superseded:
the observed evidence red is `:491`.

## Per-file map of the original `0504a280` commit

| original file | current patch |
|---|---|
| `crates/mc-edge/src/server/connection.rs` (87 +/-): creates `admit_native_session` and `refresh_join_permission_tree`, replaces the direct `join_session` call, snapshots `burst_op_level`, takes the registration guard, calls the tree refresh | **this patch**: the admission fence (body = pre-fix level snapshot), the burst-level snapshot, the guard and the refresh call are reversed; the two function signatures and the caller's call are kept (b). |
| `crates/mc-core/src/command_debug.rs`, `command_execution.rs` (7 +/- each) | (a) another row's behaviour — the command-thread/trace work; not this row's admission fence. |
| `crates/mc-core/src/tick/entity_phase.rs` (8-), `dimensions_tests.rs`, `entity_phase_tests.rs`, `journal_touched_tests.rs`, `player_tests.rs`, `tick/statistics_tests.rs`, `crates/mc-edge/src/admin_tests.rs`, `computer_tests.rs` | test hunks (kept) and their call-site updates — not a production patch. |
| `docs/bugs/ledger.toml` | documentation. |

Note: the `operator_updates` lock around `op`/`deop` (`server.rs:1030`, `:1135`) is **not** part of
`0504a280` (git: the commit holds `crates/mc-edge/src/server.rs` but does not modify it — its stat lists
`server/connection.rs` only; `git log -S operator_updates -- crates/mc-edge/src/server.rs` reaches
`d0ae64284` and older). It belongs to the later op/deop batch and stays on both sides (reason (a) for
`EDGE-QUEUED-OP-AFTER-DEOP`).
