# MODEL-OPUS-9 — hunk map for the committed reversal pair

Promotion: `docs/bugs/promotions/MODEL-OPUS-9.json` (written by `ledger promote`).
Scene: `native-pufferfish` (`tools/harness/pufferfish_scene.json`), proving assertion
`pufferfish_layer_uv_digest`.

| half | client commit | content |
|---|---|---|
| control | `client:53b58c66` (`wt/D123PP3Opus9Ctl2`) | the two pufferfish lookups at their `35df28a0` pre-image, byte-identical (`git show 35df28a0^:<path>`); unchanged since that commit |
| restore (= `fix_commit`) | `client:bb33b3f1` (`wt/D123PP3Opus9`) | the same two lookups re-landed at the tip on top of the report fields (`cdb7d514` = `ba1971d7` + `8eda5627` + `a5bd1f5e`) |

`53b58c66` is the parent commit of `bb33b3f1` on the same branch, so the pair diff is
exactly the re-landing's two-file patch, which is byte-for-byte `35df28a0`'s hunks on
those two paths. The boat hulls and the `boat_root` anchor of `35df28a0` live in the
base tree on both halves (their presentation.rs hunk no longer reverses at the tip, so
they cannot be part of the pair) and the row's `repro_check` records them.

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
