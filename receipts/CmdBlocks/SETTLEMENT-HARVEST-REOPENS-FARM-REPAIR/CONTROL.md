# SETTLEMENT-HARVEST-REOPENS-FARM-REPAIR — control on bbf5b71e5

Fix identity `patch:f0db3e3b2c2cf65a8499ec65842dc9a9b3a9e6057ec2c11f35c00a42b80b87cf`
(`docs/bugs/fix-patches/SETTLEMENT-HARVEST-REOPENS-FARM-REPAIR.patch`, 2 hunks,
one production file). HEAD equivalent of `dde9a9e72b` because that commit no
longer reverses at HEAD: `983178bde` rewrote the inherited-cell fold and the
call sites were reformatted.

- Reversed only the two call-site clauses in `recover` and `reopen_emptied`.
  `harvestable_crop` stays in the control tree: `983178bde`'s inherited fold
  calls it (class b, kept API).
- `git apply -R --check` passes at `bbf5b71e5`.
- RED: `.analysis/validation/20261128T085243-test-5m5d7c8q` FAIL, exit 100, the
  row's own test panics in `complete()`:
  `construction did not finish: ["farm · level 1 · 99%", … "Missing materials:",
  "minecraft:wheat_seeds: 1" …]` — exactly the row's stalled repair.
- GREEN: `.analysis/validation/20261128T092429-test-07wzg06v` PASS for
  `settlements::construction::fencing_tests::a_harvested_farm_crop_does_not_reopen_a_repair_stage`.
- Tree restored: no `.rs` diff after the control.
