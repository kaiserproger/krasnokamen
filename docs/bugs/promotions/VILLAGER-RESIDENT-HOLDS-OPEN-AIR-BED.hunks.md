# VILLAGER-RESIDENT-HOLDS-OPEN-AIR-BED — HEAD re-break control patch

Patch: `docs/bugs/promotions/VILLAGER-RESIDENT-HOLDS-OPEN-AIR-BED.patch` (`patch:bf632d422b416bf00f6468002765454fa756cc291fc3a31381133f6ea7b29d14`), the row's
`fix_commit`. It reverses on the committed tree `6ef75a905` (`git apply -R
--check`, plain context) and is what a promotion passes as `--fix-patch`.

## What the fix is (origin `patch from fix-patches/VILLAGER-RESIDENT-HOLDS-OPEN-AIR-BED.patch`)

Files: crates/mc-core/src/settlements/allocator.rs.

The pre-existing fix patch did not reverse at HEAD with full context; it does at reduced context, so the HEAD patch was re-derived from the reversed tree with default context.

## HEAD equivalents and omissions

Contents are the same fix hunks; only the context lines are HEAD's.

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/VILLAGER-RESIDENT-HOLDS-OPEN-AIR-BED.patch` on `6ef75a905`
succeeds (clean tree); the real-client red is the later integrated sweep's
control run, not claimed here.

## a/-side audit (B5, 2026-11-24)

Mechanical check: `git show <origin>^:<file>` vs the file after `git apply -R`
of this patch at the audit head; script and full output in
`receipts/D123/D123CivGates/a-side-audit.md`. Every line outside the origin
commit's own change, with the commit that last changed it and whether the
line is in `origin^:file`:

- `crates/mc-core/src/settlements/allocator.rs` `@@ -1366,7 +1366,7 @@ fn acquire(` (adapted; origin `a09515875`, discovered origin):
  - a/-side `.retain(|(_, poi)| scope_allows(scope, poi.pos));` — 1f4996071 fix(villager): residents claim only roofed beds, leaving open-air beds to unbound villagers; in origin^: no

HEAD-side lines whose last change is another commit are that later commit's
form: reversing them is the HEAD drift a control must reverse to reach the
pre-fix decision (rule (c)), not part of the fix. An a/-side line that is not
in `origin^` is an adaptation this control relies on; each such line is named
in the list above and changes no behaviour beyond the pre-fix decision.
