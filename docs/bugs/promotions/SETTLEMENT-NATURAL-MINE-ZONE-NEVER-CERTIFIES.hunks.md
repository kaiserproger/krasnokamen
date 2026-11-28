# SETTLEMENT-NATURAL-MINE-ZONE-NEVER-CERTIFIES — promotion patch and its control

Row fix identity `patch:3006c98d6f760c6d2184875e2fe2f5296d19aa1fc886ece8bc678ecf4c435e35`
(`promotions/SETTLEMENT-NATURAL-MINE-ZONE-NEVER-CERTIFIES.patch`, 7 hunks, one
production file `crates/mc-core/src/settlements/autonomy.rs`). The patch
reverse-applies on the candidate: `git apply -R --check` passes at
`bbf5b71e5`.

## Commits the patch carries

The mine queue/3D-piece rule and its review waves, chiefly
`836f66b0b` "fix(settlements): a mine zone holds no built piece in 3D
(review R1-R3)" (queue_natural 3D piece rule, the probe restructure, the
`refused` logging) and `7af1eca8c` / `906fd3650` where they touched the same
probe/prove code. The hunk map below is by hunk, not by commit, because later
review waves rewrote the same functions.

## Omitted hunk: the entrance-hold const (class b)

`daacf2338` "the entrance hold is bounded by candidate age (H1-H4)" added

```
const NATURAL_ZONE_ENTRANCE_HOLD_TICKS: u64 = 3200;
```

and the `age < …` wait in `prove_natural`. A control that reverses the const
cannot compile against the unmodified candidate tests:
`settlements::autonomy::tests::loaded_but_unsimulated_natural_worksite_is_not_certified`
reads `NATURAL_ZONE_ENTRANCE_HOLD_TICKS` directly (`autonomy_tests.rs:1246,1251`).
`ledger promote` keeps test files untouched, so the hunk is left out and the
const stays in the control tree, unused by the reversed production path (a
dead-code warning only). This is the class-(b) kept-API adaptation. The
behaviour the row needs — a candidate is actually queued/proved/certified — does
not depend on the hold.

Test-only hunks of the same review waves are not in the patch: the control keeps
the candidate tests and reverses production under them.

## Red and green (both observed)

- Red — this tree with the patch reversed (`git apply -R`; one production file,
  7 hunks): `.analysis/validation/…` FAIL at the row's own
  `natural_mine_queue_tests.rs:251`, `no mine candidate queued: every probe box
  holds a built piece in the quarry`.
- Green — clean candidate: `.analysis/validation/20261128T092429-test-07wzg06v`
  PASS for `settlements::autonomy::tests::natural_mine_queue_tests::a_plains_village_queues_proves_and_certifies_its_mine`.
