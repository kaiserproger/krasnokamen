# MODEL-OPUS-9 — hunk map for the committed reversal pair

Promotion: `docs/bugs/promotions/MODEL-OPUS-9.json` (written by `ledger promote`).
Scene: `native-pufferfish` (`tools/harness/pufferfish_scene.json`), proving assertion
`pufferfish_layer_uv_digest`.

| half | client commit | content |
|---|---|---|
| control | `client:431e69ca` (`wt/D123PP3Opus9Ctl`) | the base commit with `35df28a0`'s eight production files reverted (`git revert 35df28a0`), the fix's test file kept so the pair diff is exactly the non-test patch |
| restore (= `fix_commit`) | `client:cdb7d514` (`wt/D123PP3Opus9`) | `ba1971d7` + the selection field `8eda5627` + the UV-digest commit `a5bd1f5e`; the fix itself is `35df28a0`, present in the base |

The base tree `cdb7d514` exists only to carry the two report fields the pair is read
through; it adds no behaviour of its own (the field reports what the renderer already
submitted, and the digest is computed from the submitted layer's vertices).

## Hunk map of `35df28a0`

| original hunk | HEAD equivalent | kept? |
|---|---|---|
| `assets/krasnokamen/models/entity/pufferfish_small.json` | base text unchanged (`git blame` names `35df28a0`) | **yes — this row's witness** |
| `assets/krasnokamen/models/entity/pufferfish_medium.json` | base text unchanged | **yes — this row's witness** |
| `assets/krasnokamen/models/entity/boat.json`, `boat_water_patch.json`, `chest_boat.json`, `chest_raft.json`, `raft.json` | base text unchanged | **yes — the same commit's boat hulls (the row aggregates the 26.3 exactness pass)** |
| `src/entities/presentation.rs` (the pufferfish state selection) | base text unchanged | **yes — this row's witness** |
| `src/entities/asset_model_regression_tests.rs` | test file | excluded: a test hunk (kept in both halves, so it is red on the control) |

Both halves carry the same report field code, so the only difference the pair measures is
the fix's production patch.
