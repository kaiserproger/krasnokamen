# CIV-SEED24-FARM-SEEDS-NEED-A-FARM — HEAD re-break control patch

Patch: `docs/bugs/promotions/CIV-SEED24-FARM-SEEDS-NEED-A-FARM.patch` (`patch:4932f851798c5f3bda3ad5bbbff94fbbfbe58feff741f1ce0eb85a33f63f1245`), the row's
`fix_commit`. It reverses on the committed tree `6ef75a905` (`git apply -R
--check`, plain context) and is what a promotion passes as `--fix-patch`.

## What the fix is (origin `fb4c1a52`)

Files: crates/mc-core/src/plugin_structures/settlement_builds.rs.

`debit` credits vanilla plant loot from the cleared cells (`forestry::plant_drops`) into storage, spilling what does not fit at the cell, so clearing short grass can seed the first Farm.

## HEAD equivalents and omissions

The commit's `forestry.rs` hunks are **omitted (b)**: the cache thread-join is SETTLEMENT-FOREST-WORKER-DETACHED's behaviour (a different row) and `plant_drops`/`roll` are API the kept forestry tests call; the control keeps them. The row's own negative control reverted only these settlement_builds.rs debit hunks.

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/CIV-SEED24-FARM-SEEDS-NEED-A-FARM.patch` on `6ef75a905`
succeeds (clean tree); the real-client red is the later integrated sweep's
control run, not claimed here.

## a/-side audit (B5, 2026-11-24)

Mechanical check: `git show <origin>^:<file>` vs the file after `git apply -R`
of this patch at the audit head; script and full output in
`receipts/D123/D123CivGates/a-side-audit.md`. Every line outside the origin
commit's own change, with the commit that last changed it and whether the
line is in `origin^:file`:

- `crates/mc-core/src/plugin_structures/settlement_builds.rs` `@@ -944,6 +944,11 @@ pub(super) fn debit(` (adapted; origin `89cb1fb61`, discovered origin):
  - HEAD `crate::settlements::forestry::plant_drops(state, cell.at, block)?` — fb4c1a522 settlements: cleared plants yield vanilla loot so the first farm can be seeded (CIV-SEED24-FARM-SEEDS-NEED-A-FARM); join the forest worker on drop (SETTLEMENT-FOREST-WORKER-DETACHED-HOLDS-STORE); in origin^: yes

HEAD-side lines whose last change is another commit are that later commit's
form: reversing them is the HEAD drift a control must reverse to reach the
pre-fix decision (rule (c)), not part of the fix. An a/-side line that is not
in `origin^` is an adaptation this control relies on; each such line is named
in the list above and changes no behaviour beyond the pre-fix decision.
