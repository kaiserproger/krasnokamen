# CIV-SEED24-FURNACE-OUTPUT — HEAD re-break control patch

Patch: `docs/bugs/promotions/CIV-SEED24-FURNACE-OUTPUT.patch` (`patch:8cd11a6fb971a2fbe4eb9713b44ae01eb1bb40f6050abd32163406ad2783bc80`), the row's
`fix_commit`. It reverses on the committed tree `6ef75a905` (`git apply -R
--check`, plain context) and is what a promotion passes as `--fix-patch`.

## What the fix is (origin `1c6f7b09`)

Files: crates/mc-core/src/villager/smelting.rs.

`smelting::work` separates a *tender* (loads ore/fuel) from an employed resident that only empties owned furnace output: the profession gate excludes Builder/Soldier/Nitwit instead of allowing only None/Shepherd, and the output/fuel/demand gates are tender-scoped.

## HEAD equivalents and omissions

The commit's `settlements/allocator.rs` hunks are **omitted (a)**: they are CIV-SEED24-FORESTRY-STAFFING's staffing sort, another row's behaviour. HEAD carries the later Miner tender (4a457567): the re-break keeps `Miner` in the tender set so that row stays fixed; only the tender concept is reversed.

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/CIV-SEED24-FURNACE-OUTPUT.patch` on `6ef75a905`
succeeds (clean tree); the real-client red is the later integrated sweep's
control run, not claimed here.
