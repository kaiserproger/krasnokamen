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
  `apply_batch_images_with_clock`, `apply_batch_entries`, every `game`-threaded match arm) — (c) later
  commits rewrote these functions (the `legacy` flag, the `SimDebt` anchored-epoch refusal, the
  `image.at(tick, held_ticks)` debt projection, the streamed `lo` boundary). At HEAD the same code is
  kept API: it constructs and passes the clock, and reversing it refuses to build without the clock the
  kept wrappers expect.
- `journal/replay.rs @@ -1302,6 +1353,9 @@`, `@@ -1313,12 +1367,26 @@`, `@@ -1326,7 +1394,8 @@`,
  `@@ -1357,6 +1426,7 @@` (`validate_replay`, `advance_replay_clocks`,
  `advance_replay_clocks_with_clock`) — (c) later commits added the `legacy` parameter, debt-anchor
  validation and the `image.at` projection; the surviving HEAD text is kept API the kept refs call.
- `native_checkpoint.rs` (3 hunks) — (b) constructs `ReplayGameClock`, observes the unrouted batches and
  passes the clock into `apply_batch_images_with_clock`/`advance_replay_clocks_with_clock`; reversing
  removes the API the kept refs use. The control keeps the clock computed and threaded.
- `tick_control.rs` (5 hunks: `held_since` field, image capture, restore, `begin_tick(output_tick)`) —
  (b) `held_since` is the durable witness `GameClockImage::held_ticks` and the kept
  `tick_control::tests::game_clock_held_restore_...` ref call; reversing it stops the control from
  compiling. It is the restart/restore half of the same row, not this control's proving path.
- `villager/persistence.rs` (2 hunks: `advance_running_clocks`, panic-deadline gate) — (b)
  `advance_running_clocks` is called by the kept `journal_touched.rs` production mirror and by the fixed
  `advance_villager`; `advance_clocks` (the pre-image body the control uses) is kept.
- `weather.rs` (2 hunks: `GameClockImage::Held { held_since }`, `held_ticks`, `LEGACY_CLOCK_ERROR`) —
  (b) the kept module calls `held_ticks`/`LEGACY_CLOCK_ERROR` and the kept tests construct `held_since`.
- Test files (`entity_state_game_clock_tests.rs`, `entity_state_persistence_tests.rs`,
  `native_game_clock_drill_tests.rs`, `native_persistence_drill_tests.rs`, the three settlements
  `*_clock_tests.rs`, `weather_tests.rs`) — (b) kept so the control compiles and runs the registered
  refs. `docs/architecture/state-ownership.md` and `docs/bugs/ledger.toml` — documentation/ledger.

## Proving refs (the row's own age assertions)

- `cargo:mc-core:core::entity_state_persistence_tests::game_clock_tests::frozen_game_clock_keeps_resident_item_age_equal_to_live_through_wal_replay`
  — `entity_state_game_clock_tests.rs:50-52`: with only the clock source reversed the frozen item ages
  by the 32 held frames and the `assert_eq!(replay.entities[&id.0].age, 5999)` / `state.age` assertions
  fail. This is the row's "held item 5999 became 6030" line.
- The other proving refs exercise the same `ReplayChunkClock` arithmetic through the checkpoint/WAL
  recovery path and must also flip; `tick_control::tests::game_clock_held_restore_...` cannot flip (it
  tests restore/capture, kept API) and is excluded by `proving_refs`.
