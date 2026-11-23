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
