# BED-OCCUPIED-PROPERTY-NOT-MIRRORED — fix-hunk map

The production patch `BED-OCCUPIED-PROPERTY-NOT-MIRRORED.patch` is the fix's non-test hunks,
`patch:9e162cbcf0e1a316c338a0cd8f3753b97f3db6ff8ffd5e5c3df23d45421cab2c`. It reverse-applies on the
candidate (`git apply -R --check`).

## The fix

| file | hunk |
|---|---|
| `crates/mc-core/src/tick.rs` | `TickState::occupied_beds: HashSet<BlockPos>` — the derived set of beds the last pass mirrored, never persisted |
| `crates/mc-core/src/tick/state.rs` | its `HashSet::default()` initialiser |
| `crates/mc-core/src/sleep.rs` | `mirror_occupied_beds` (wanted = players' `sleepers` ∪ sleeping residents' `home`; clears a previously occupied bed that lost its sleeper) and `set_bed_occupied` (writes the `occupied` property only when it differs) |
| `crates/mc-core/src/tick/dimension.rs` | the call in the commit path, after the phases that can start or end a sleep and before the merge (so the write lands in the same block phase and a break staged by the same tick wins) |

The first version put the call inside `sleep::run` (after the block phase); that landed the write one
tick late and could re-apply an occupied bed over a block broken in between
(`sleep::tests::breaking_the_bed_or_taking_a_hit_wakes_the_sleeper` caught it). The move to the
pre-merge commit point is the second commit of the fix series.

## a/-side

The real pre-image: no `occupied_beds` field, no mirror function, no call. No adaptation line; the
tests live in `sleep_tests.rs` and are not part of the patch.

## Control

Reversing the patch reds both registered refs at their own assertions:
`crates/mc-core/src/sleep_tests.rs:1298` ("the sleeper's bed must read occupied") and `:1322`
("the resident's bed must read occupied") — observed as `20261124T100301-test-6_b51pwu` for the
first version of the reverse tree; the same reversal now also removes the pre-merge call. Green on the
fixed tree: `20261124T094119-test-jtmh3vfr` (first version) and `20261124T112017-test-67gp6f1z` (all
`sleep::tests`, after the move).

## What stays out of the row

Bed *use* (who may sleep in a bed, the "occupied" refusal message, `kick_out_of_bed`) is unchanged: it
derives from the sleepers as before. The property is client-visible, so promotion still needs a scene
that reads the bed head during sleep (`OWNER-DECISIONS-NEEDED.md` item 2).
