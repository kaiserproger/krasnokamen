# CLIENT-WITHER-SKULL-MATERIAL-SELECTION — hunk map for the committed reversal pair

Promotion: `docs/bugs/promotions/CLIENT-WITHER-SKULL-MATERIAL-SELECTION.json` (written by
`ledger promote`).
Scene: `native-wither-skull` (`tools/harness/wither_skull_scene.json`), proving assertion
`wither_skull_texture_selected`.

The real fix `f4bd26d4` predates the client rename (`f0a6abf6`, 2026-11-12), so its tree cannot
build a pre-fix client. The control is therefore the committed reversal on the current tip, and the
re-landing commit is the row's `fix_commit`:

| half | client commit | content |
|---|---|---|
| field (both sides) | `8eda5627` (`wt/D123PP3Wither`) | `6ab059c5`'s per-entity selection report, which the scene reads (`entity_extraction.entities`) |
| control | `client:a9d2aba4` | the real pre-image of the row's hunk: `presentation::material` has no wither-skull material branch (11 lines removed, the parenthesised `else if` arm only) |
| restore (= `fix_commit`) | `client:b9094755` | the re-landing; `git diff a9d2aba4 b9094755` is exactly this row's forward-ported fix (11 lines added) |

## Hunk map of `f4bd26d4` (the real commit)

| original hunk | HEAD equivalent | kept? |
|---|---|---|
| `presentation.rs` @@ -301,6 +301,13 @@ `material`: the `else if e.type_name == "minecraft:wither_skull"` arm selecting `WITHER_INVULNERABLE_LOCATION` / `WITHER_LOCATION` from `DATA_DANGEROUS` | the same arm at `presentation.rs:535` (`git blame -C -C` names `f4bd26d4`), text unchanged by later commits | **yes — this row's fix** |
| `presentation.rs` `special_item` bed item arm removed | n/a | (a) other rows' behaviour (bed item models) |
| `presentation.rs` remaining `special_item`/`material` hunks (decorated pot sides, held trident, chicken/cow variants, baby zombie armour wiring) | n/a | (a) other rows' behaviour |
| `assets/**/*.json` (armour shells, husk/zombie babies, bed/bell/trident/decorated-pot/cold-warm variants) | superceded by later rig rewrites (`da704d9b`, `b40b8d63`, ...) | (a) other rows' behaviour, (c) later commits rewrote the files |
| `src/entities/resources.rs`, `src/entities/render_blocks.rs`, `src/entities/render_blocks/village_tests.rs` | later commits rewrote those paths | (a) other rows' behaviour, (c) no HEAD equivalent |
| `src/entities/asset_model_regression_tests.rs`, `src/entities/solaris_models/tests.rs`, `docs/ENTITY_MODELS.md` | test and doc files | excluded: not production hunks |

The a/-side of the control is the real pre-image (the arm did not exist before `f4bd26d4`); nothing
is hand-invented. The `DATA_DANGEROUS` sub-branch itself needs a skull the server marks dangerous,
which no scene fixture can set yet, so it stays covered by
`entity_selection_coverage_wither_skull_material_tracks_received_dangerous_flag`; the scene proves
the *normal* skull's selected texture, which the pre-image could not resolve at all (the binding
fell through to the empty family default and the entity landed in `unrendered`).
