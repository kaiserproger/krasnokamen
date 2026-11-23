# CIV-SEED24-HOUSE-BED-IRON-NEVER-SMELTED — HEAD re-break control patch

Patch: `docs/bugs/promotions/CIV-SEED24-HOUSE-BED-IRON-NEVER-SMELTED.patch` (`patch:113c2e042ed5a6f8f35ea038c69f31447b3f19e68a625d24cd65b20743080c79`), the row's
`fix_commit`. It reverses on the committed tree `6ef75a905` (`git apply -R
--check`, plain context) and is what a promotion passes as `--fix-patch`.

## What the fix is (origin `4a457567`)

Files: crates/mc-core/src/settlements/construction.rs + villager/smelting.rs.

`construction::iron_demand` no longer lets an active project's bill replace the autonomous candidates' bills (max, not early return), so iron for an unfunded priority bed still drives smelting; `smelting::work` counts a Miner as a tender, so a town whose adults all hold jobs still smelts construction iron.

## HEAD equivalents and omissions

HEAD drifted: the single `record.project` became `access::projects` (a D8 multi-project change, other rows). The HEAD equivalent of the pre-image early return is `if access::projects(record).next().is_some() { return active; }` before the candidate fold; the active project's own bill is still summed by the loop above it. The Miner tender addition is reversed in `smelting.rs` to the 1c6f7b09 set (None|Shepherd), which keeps that row's tender framing.

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/CIV-SEED24-HOUSE-BED-IRON-NEVER-SMELTED.patch` on `6ef75a905`
succeeds (clean tree); the real-client red is the later integrated sweep's
control run, not claimed here.
