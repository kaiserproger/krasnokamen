# CIVILIZATION-FUNDED-KIND-SITE-STARVATION — forward-ported control

[Control patch](../fix-patches/CIVILIZATION-FUNDED-KIND-SITE-STARVATION.patch) for `7c1a96e92a6de2680bc8efe45bffec6e6d6adf86`.
`fix_commit = "patch:081e849f57e8f4715ccbfcb0313ff741d4fe3d84071fb35aba9146dce2c9fca9"` names the canonical non-test patch. It replaces `patch:52f3b7a5c34f1c9772475a9240a6c42e69a0f14a59a0a268e95a3daa9155a793`, whose hunk no longer applies after the `site_project` call gained `sweeps`, `house.cursor`, `budget.take()` and `&images`.

## Provenance

- The fix content is the guard of `898af4725` (`fix(settlements): site search follows verges; working adults shear active sheep`): the funded prefix passed to `site_project` must be the whole funded run, not one cell.
- That commit did more than this row, so the reversal target has been its carved non-test hunk (previously recorded as `patch:52f3b7a5…`). The carved hunk never changed; only its anchor moved.

## Mapping

| Original (carved) hunk | Current equivalent at the candidate |
|---|---|
| `construction.rs` `@@ -1556,7 +1556,7 @@` (`.take(funded_count)`) | `@@ -1775,7 +1775,7 @@`, same one-line change at the `site_project` call (`.take(funded_count)` → pre-image `.take(1)`) |

## Exclusions

None: the patch is the complete carved hunk. Test and docs sections are not in the patch because the canonical splitter keeps them out, not because of a patch-mode exclusion.

## Proving assertion

- `cargo:mc-core:settlements::construction::site_tests::funded_mine_without_a_site_does_not_starve_a_legal_mason` — `crates/mc-core/src/settlements/construction_site_tests.rs:1296`. With `.take(1)` a funded but site-less mine consumes the pass and the legal mason starves; the historical control failed at the test's own step body (`site_tests.rs:1170` in the old layout). The other registered checks are unchanged.
