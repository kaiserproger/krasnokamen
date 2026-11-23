# CIV-SEED24-FORESTRY-STAFFING — HEAD re-break control patch

Patch: `docs/bugs/promotions/CIV-SEED24-FORESTRY-STAFFING.patch` (`patch:b17555791e307a2042da9449ea95357b55ddda0b40422bfa20c4af0e7f8c49e4`), the row's
`fix_commit`. It reverses on the committed tree `6ef75a905` (`git apply -R
--check`, plain context) and is what a promotion passes as `--fix-patch`.

## What the fix is (origin `1c6f7b09`)

Files: crates/mc-core/src/settlements/allocator.rs.

`acquire` keeps staffing certified resource slots after Storage is built: `needs_resources` (only profression None before Storage) and the boolean resource sort are replaced by the unconditional priority ladder Lumberjack>Miner>others.

## HEAD equivalents and omissions

The commit's `villager/smelting.rs` hunks are **omitted (a)**: they are CIV-SEED24-FURNACE-OUTPUT's tender gating, another row's behaviour.

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/CIV-SEED24-FORESTRY-STAFFING.patch` on `6ef75a905`
succeeds (clean tree); the real-client red is the later integrated sweep's
control run, not claimed here.
