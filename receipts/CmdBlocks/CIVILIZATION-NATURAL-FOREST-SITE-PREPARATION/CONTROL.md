# CIVILIZATION-NATURAL-FOREST-SITE-PREPARATION — control on bbf5b71e5

Fix identity `patch:2b697b8843f47e2059d5a7ed52baa8c819ee8cf596646854ea2d51b2b2414a3a`
(`docs/bugs/promotions/CIVILIZATION-NATURAL-FOREST-SITE-PREPARATION.patch`, 2
hunks, one production file `crates/mc-core/src/settlements/construction_site_refusals.rs`).
The patch exempts natural forest from the construction-site refusal (the
`tree_clearable` exemption); reversing it restores the pre-fix refusal of a lot
holding a natural trunk.

- `git apply -R --check` passes at `bbf5b71e5`.
- RED: `.analysis/validation/20261128T092013-test-2rpjhn8p` FAIL, exit 100, the
  row's own `settlements::forestry::tests::natural_forest_lot_and_access_are_felled_into_storage_before_construction`
  panics at `forestry_tests.rs:214` (`assert_eq!(project.origin, expected.origin)`,
  left `[15, 200, 11]`): the funded site is not the felled forest lot.
- GREEN: `.analysis/validation/20261128T092429-test-07wzg06v` PASS (all 18
  registered `cargo:mc-core:settlements::forestry::tests::*` / villager refs run
  in the integration batches; this receipt covers the row's proving ref).
- Tree restored: no `.rs` diff after the control.
