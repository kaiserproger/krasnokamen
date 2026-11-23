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
