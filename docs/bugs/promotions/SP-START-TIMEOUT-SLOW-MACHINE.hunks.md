# SP-START-TIMEOUT-SLOW-MACHINE — hunk map for the committed reversal pair

Promotion receipt: `promotions/SP-START-TIMEOUT-SLOW-MACHINE.json` (written by `ledger promote`).
Client pair: reversal `client:7d5ce3e949` → re-land `client:c80d1ffe` (the row's `fix_commit`).

## Why a reversal and not the pre-image pin

The real fix is `6a8d255a` "fix: improve weak hardware singleplayer startup", a bulk commit whose
eleven files carry five behaviours at once, and none of its hunks reverse-apply on the current client
tip (checked 2026-11-28:

```
git show 6a8d255a -- src/singleplayer.rs src/lib.rs … | git apply --reverse --check
error: patch failed: src/singleplayer.rs:31
error: patch failed: src/session/actions.rs:5
…
```

). The committed reversal pair keeps the tip, reverses only this row's guarded behaviour and
re-lands it, which is the same pair shape as `MODEL-ITEM-BARS` / `NATIVE-REPLACEMENT-INPUT-CLEARED`.

## Original commit

`client:6a8d255a` — `src/singleplayer.rs` removed the fixed 180 s startup deadline
(`START_TIMEOUT`, `recv_timeout(START_TIMEOUT)`, and the `"Server did not start in time"` report)
and waits on the server's event stream instead. Its other hunks are the software-adapter weak-hardware
defaults and the startup-progress relay, which are separate behaviours.

## Hunks carried by the pair

| file | hunk | content | disposition |
|---|---|---|---|
| `src/singleplayer.rs` | `@@ -31,7 +31,6 @@` | `const START_TIMEOUT: Duration = Duration::from_secs(180);` deleted | REVERSED — the constant is re-added |
| `src/singleplayer.rs` | `@@ -698,17 +704,20 @@` (the loop head only) | `match server.events.recv_timeout(START_TIMEOUT) {` → `match server.events.recv() {`, plus its two comment lines | REVERSED — `recv_timeout(START_TIMEOUT)` and the pre-fix shape restored; the same hunk's `starting`/`Progress` lines are kept (see (b) below) |
| `src/singleplayer.rs` | `@@ -746,7 +759,7 @@` | `"Server did not start in time"` → `"Server startup event stream closed"` | REVERSED — the pre-fix message restored, so the control fails with the row's own symptom text |

`git diff 7d5ce3e949 c80d1ffe` is exactly those three deletions plus the two comment lines and
`patch-id --verbatim` is the fix's non-test patch (promote records `fix_patch_id`). The re-land tree is
identical to `client:c879f0ae38b7` (`git diff c879f0ae38b7 c80d1ffe --stat` is empty), so the restore
pin is the current client tip plus the re-land commit.

## Omitted hunks and why

Test hunks and `README.md` are outside `non_test_patch` and need no entry.

| file | hunks | reason |
|---|---|---|
| `src/lib.rs` | `@@ -1104,6 +1104,7 @@` | (a) weak-hardware adapter defaults (the row is the deadline) |
| `src/local_config.rs` | `@@ -209`, `@@ -217`, `@@ -312`, `@@ -323` | (a) weak-hardware settings defaults |
| `src/render.rs` | `@@ -113`, `@@ -139`, `@@ -184`, `@@ -437`, `@@ -543`, `@@ -783`, `@@ -861`, `@@ -882` | (a) software-adapter defaults and far-terrain backend selection |
| `src/render/far_generate.rs` | `@@ -3`, `@@ -23` | (a) far-terrain generation on a software adapter |
| `src/session.rs` | `@@ -159`, `@@ -261` | (a) the weak-hardware `client_defaults` field and its initializer |
| `src/session/actions.rs` | `@@ -5`, `@@ -729`, `@@ -746`, `@@ -779`, `@@ -847` | (a) the UI/session progress publication behaviour |
| `src/session/local_server.rs` | `@@ -168` | (a) progress propagation from the local-server launch path |
| `src/singleplayer.rs` | `@@ -477,7 +476,7 @@` | (a) a comment in the generated singleplayer config (share_terrain_keys wording) |
| `src/singleplayer.rs` | `@@ -517,6 +516,8 @@`, `@@ -607,6 +608,7 @@`, `@@ -621,6 +623,8 @@`, `@@ -655,6 +659,7 @@`, `@@ -666,6 +671,7 @@`, `@@ -722,8 +731,11 @@`, `@@ -732,6 +744,7 @@`, `@@ -1005,6 +1018,7 @@` | (b) the `Event::Progress` variant, the `starting` flag, the `pump`/`spawn_server` progress parameters and their call sites — the kept base test `startup_pushes_progress_before_waiting_for_server_readiness` constructs this API; reversing it leaves the control's own test file uncompilable |

## Proving ref

`harness:singleplayer-regressions:native-sp-slow-machine` → `startup_crossed_former_deadline`
(`tools/harness/backends/singleplayer_regressions.py`, the check now evaluated before every
listening-only check).

- Restore (`c80d1ffe`): the 1 %-CPU release server reaches `is listening` after more than 180 s and the
  client keeps waiting, so the assertion is true and the run is `passed`.
- Control (`7d5ce3e949`): the 180 s `recv_timeout` fires, the client reports
  `"Server did not start in time"` and stops the server, so `server_outcome` is `exited`,
  `startup_crossed_former_deadline` is recorded false and the run is `failed` at that assertion.

The harness records a failed singleplayer run's own assertion map (`ctx.failure_scenario`) so the
control is distinguishable from a timeout or a crashed client.
