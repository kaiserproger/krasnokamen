# CIV-STALL-ORACLE-WAITING-MATERIALS — HEAD re-break control patch

Patch: `docs/bugs/promotions/CIV-STALL-ORACLE-WAITING-MATERIALS.patch` (`patch:2376e539be37f932a488d5809095410de40b951f29e7ed3f36b7e97b24d14ac2`), the row's
`fix_commit`. It reverses on the committed tree `6ef75a905` (`git apply -R
--check`, plain context) and is what a promotion passes as `--fix-patch`.

## What the fix is (origin `7ed4d504`)

Files: tools/harness/backends/civilization_seed_qa.py.

`stall_signature` keeps only the active project's own dialog line while a project holds the slot; without one the waiting lists still count as progress.

## HEAD equivalents and omissions

One file, one function. The commit's python test additions are excluded as test files. The original patch needed context reduction on the candidate; this patch was re-derived from the reversed HEAD tree with default context.

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/CIV-STALL-ORACLE-WAITING-MATERIALS.patch` on `6ef75a905`
succeeds (clean tree); the real-client red is the later integrated sweep's
control run, not claimed here.
