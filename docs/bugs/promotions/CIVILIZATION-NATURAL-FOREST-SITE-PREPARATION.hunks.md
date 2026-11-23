# CIVILIZATION-NATURAL-FOREST-SITE-PREPARATION — HEAD re-break control patch

Patch: `docs/bugs/promotions/CIVILIZATION-NATURAL-FOREST-SITE-PREPARATION.patch`
(`patch:2b697b8843f47e20…`), the row's `fix_commit`. It reverses on the clean
committed tree at `b67e1b0612` (`git apply -R --check`, plain context) and is
what a promotion passes as `--fix-patch`.

## What the fix is (origin `3d6822c28`)

A funded lot covered by natural forest could not be admitted: `site_free`
required every cell above the floor to be clearable/excavatable, and a tree is
neither, so all 185 seed-24 house lots were refused. The slice made a tree on
the lot (or its prepared walk) *clearable* through `forestry::Clearance`,
carried it in the prepared cell list, and felled it into storage during site
preparation, with worldgen evidence deciding what is natural.

## HEAD equivalent: the two admission hunks

At HEAD the admission lives in
`crates/mc-core/src/settlements/construction_site_refusals.rs` (`check`,
authored volume loop): a tree cell is allowed when
`clearance.clearable(state, at, Some(lot))` — or, for the volume, when it is
already generically clearable. The control removes that exemption again, so an
authored volume containing a natural trunk is refused exactly as before the
fix (the lot never starts; the pass moves to a clear lot).

## Omissions (one reason per HEAD equivalent of the origin's files)

- `block_log.rs` (worldgen evidence, player history) — **(b)**: the kept
  forestry tests call `block_log`/`player_history` and the evidence map is read
  by `forestry::vetoed`/`Clearance::verify`; deleting it stops the control from
  compiling.
- `settlements/forestry.rs` (the module), `settlements/mod.rs`,
  `settlements/registry.rs` — **(b)**: `forestry::tree`,
  `forestry::Clearance`, `footing_of`, `drops`/`roll` and the registry's
  clearance state are called by the kept tests (`forestry_tests.rs`,
  `settlement_builds_preparation_tests.rs`, `portions_tests.rs`) and by the
  lumber row's production path; removing them stops the control from
  compiling.
- `plugin_structures.rs`, `plugin_structures/settlement_builds.rs` (felling the
  cells, drops into storage, repair) — **(a)**: the same executor clearing
  serves CIVILIZATION-PREMATURE-LOG-DELIVERY and the street/verge rows whose
  registered tests drive `forestry::roll`/`drops`; reversing it turns their
  assertions red.
- `settlements/construction.rs` (`site_free`, `planned_cells` insertion of the
  prepared cells, `begin_cells` site bounds, `surface_y` tree skip) — **(c)**:
  later commits rewrote all of it (`site_refusals::check`, `executor::prepare`'s
  `site` parameter, the clearance-aware `surface_y_between`), so the origin's
  hunks have no HEAD equivalent; the included hunk above is the HEAD form of
  the admission rule they carried.
- `settlements/layout.rs` (frontage walk over verges and the forest forecast) —
  **(a)**: it is CIVILIZATION-STREET-VERGE's and the street-walk behaviour,
  whose own control and tests pin it.
- `villager/building.rs` (felling reach / `footing_of` use) — **(b)**:
  `footing_of` and the reach helpers are called by the kept
  `building_preparation_tests`, `settlement_builds_preparation_tests` and the
  lumber tests.
- `villager/lumber.rs` — **(a)**: CIV-SEED24-CROSS-VILLAGE-TREE-SWEEP's
  behaviour (near-first search), already READY with its own control.
- `docs/settlements/construction.md` — **(c)**: the prose was rewritten by
  later slices (D8 preparation/repair text).

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/CIVILIZATION-NATURAL-FOREST-SITE-PREPARATION.patch`
on the committed tree `b67e1b0612` succeeds.

Control run (reversed tree, then restored): `python3 -m tools.harness run test
-p mc-core --lib settlements::forestry::tests::natural_forest_lot_and_access_are_felled`
→ `.analysis/validation/20261123T201013-test-nqdwad60` (`status: failed`,
`dirty` tree at head `5ccc309d97`). The red is the row's own assertion
`crates/mc-core/src/settlements/forestry_tests.rs:214`
(`assert_eq!(project.origin, expected.origin)`: the forest lot is refused and a
different lot is chosen, so the funded site is not the felled forest lot). The
graphical `harness:buildings-qa:natural_forest_site` red is the integrated
sweep's control run, not claimed here.
