# SP-PLAYER-ROTATION-LOST-ON-REOPEN: patch-mode fix identity

The fix is in two commits of this branch, because the API shape and the
behaviour had to be separable for a compiling control:

- `932a23f4ea` (with the review-10 docs) carries `JoinedSpawn`'s `rotation`
  field, the registry/pump/connection plumbing that fills it, `JoinBurst`'s
  `rotation` input, and the behaviour line below.
- `5e233f40ff` carries the pump/test fixtures that construct the struct.

`fix_commit = "patch:4f267ef894e374b77995f0b77882a7195e801aaacef8fbf354bad26a602c21b0"`
names only the behaviour in `docs/bugs/promotions/SP-PLAYER-ROTATION-LOST-ON-REOPEN.patch`:
the join `SynchronizePlayerPosition` sends the restored rotation instead of
the hardcoded default. `git apply -R` restores `yaw: 0.0, pitch: 0.0`.
Re-audit:
`python3 -m tools.harness ledger promote SP-PLAYER-ROTATION-LOST-ON-REOPEN --fix-patch docs/bugs/promotions/SP-PLAYER-ROTATION-LOST-ON-REOPEN.patch --dry-run`

## Why the behaviour and not the whole fix

Reversing the whole fix removes `JoinedSpawn.rotation`, `JoinBurst.rotation`
and the plumbing that the kept refs construct
(`play_tests.rs` builds `JoinBurst { rotation: [90.0, 20.0], … }` and
`server_view_distance_tests.rs` builds `JoinedSpawn { rotation, … }`), so the
control would not compile (E0560). The API is class (b) and stays on both
sides; only the wire value is reversed.

## Carried (1 hunk, the real pre-image)

| original (HEAD before the fix) | candidate |
|---|---|
| `crates/mc-edge/src/play.rs:184-185` `yaw: 0.0, pitch: 0.0` | `yaw: rotation[0], pitch: rotation[1]` |

The `a/`-side is the tip's pre-fix text at the same lines (the change that
introduced the bug was the fix's own predecessor, `932a23f4ea`'s parent), so
nothing outside the pre-image is written.

## Excluded

- `JoinedSpawn.rotation` and the registry/pump/connection plumbing
  (`server.rs`, `server/registry.rs`, `server/connection.rs`, `server/pump.rs`)
  — (b) API, as above.
- `play_tests.rs`'s new regression and the singleplayer scenario — test files,
  kept on both sides.

## Proving refs

- `harness:singleplayer-regressions:native-sp-rotation-reopen`, proving
  assertion `reopen_keeps_rotation`
  (`tools/harness/backends/singleplayer_regressions.py`): the client looks
  90/20, pauses, saves and quits to title (the world's server exits), reopens
  the world and reads its camera. Reversed, the reopened camera is 0/0.
- `cargo:mc-edge:play::tests::join_burst_syncs_the_restored_player_rotation` —
  red in the same control at the `(sync.yaw, sync.pitch) == (90.0, 20.0)`
  assertion, green on the candidate (`.analysis/validation/20261124T052712-test-0lx_aczo`).
