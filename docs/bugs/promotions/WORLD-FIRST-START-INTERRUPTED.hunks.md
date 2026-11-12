# WORLD-FIRST-START-INTERRUPTED: 922d9dd6 hunk map (patch mode)

Reversing all non-test hunks of 922d9dd6 leaves a tree that does not build: `core.rs` has moved since
(the commit no longer reverse-applies on main) and the kept tests use the `FIRST_START_MARKER`
constant, the `FAIL_CHECKPOINT_AFTER_REGIONS` set and `NativeStore::has_checkpoint`. The fix patch
holds the one behaviour hunk against the head `684e6adc`.

| 922d9dd6 hunk | what it does | in the fix patch |
|---|---|---|
| core.rs `Core::open` (`let resumed = first_start.exists();`) | **behaviour**: a start that finds `.solaris-first-start` repeats the first start on what the stopped one left, instead of opening the half-written store as an existing world and refusing with "native global/checkpoint: missing" | **yes** |
| core.rs (marker written first, store created as `state.redb.new` and renamed, marker removed after the initial checkpoint, `has_checkpoint` check) | the marker's life cycle; only acts together with the line above | no: inert without it (the marker is written and removed, never read) |
| core/journal.rs, tests, `FAIL_CHECKPOINT_AFTER_REGIONS` as a set | fault-injection hook shared by the kept tests | no: test API |
| mc-world native/checkpoint.rs (`initialize_dimensions` accepts the identical descriptor again, `has_checkpoint`) | supports the repeated first start | no: harmless without the marker read |
| tests, docs | | no |

Control (20261111T183404-ledger-promote-_eswwgbb): with the hunk reversed,
`first_start_stopped_before_its_initial_checkpoint_reopens_as_a_new_world` is red (second
`Core::open` of the interrupted world refuses). The real-client scenario
`native-world-first-start-interrupted` failed in the control tree (exit 1) but its receipt carries no
negative verdict, so it counts as unrunnable there and the client surface is proven only on the fixed
tree (probe 20261111T181442-ledger-probe-n22d678n, scenario run
20261111T182041-native-world-first-start-interrupted-arfibx8c passed). Restored: the cargo ref is
green; the client ref is covered by that receipt.
