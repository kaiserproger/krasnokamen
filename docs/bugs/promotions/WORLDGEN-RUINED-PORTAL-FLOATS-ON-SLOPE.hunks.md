# WORLDGEN-RUINED-PORTAL-FLOATS-ON-SLOPE: patch-mode fix identity

Original fix, two commits: `81a2b6bb8` (support column under a `Height::Surface` template that hangs over
air; `WORLDGEN_REVISION` 32) and `fee158398` (the column is filled with terrain material, never the
template's own block; `WORLDGEN_REVISION` 33). `fix_commit = "patch:f68a0a8f9e5b90a4363af71ee49751983a1cf167224a58a2553f7c19134b103c"`
names the HEAD form of both commits' non-test hunks in
`docs/bugs/promotions/WORLDGEN-RUINED-PORTAL-FLOATS-ON-SLOPE.patch`; `git apply -R` restores the pre-fix behaviour
(no support column, `WORLDGEN_REVISION` 31). The row's path was closed by more than one commit, so the patch
carries the non-test hunks of each. Re-audit:
`python3 -m tools.harness ledger promote WORLDGEN-RUINED-PORTAL-FLOATS-ON-SLOPE --fix-patch docs/bugs/promotions/WORLDGEN-RUINED-PORTAL-FLOATS-ON-SLOPE.patch --dry-run`.

Why patch mode: reversing `fee158398` and `81a2b6bb8` as commits no longer applies cleanly on HEAD, because
`2fadbef9a` (structure placement ring search) changed neighbouring lines of `overworld_structures.rs`. The
patch is HEAD's file against the same file with only the two fixes' behaviour taken out; `2fadbef9a` is not
part of it.

| commit | original hunk | in the fix patch |
|---|---|---|
| 81a2b6bb8, fee158398 | `lib.rs` `WORLDGEN_REVISION` 31 -> 32 -> 33 and its doc | yes (one hunk, 31 -> 33) |
| 81a2b6bb8 | `is_surface_family` | yes (removed on reversal) |
| 81a2b6bb8, fee158398 | `support_columns` body: footprint lowest cell, descent through the air gap, terrain material or stone fill | yes: the body is the behaviour |
| 81a2b6bb8 | call in `OverworldStructures::visit` before `paste_template_with_eyes` | yes |
| 81a2b6bb8 | struct field `air` and its initializer | no: (b) API the kept tests call (`overworld_structures_tests.rs:108`, `:145` build `OverworldStructures` with `air`) |
| fee158398 | fields `ground`, `filler` and their initializers | no: (b) same constructor calls (`:109`, `:110`, `:146`, ...) |
| 81a2b6bb8, fee158398 | `overworld_structures_tests.rs` | no: kept tests |

a/-side adaptation: kept tests call `structures.support_columns(&mut chunk, &template, origin)` directly
(`overworld_structures_tests.rs:877`, `:916`), so on reversal the function stays as the API with a no-op body
`let _ = (chunk, template, origin);`. It changes no behaviour of the pre-fix tree, which never placed a column.
The now-unused fields only produce dead-code warnings.

Proving ref: `overworld_structures::tests::floating_surface_structure_rests_on_a_support_column` goes red at
`crates/mc-worldgen/src/overworld_structures_tests.rs:781` (a footprint column floats above air: the row's
symptom). It reaches the fix through `structures.paste` -> `visit` -> `support_columns`, so removing the call and
the body reproduces the floating platform.

Not evidence: `overworld_structures::tests::ruined_portal_support_columns_never_copy_template_material` calls the kept
`support_columns` API directly (`overworld_structures_tests.rs:877`, `:916`) and goes red only because the
a/-side adaptation empties that body. It is a regression ref for the material rule, not a control of it.

Per-commit controls recorded when the fix was made (receipts, not re-run here): reversing the support-fill
hunk of `81a2b6bb8` fails the first ref (`receipts/XCoreBlocks/nc-support-column.log`); restoring the
last-visited-template-block material that `fee158398` removed fails the template-material check over the real
13 ruined-portal templates (`receipts/XPortalSupport/nc-template-material.log`). The control receipt of this
promote is in `docs/bugs/promotions/WORLDGEN-RUINED-PORTAL-FLOATS-ON-SLOPE.json`. The row is `repro = "source"`:
a worldgen state error reproducible by offline pregeneration, so a Cargo test through the real placement path
closes it without a client run.
