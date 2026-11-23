# ISSUE-1-16 — HEAD re-break control patch

Patch: `docs/bugs/promotions/ISSUE-1-16.patch`
(`patch:53ff4aa3198230a3…`), the row's `fix_commit`. It reverses on the
committed tree (`git apply -R --check`).

## What the fix is

The wall blueprint's registration in
`settlements/blueprints.rs` — `SOURCES` carries
`assets/blueprints/wall.json`. Reversed, the wall blueprint is absent, so a wall
cannot be ordered, built or certified and `harness:buildings-qa:wall.L1_built`
is red.

## Original patch and HEAD equivalent

The recorded fix patch (`docs/bugs/fix-patches/ISSUE-1-16.patch`, digest
`patch:a3d1b2cd…`) expects `SOURCES: [&str; 9] -> 10` plus the wall line. HEAD
has grown the array to 26 entries, so that patch no longer reverses; this patch
is the same change at HEAD (`25 -> 26` and the wall line), derived by removing
wall.json from the current array and re-emitting the fix direction. Test files
stay in the control.

## Reverse-apply evidence

`git apply -R --check` on the committed tree succeeds; the graphical red is the
integrated sweep's control run, not claimed here.
