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
`patch:a3d1b2cd…`, the pre-fix record kept whole in `promotions/ISSUE-1-16.origin.patch`) expects `SOURCES: [&str; 9] -> 10` plus the wall line. HEAD
has grown the array to 26 entries, so that patch no longer reverses; this patch
is the same change at HEAD (`25 -> 26` and the wall line), derived by removing
wall.json from the current array and re-emitting the fix direction. Test files
stay in the control.

## Reverse-apply evidence

`git apply -R --check` on the committed tree succeeds; the graphical red is the
integrated sweep's control run, not claimed here.

## a/-side audit (B5, 2026-11-24)

Mechanical check: `git show <origin>^:<file>` vs the file after `git apply -R`
of this patch at the audit head; script and full output in
`receipts/D123/D123CivGates/a-side-audit.md`. Every line outside the origin
commit's own change, with the commit that last changed it and whether the
line is in `origin^:file`:

- `crates/mc-core/src/settlements/blueprints.rs` `@@ -32,7 +32,7 @@ pub type ItemId = u32;` (adapted; origin `23aad8f43`, discovered origin):
  - a/-side `const SOURCES: [&str; 25] = [` — no commit; in origin^: no

HEAD-side lines whose last change is another commit are that later commit's
form: reversing them is the HEAD drift a control must reverse to reach the
pre-fix decision (rule (c)), not part of the fix. An a/-side line that is not
in `origin^` is an adaptation this control relies on; each such line is named
in the list above and changes no behaviour beyond the pre-fix decision.
