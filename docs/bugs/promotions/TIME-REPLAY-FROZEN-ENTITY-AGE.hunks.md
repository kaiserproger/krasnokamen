# TIME-REPLAY-FROZEN-ENTITY-AGE — hunk map (patch mode, kept API)

Row: `TIME-REPLAY-FROZEN-ENTITY-AGE` (gameplay, major) — entity recovery counted held native frames as
active simulation, ageing frozen items ("held item 5999 became 6030").

Real fix: `4a2ef89f9` ("fix(core): replay entity clocks in global running time"). It created
`crates/mc-core/src/journal/replay_game_clock.rs` (the persistent running-time prefix) and rewired
`ReplayChunkClock` to ask that prefix instead of the native tick difference. The whole commit no longer
reverses on this tip: `journal/replay.rs` was rewritten again by later commits (`apply_batch_entries`
split, `legacy` parameter, debt-anchor validation, `image.at`), and `replay_game_clock.rs` did not exist
before the fix, so deleting it deletes API the kept tests drive.

Patch mode therefore carries only the HEAD equivalents of the hunks that decide *where the age comes
from*: the `ReplayChunkClock` bodies. `journal/replay_game_clock.rs` stays compiled; the `game:
&ReplayGameClock` arguments stay threaded through every call site (kept API); the reversed bodies ignore
them, exactly the pre-fix arithmetic.

## Carried (HEAD equivalents)

| original hunk (`4a2ef89f9`, `journal/replay.rs`) | HEAD equivalent | note |
|---|---|---|
| `@@ -97,19 +100,25 @@` `ReplayChunkClock::through` | `crates/mc-core/src/journal/replay.rs:102-111` | `point.elapsed + game.between(point.tick, tick)` replaces `tick - point.tick`; carried verbatim |
| `@@ -125,9 +134,9 @@` `ReplayChunkClock::observe` active branch | `replay.rs:130-134` | `self.through(..) + game.between(.., ..)` replaces `self.through(..) + 1`; carried verbatim |
| `@@ -143,6 +152,7 @@` `advance_villager` parameter | `replay.rs:144-150` | carried with the body hunk |
| `@@ -155,8 +165,15 @@` `advance_villager` active loop | `replay.rs:160-176` | `game.running_intervals(..) { advance_running_clocks(.., game.last_running_at(first), first + 1, last) }` replaces `advance_clocks(villager, start, end)`; carried verbatim |

**a/-side adaptation (named, per README:250-258):** the reversed `through` and `advance_villager` keep
their `game: &ReplayGameClock` parameter as `_game` (a `_`-parameter rename) because every caller
(`between`, `observe`, `advance_replay_clocks_with_clock`) is kept API that passes it; deleting the
parameter stops the control from compiling. The parameter is unused in the control, so the control
proves the *clock source* the row lacked, not the signature. `ReplayChunkClock::between` keeps its
`game` parameter and body unchanged (it only forwards to `through`); its signature hunk is class (b).

## Left out

- `journal/replay_game_clock.rs` (new, 182 lines) — (b) whole module kept: the kept refs construct
  `ReplayGameClock` and the fix's own doc block names it as the retained prefix.
- `journal.rs @@ -28,6 +28,8 @@` (`mod replay_game_clock; pub(crate) use ...`) — (b) the module
  declaration the kept module needs.
- `journal/replay.rs @@ -43,6 +43,9 @@` (`Replay::game_clock`, `Replay::clock_fault`) — (b) fields the
  kept tests and `native_checkpoint.rs` set/read.
- `journal/replay.rs @@ -621,6 +638,45 @@` and `@@ -629,651 +685,646 @@` (`apply_batch_images`,
  `apply_batch_images_with_clock`, `apply_batch_entries`, every `game`-threaded match arm) — (b) kept
  API: at HEAD these functions construct, observe and pass the `ReplayGameClock` the kept tests and
  `native_checkpoint.rs` call; reversing them removes that API and the control does not build. Later
  commits also extended the area (the `legacy` flag, the `SimDebt` anchored-epoch refusal, the
  `image.at(tick, held_ticks)` projection, the streamed `lo` boundary), which is why the class is kept
  API rather than a carried hunk.
- `journal/replay.rs @@ -1302,6 +1353,9 @@`, `@@ -1313,12 +1367,26 @@`, `@@ -1326,7 +1394,8 @@`,
  `@@ -1357,6 +1426,7 @@` (`validate_replay`, `advance_replay_clocks`,
  `advance_replay_clocks_with_clock`) — (b) kept API: these wrappers and the `legacy`/held-tick
  parameters they pass are called by the kept tests and `native_checkpoint.rs`; later commits added the
  debt-anchor validation and the `image.at` projection to the same text.
- `native_checkpoint.rs` (3 hunks) — (b) constructs `ReplayGameClock`, observes the unrouted batches and
  passes the clock into `apply_batch_images_with_clock`/`advance_replay_clocks_with_clock`; reversing
  removes the API the kept refs use. The control keeps the clock computed and threaded.
- `tick_control.rs` (5 hunks: `held_since` field, image capture, restore, `begin_tick(output_tick)`) —
  (b) `held_since` is the durable witness `GameClockImage::held_ticks` and the kept
  `tick_control::tests::game_clock_held_restore_...` ref call; reversing it stops the control from
  compiling. It is the restart/restore half of the same row, not this control's proving path.
- `villager/persistence.rs @@ -333,7 +333,15 @@` (`advance_running_clocks`) — (b) the helper is called
  by the kept `journal_touched.rs` production mirror and by the fixed `advance_villager`; `advance_clocks`
  (the pre-image body the control uses) is kept.
- `villager/persistence.rs @@ -348,7 +356,11 @@` (the panic-deadline `panic_until >= first` gate) — (a)
  another row's behaviour: the control's reversed arithmetic never reaches it (the op is a no-op for
  `from + 99 == panic_until`, as the review measured against A1Int), and no registered ref asserts it.
- `weather.rs` (2 hunks: `GameClockImage::Held { held_since }`, `held_ticks`, `LEGACY_CLOCK_ERROR`) —
  (b) the kept module calls `held_ticks`/`LEGACY_CLOCK_ERROR` and the kept tests construct `held_since`.
- Test files (`entity_state_game_clock_tests.rs`, `entity_state_persistence_tests.rs`,
  `native_game_clock_drill_tests.rs`, `native_persistence_drill_tests.rs`, the three settlements
  `*_clock_tests.rs`, `weather_tests.rs`) — (b) kept so the control compiles and runs the registered
  refs. `docs/architecture/state-ownership.md` and `docs/bugs/ledger.toml` — documentation/ledger.

## Proving refs (the row's own age assertions; measured 2026-11-25 by the promote)

Every red below is an `assertion` panic on the row's own age/memory line, not a count or a fixture
(`.analysis/validation/20261125T202736-ledger-promote-20chdygl/promote.json`):

- `cargo:mc-core:core::entity_state_persistence_tests::game_clock_tests::frozen_game_clock_keeps_resident_item_age_equal_to_live_through_wal_replay`
  — red at `entity_state_game_clock_tests.rs:47`
  (`assert_eq!(replay.entities[&id.0].age, 5999)`), the row's "held item 5999 became 6030" line.
- `cargo:mc-core:core::entity_state_persistence_tests::game_clock_tests::game_clock_mid_hold_checkpoint_bridges_fresh_memories_not_stale_or_expired_ones`
  — red at `entity_state_game_clock_tests.rs:309` (`assert_eq!(memory.age, -200)`).
- `cargo:mc-core:core::native_checkpoint_crash_tests::persistence_drills::game_clock_tests::native_frozen_item_clock_survives_checkpoint_tail_recovery_and_frozen_open`
  — red at `native_game_clock_drill_tests.rs:39` (`assert_eq!(restored.state.entities.row(slot).age, 5999)`).
- `cargo:mc-core:core::native_checkpoint_crash_tests::persistence_drills::game_clock_tests::native_game_clock_overworld_only_transition_controls_nether_and_end_items`
  — red at `native_game_clock_drill_tests.rs:149` (`assert_eq!(record.age, 5999)`).
- `cargo:mc-core:tick_control::tests::game_clock_held_restore_preserves_capture_before_first_tick_and_restart_modes`
  — green in the control (it tests capture/restore of `TickControl`, kept API), so it is excluded by
  `proving_refs` as the rule requires.
