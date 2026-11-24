# VILLAGER-RESIDENT-HOLDS-OPEN-AIR-BED — HEAD re-break control patch

Patch: `docs/bugs/promotions/VILLAGER-RESIDENT-HOLDS-OPEN-AIR-BED.patch` (`patch:bf632d422b416bf00f6468002765454fa756cc291fc3a31381133f6ea7b29d14`), the row's
`fix_commit`. It reverses on the committed tree `6ef75a905` (`git apply -R
--check`, plain context) and is what a promotion passes as `--fix-patch`.

## What the fix is (origin: the fixed row's original patch file, now retired)

The original patch file `docs/bugs/fix-patches/VILLAGER-RESIDENT-HOLDS-OPEN-AIR-BED.patch`
was deleted on 2026-11-24 (review N4): its `patch:2c325bc0...` digest no longer matched the
row's `fix_commit`, and this promotions/ patch is the HEAD re-break the promotion uses. The
fix itself is the allocator `bed_allowed` change in `crates/mc-core/src/settlements/allocator.rs`
(row source: BreakIssue1x11 finding F4); no commit carries it, so the patch is the identity.

Files: crates/mc-core/src/settlements/allocator.rs.

The pre-existing fix patch did not reverse at HEAD with full context; it does at reduced context, so the HEAD patch was re-derived from the reversed tree with default context.

## HEAD equivalents and omissions

Contents are the same fix hunks; only the context lines are HEAD's.

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/VILLAGER-RESIDENT-HOLDS-OPEN-AIR-BED.patch` on `6ef75a905`
succeeds (clean tree); the real-client red is the later integrated sweep's
control run, not claimed here.
