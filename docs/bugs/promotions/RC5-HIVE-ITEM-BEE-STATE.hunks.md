# RC5-HIVE-ITEM-BEE-STATE: patch-mode fix identity

Original fix, two commits on `crates/mc-world-conversion/src/pre_rename.rs`: `259a078ef` (typed route
`(ItemComponents, "minecraft:bees") => Bees` and the item-stack owners, so the legacy bee components inside
typed item stacks are migrated) and `602c20cfd` (the private `krasnokamen:HiveEntity` JSON image, whose scalar
`rider` becomes `riders`; player `krasnokamen:Carried`, `krasnokamen:CraftingGrid` and `equipment`).
`fix_commit = "patch:f2b02f99a0411b887329667c47078408bce9d513fd36875ecabeab709a7befae"` names the HEAD form of
both commits' non-test hunks in `docs/bugs/promotions/RC5-HIVE-ITEM-BEE-STATE.patch`; `git apply -R` restores the
pre-fix migration. The row's path was closed by two commits, so the patch carries the hunks of each. Re-audit:
`python3 -m tools.harness ledger promote RC5-HIVE-ITEM-BEE-STATE --fix-patch docs/bugs/promotions/RC5-HIVE-ITEM-BEE-STATE.patch --dry-run`.

Why patch mode: the row names two commits, and a later commit (`a89cb107c`, scoping legacy renames to owning
records) touched the same file, so the commits are reversed together on HEAD instead of one by one.

| commit | original hunk | in the fix patch |
|---|---|---|
| 259a078ef | module doc, `Owner::{ItemStack, ItemComponents, ItemContainer, Equipment}` | yes |
| 259a078ef | `Owner::child` item-stack, equipment, `components`, `minecraft:bees`, `minecraft:container`, bundle and projectile routes | yes |
| 602c20cfd | module doc, `Owner::HiveImage` | yes |
| 602c20cfd | player `krasnokamen:Carried`, `krasnokamen:CraftingGrid`, `(Player \| Entity, "equipment")` routes, `(Entity, "krasnokamen:HiveEntity") => HiveImage` | yes |
| 602c20cfd | `Tag::String` arm for `HiveImage` and `migrate_hive_image` | yes |
| 259a078ef, 602c20cfd | `pre_rename_bee_tests.rs`, `pre_rename_tests.rs`, `fixtures/rc5-bee-hive.json`, `pre_rename_unscoped_control.patch` | no: kept test files and fixtures |
| both | `docs/MIGRATION.md` | no: docs |

Every original non-test hunk is carried; none is left out and no a/-side adaptation was needed (the kept tests
call no fix-only API).

Controls: the promote control reverses this patch on HEAD (`control_tree: head`) and runs all five refs; the
receipt is `docs/bugs/promotions/RC5-HIVE-ITEM-BEE-STATE.json`. The earlier single-commit control of `602c20cfd`
turned all five refs red; the `259a078ef` route is now covered in the same reversal.

Remaining gap, tracked separately: the fixture is a synthetic RC5 bee image (captured with RC5 code and embedded
in a current player export), not a real RC5 save. The import of an actual RC5 world is `RC5-HIVE-REAL-WORLD-IMPORT`.
