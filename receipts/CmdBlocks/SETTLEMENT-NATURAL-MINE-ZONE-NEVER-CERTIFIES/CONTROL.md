# SETTLEMENT-NATURAL-MINE-ZONE-NEVER-CERTIFIES — control on bbf5b71e5

Fix identity `patch:3006c98d6f760c6d2184875e2fe2f5296d19aa1fc886ece8bc678ecf4c435e35`
(`docs/bugs/promotions/SETTLEMENT-NATURAL-MINE-ZONE-NEVER-CERTIFIES.patch`, 7
hunks, one production file). This revision drops `daacf2338`'s const hunk
`NATURAL_ZONE_ENTRANCE_HOLD_TICKS` (class b): the candidate tests read the const
(`autonomy_tests.rs:1246,1251`), and `ledger promote` keeps test files
unmodified, so the const stays in the control tree, unused by the reversed
production path (dead-code warning only).

- `git apply -R --check` passes at `bbf5b71e5`.
- RED: `.analysis/validation/20261128T091846-test-a6mgtipl` FAIL, exit 100, at
  the row's own `natural_mine_queue_tests.rs:251`, `no mine candidate queued:
  every probe box holds a built piece in the quarry`.
- GREEN: `.analysis/validation/20261128T092429-test-07wzg06v` PASS
  (`settlements::autonomy::tests::natural_mine_queue_tests::a_plains_village_queues_proves_and_certifies_its_mine`).
- Tree restored: no `.rs` diff after the control.

The real-client receipt remains `harness:civilization-seed-qa:new_house`, a
variable gate on the merged pin — outside this worker's reach.
