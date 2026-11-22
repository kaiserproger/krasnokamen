# CIVILIZATION-MIXED-WOOD-FUNDING-STALL — forward-ported control

[Control patch](../fix-patches/CIVILIZATION-MIXED-WOOD-FUNDING-STALL.patch) for `7c1a96e92a6de2680bc8efe45bffec6e6d6adf86`.
`fix_commit = "patch:1535c479db3ca4140a8f25b72dfc66bd7245c66f05d04f814d9bcbb4407182ee"` names the canonical non-test patch. It replaces `patch:1d0f3fb02785d3fa6dae0487338ad0fbbe1da62d66cbdc9cf4c9548f9275487f`, whose hunk no longer applies after the funding loop moved and gained the post-haul reservation block below it.

## Provenance

- The fix content is the guard of `e2a617447` (`fix(settlements): preserve whole-bill mixed-wood construction funding`): a mixed-wood bill must reserve against the whole future bill instead of `Default::default()`, and halve on failure.
- That commit changed `settlement_builds.rs` and `construction.rs` broadly, so the reversal target has been its carved non-test hunk (previously recorded as `patch:1d0f3fb0…`). The carved content is unchanged; only its anchor moved.

## Mapping

| Original (carved) hunk | Current equivalent at the candidate |
|---|---|
| `settlement_builds.rs` `@@ -1003,7 +1003,18 @@` (`let reserved = Default::default();` → the `wood_specific`/`reserve_wood` block) | `@@ -1648,7 +1648,18 @@`, same block at the funding loop; pre-image restores `let reserved = Default::default();` |

## Exclusions

None: the patch is the complete carved hunk. Test and docs sections are excluded by the canonical splitter.

## Proving assertion

- `cargo:mc-core:settlements::construction::tests::admitted_birch_camp_can_finish_from_the_same_mixed_stock`. The admitted camp on `e55305c50e33` stalls at 66/183 with 34 oak logs still in stock when only this fix is reversed. The completion assertion at `crates/mc-core/src/settlements/construction_tests.rs:105` fails with `remaining stock: …`; the same ref passes on the restored candidate.
