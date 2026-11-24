# BED-OCCUPIED-PROPERTY-NOT-MIRRORED — fix-hunk map

The production patch `BED-OCCUPIED-PROPERTY-NOT-MIRRORED.patch` is the fix's non-test hunks,
`patch:06c5dace1735a3c911934b460aba6dd11d71183184d6533ba798334bb733fd65`. It reverse-applies on the
candidate (`git apply -R --check`).

## The fix

| file | hunk |
|---|---|
| `crates/mc-core/src/tick.rs` | `TickState::occupied_beds: HashSet<BlockPos>` — the derived set of beds the last pass mirrored, never persisted |
| `crates/mc-core/src/tick/state.rs` | its `HashSet::default()` initialiser |
| `crates/mc-core/src/sleep.rs` | `mirror_occupied_beds` (wanted = players' `sleepers` ∪ sleeping residents' `home`; clears a previously occupied bed that lost its sleeper) and `set_bed_occupied` (writes `occupied` on **both halves**, only when it differs, and never on a bed an accepted write already targets) |
| `crates/mc-core/src/tick/dimension.rs` | the call **after `state.merge`** and before the block phase |

## Why after the merge (review B1, 2026-11-24)

A break/replace intent carries `block_expect` for both bed halves and the merge rejects a whole intent
whose expect no longer matches (or whose cell was already written this tick). A mirror that ran before
the merge therefore invalidated a same-tick break of that bed. Running it after the merge, and skipping
a bed pair whose cell is already in `merge_block_writes`, makes the accepted write win: a broken bed is
never re-occupied and a same-tick break is accepted. (The first version ran inside `sleep::run`, after
the block phase, so its write landed a tick late and could re-apply an occupied bed over a break — the
v1 defect the block-phase ordering had already fixed in the other direction.)

## a/-side

The real pre-image: no `occupied_beds` field, no mirror function, no call. No adaptation line; the
tests live in `sleep_tests.rs` and are not part of the patch.

## Control

Reversing the patch reds the two proving refs at their own assertions
(`crates/mc-core/src/sleep_tests.rs:1298` "the sleeper's bed must read occupied" and `:1322`
"the resident's bed must read occupied"; observed `20261124T100301-test-6_b51pwu` on the first
version of the reverse tree). The B1 regression now also proves the CAS direction:

- `a_bed_broken_in_the_tick_its_player_lies_down_is_not_restored` — one tick with the lie-down and a
  `Dig::Finish` on the head; the break is accepted and the next tick does not restore the bed;
- `a_bed_broken_when_the_property_clears_is_not_restored` — the player is up, the mirror wants
  `occupied=false`, the same-tick break wins and the bed stays broken.

## Vanilla parity (review non-blocker, checked)

`BedBlock.updateShape` copies `OCCUPIED` to the connected half, so both halves carry the state; the
palette has `part=foot, occupied=true` states (`crates/mc-data/data/required_blocks.json`, e.g.
`minecraft:black_bed` id 2271), so the pair write is legal. The mirror writes both.

## Recorded gaps (not closed here)

- `occupied_beds` is runtime state. A checkpoint taken while a player sleeps, followed by a crash or a
  stop before any tick, leaves `occupied=true` in the saved world with an empty derived set; the next
  wake cannot clear it. Residents restore because their `sleeping` is persisted. Gameplay is unaffected
  (`use_bed` derives occupancy from `state.sleepers`, never from the property). A cheap follow-up is to
  re-derive `occupied_beds` on load by scanning the loaded beds once.
- The pass walks the chunk-indexed resident list every tick (one `row()` per resident). It is bounded
  by the resident count and empty in non-settlement worlds; deriving from the brain's sleep transitions
  would remove the scan and is left as an optimisation.
- The property is client-visible: promotion still needs a scene that reads the bed head during sleep
  (`OWNER-DECISIONS-NEEDED.md` item 2).

## Regenerated 2026-11-24 (`patch:d3314283…`, was `patch:06c5dace…`)

The recorded patch stopped reversing on the candidate: its `accepted()` helper was the v1 linear scan,
while `71261486d` (review N4) made it one pass-local `FxHashSet<BlockPos>`. A stale patch is refused by
the ledger ("does not apply in reverse on the candidate"), so the patch was regenerated from git
history, not edited: `git apply -R --3way` of the old patch (three files clean, `sleep.rs` conflicted),
the conflict region being exactly the two added functions, then `git diff -R` against HEAD restricted
to the four production files. The a/- side is therefore the real pre-image (no `occupied_beds`, no
mirror, no call) and the b/- side is HEAD after `71261486d`. Verified: `git apply -R --check` passes on
`292c3b7a0`.
