# CIV-FIXTURE-INACTIVE-MINE — HEAD re-break control patch

Patch: `docs/bugs/promotions/CIV-FIXTURE-INACTIVE-MINE.patch` (`patch:939c0dde974f31d77fedc01ab4482eabab007e1636aa7d0339a94eb01a4b2db5`), the row's
`fix_commit`. It reverses on the committed tree `6ef75a905` (`git apply -R
--check`, plain context) and is what a promotion passes as `--fix-patch`.

## What the fix is (origin `4e6874ded`)

Files: tools/harness/backends/village_autonomy_qa.py.

The civilization fixture raises only `simulation_distance` 4->6 so the village mine ticks beyond the four-chunk presentation window.

## HEAD equivalents and omissions

From the CivSim-5 slice commit. The other harness hunks (entity census, ecology, capture helpers) are **omitted (a)**: they are other rows' behaviour.

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/CIV-FIXTURE-INACTIVE-MINE.patch` on `6ef75a905`
succeeds (clean tree); the real-client red is the later integrated sweep's
control run, not claimed here.
