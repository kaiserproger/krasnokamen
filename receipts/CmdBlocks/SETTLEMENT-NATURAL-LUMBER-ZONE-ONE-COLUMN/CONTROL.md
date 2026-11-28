# SETTLEMENT-NATURAL-LUMBER-ZONE-ONE-COLUMN — control on bbf5b71e5

Fix identity `patch:113f3d8cdcd459341498bbacb7401c098a5ed5418ad72e8cf2163ea416e4aa43`
(`docs/bugs/promotions/SETTLEMENT-NATURAL-LUMBER-ZONE-ONE-COLUMN.patch`, 5 hunks,
one production file). HEAD equivalent of `8a93c14578` + `544c763a44`: the 2026-11-24
patch's hunk 3 no longer reversed because `836f66b0b` replaced the certify
result with the `refused`/logging match. This revision keeps that later logging
(class a/c) and reverses only the grove growth (`candidate.bounds` instead of
`grown`).

- `git apply -R --check` passes at `bbf5b71e5`; reversal is 2 insertions / 120
  deletions in `autonomy.rs`.
- RED: `.analysis/validation/20261128T085525-test-h0jp3cnb` FAIL, exit 100, the
  row's own `natural_lumber_camp_certifies_the_grove_not_one_tree_column` panics
  at `autonomy_tests.rs:993`,
  `a natural camp's zone is the grove, not one tree column: 2x1 at [-15, 201, 4]..[-14, 206, 4]`.
- GREEN: `.analysis/validation/20261128T092429-test-07wzg06v` PASS.
- Tree restored: no `.rs` diff after the control.

`harness:civilization-seed-qa:all` (a passed gate on this tree) is the remaining
real-client proof, outside this worker's reach.
