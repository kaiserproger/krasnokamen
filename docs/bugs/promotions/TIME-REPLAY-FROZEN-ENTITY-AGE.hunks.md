# TIME-REPLAY-FROZEN-ENTITY-AGE: patch-mode fix identity

Original fix: `4a2ef89f9` "fix(core): replay entity clocks in global running time".
`fix_commit = "patch:b612a95035de6c0adba12209857fe7ac6a6eae41d6ee8661af138675a3160964"`
names the held-branch age of that fix on the candidate in
`docs/bugs/promotions/TIME-REPLAY-FROZEN-ENTITY-AGE.patch`; `git apply -R`
makes a held frame age items again.
Re-audit:
`python3 -m tools.harness ledger promote TIME-REPLAY-FROZEN-ENTITY-AGE --fix-patch docs/bugs/promotions/TIME-REPLAY-FROZEN-ENTITY-AGE.patch --dry-run`

## Why not the commit itself

`4a2ef89f9` rewrote `journal/replay.rs` wholesale and touched
`native_checkpoint.rs`, the settlements clocks and their tests; the stage-0
merges moved that text, so the commit's hunks no longer reverse (`git apply -R`
fails). Class (c): a later commit rewrote the text, and it stays on both sides.

## Carried (1 hunk)

| original | HEAD |
|---|---|
| replay_game_clock.rs `game_ticks` held branch returns the frozen value | journal/replay_game_clock.rs:213-218 |

## The control

Reversed, `ReplayGameClock::game_ticks` adds the held frames
(`game_ticks + tick - since`), and the registered refs fail by assertion:

- `core::entity_state_persistence_tests::game_clock_tests::frozen_game_clock_keeps_resident_item_age_equal_to_live_through_wal_replay`
  at `entity_state_game_clock_tests.rs:47`: `left: 6030`, `right: 5999` — the
  exact symptom the row records (held item 5999 became 6030).
- `core::native_checkpoint_crash_tests::persistence_drills::game_clock_tests::native_frozen_item_clock_survives_checkpoint_tail_recovery_and_frozen_open`
  at `native_game_clock_drill_tests.rs:39`.

`game_clock_mid_hold_checkpoint_bridges_fresh_memories_not_stale_or_expired_ones`
was green in that control and is a green regression, not control evidence.
