# KELP-ITEMS-SPAWN-ON-OCEAN-FLOOR — HEAD re-break control patch

Patch: `docs/bugs/promotions/KELP-ITEMS-SPAWN-ON-OCEAN-FLOOR.patch`
(`patch:f4c199c6856deb5d15d4b12c2d4feb007b9aed08b2054167972fac916145104d`),
the row's `fix_commit`. It reverses on the clean committed tree
(`git apply -R --check`, plain context) and is what a promotion passes as
`--fix-patch`.

## What the fix is (origin `af58aeb92`)

Two files:

- `crates/mc-core/src/fluids.rs`: `enter` decides `Flow::Wash` from the
  `washed_away_by_fluids` tag table (which excludes kelp/seagrass/waterloggable
  states) instead of the collision-shape test that washed every non-colliding
  plant.
- `crates/mc-core/src/registry.rs`: `RegistryTables` gains the dense
  `washed_by_fluids` table, built once at load from the block tag minus every
  `LiquidBlockContainer` and waterloggable state.

## HEAD equivalents and omissions

One hunk of the `enter` hunk-set is the HEAD equivalent: HEAD carries later
lava-bucket and spreading branches (other rows) which the re-break keeps, and the
reversal restores the pre-fix collision-shape decision at the same place.

The four `registry.rs` hunks (the field, its tag build, its per-state assignment and
its struct initialiser) are **reversed too** (the earlier control omitted the
field hunk with a wrong (b) reason — nothing but `fluids.rs` read the table, and
the reversed `enter` does not, so the field could not stop the control from
compiling). This patch removes the field, its tag build, its per-state assignment
and its struct initialiser, so the a/-side is the real pre-image: no
`washed_by_fluids` table exists and `enter` never asks for it. The tag build is
inside the `registry.rs` hunks; nothing else in the tree reads the table (checked:
`grep washed_by_fluids crates/` is exactly the field, its build and the `enter`
read).

## Omissions

- `fluid_plants_tests.rs`, `fluids_tests.rs` (the fix's regression tests) —
  **(b)**: the kept tests call the fixed decision and are the row's registered
  refs; reversing them would delete the control's red.
- Nothing else: the two source files are complete.

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/KELP-ITEMS-SPAWN-ON-OCEAN-FLOOR.patch`
on the committed tree at the promotion head succeeds (plain context, two files).

## Control runs (own reverted tree)

- Cargo control: `cargo:mc-core:fluids_tests::flowing_water_keeps_kelp_and_seagrass_without_loot`
  and `cargo:mc-core:fluids_tests::kelp_growing_in_still_water_drops_no_items` go
  red on the reversed tree (the collision-shape test washes kelp again). Receipts in
  the promotion JSON's `negative_control`.
- Graphical: `harness:scene-shot:native-kelp-ocean-floor` on the same reverted tree,
  with its own pin. The 002042 run was one shared reverse of three patches with a
  tree-identity mismatch (the pin names the dirty tree `5d1b9a9e` while the receipt
  records the clean commit) and is not this row's per-row proof; see the promotion
  JSON for this row's own run.

## a/-side audit (B5, 2026-11-24)

Script and full output: `receipts/D123/D123CivGates/a-side-audit.md`. Against the origins
`af58aeb92` and `b2b442ded` (the audit's origin column for this patch: `af58aeb92` for the
`enter`/tag-build lines the reversal rewrites, `b2b442ded` for the `registry.rs` table
itself), the `enter` hunk is the origin's own change at HEAD's merged function (the
origin changed `flow_into`, HEAD has one `enter`), and the flagged lines of the
reversed `registry.rs` hunk are HEAD's later multi-line tag build (`9710e4356`
rewrote the call as `tags\n.raw_ids(mc_data::tags::BLOCK_REGISTRY, …)`), i.e. the HEAD
form of the origin's own `let washed_away = tags.raw_ids(...)` — drift the reversal
must undo, not a difference in the table's content.
