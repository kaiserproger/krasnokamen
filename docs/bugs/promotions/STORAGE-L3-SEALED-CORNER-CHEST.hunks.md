# STORAGE-L3-SEALED-CORNER-CHEST — the one-line fix patch

Fix patch: `docs/bugs/fix-patches/STORAGE-L3-SEALED-CORNER-CHEST.patch`
(`patch:cc57c08cd8ee096658ff998a85cb5dadfd3ee6e9d53e72c2fcfe38f430fed1d3`),
the row's `fix_commit`; it applies in reverse on the candidate
`wt/D123Civ` at `f9a8d45596696880fc6a0c9d92f5cd1758002651`
(`git apply -R --check`).

## Completeness: every production hunk of the fix is in the patch

`50f1e9a0eb7df6cd85d0999a9261283442cccfec` "fix(settlements): keep every
level-three storage container reachable" changes exactly two files:

| 50f1e9a0e hunk | disposition |
|---|---|
| `crates/mc-core/assets/blueprints/storage.json` `@@ -46,7 +46,7 @@`: level-3 layer row 2 moves the barrel `b` off the corner chest `c`'s cell (`p.b..b.bp`/`pc.....bp` → `p....b.bp`/`pcb....bp`) | **included** — the whole production change |
| `crates/mc-core/src/settlements/blueprints_tests.rs` (65 test lines) | kept (test) |

There is no other production hunk, so the patch needs no omission list and no
a/-side adaptation.

`90e282cec0e16408f3dfe3aa585ffdc0850468fa` ("test(settlements): check storage
reach against each level's full picture", 1 file, 18 lines of
`blueprints_tests.rs`) is a later **test-only** commit on the stage2/edge
branch's top, not part of the fix: it reshapes the reachability test, and its
file is a test, so it is kept in the control tree.

## Proving refs

- `cargo:mc-core:settlements::blueprints::tests::every_storage_container_is_reachable_from_the_entrance`
  — red with the patch reversed (`promote.json`: `FAIL`), green on the restore.
  This is the row's proof.
- `cargo:mc-core:villager::building_autonomy_tests::adopted_village_with_resident_shears_produces_stock_and_builds`
  is **green in the control** (`docs/bugs/promotions/STORAGE-L3-SEALED-CORNER-CHEST.json`,
  `negative_control`): it is a guard on the adopted-village path, not evidence
  for this row, and it stays green on both trees.
