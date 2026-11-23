# CIV-HOUSE-CAPTURE-CLOSED-ENTRANCE — HEAD re-break control patch

Patch: `docs/bugs/promotions/CIV-HOUSE-CAPTURE-CLOSED-ENTRANCE.patch` (`patch:88c13dfe5026f7daa54a5fa033e70914b564ac575a321ff1d5fe5b05b864d006`), the row's
`fix_commit`. It reverses on the committed tree `6ef75a905` (`git apply -R
--check`, plain context) and is what a promotion passes as `--fix-patch`.

## What the fix is (origin `4e6874ded`)

Files: tools/harness/backends/civilization_seed_qa.py.

The `new_house` milestone capture opens the received entrance (look + use, wait for both halves open) and returns the observer outside before the exact bed raycast.

## HEAD equivalents and omissions

The re-break reverts the call site to the pre-fix `capture_target`, so the capture navigates through a closed door again. The `capture_house` addition is **omitted (b)**: the kept python tests call `qa.capture_house` directly; deleting it would stop the control's tests from collecting, and the helper being defined is not behaviour.

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/CIV-HOUSE-CAPTURE-CLOSED-ENTRANCE.patch` on `6ef75a905`
succeeds (clean tree); the real-client red is the later integrated sweep's
control run, not claimed here.
