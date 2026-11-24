# MODEL-OPUS-3 — hunk map for the tip reversal pair

Promotion receipt: `promotions/MODEL-OPUS-3.json` (written by `ledger promote`).
Client pair: reversal `client:50a177c0` → re-land `client:4d98dcdd` (the row's `fix_commit`).
Both pins clean (`50a177c07bde`, `4d98dcddb16a`); the pair's net tree is client `main` `530556e8`.

## Why the pair exists, and the commits that closed the path

The row is `fixed-unpromoted` behind `client:e8fe40c2` ("hand-drawn rigs and missing entity visuals",
2026-11-05). That commit introduced the `animal_overlay` table — `("minecraft:pig", 7) =>
("minecraft:pig_saddle_equipment#main", "pig_saddle")` plus fourteen other species — and replaced
the pre-fix inline `horse`/`wolf` selection that never resolved any other species' mount slot.
`client:83994480` (2026-11-09) added a refinement on the same path: a baby wolf/llama wears
`Worn::Omitted` instead of the "baby animal equipment overlay is not available" error. Both commits'
hunks for this path are carried; every other hunk of either commit is listed below with its reason.

The server already sends the pig's saddle: `crates/mc-core/src/command_summon.rs:587-592` accepts
`equipment:{saddle:{…}}` for `minecraft:pig`, `entity_loot.rs:302,308-318` stores it as slot 7, and
`mc-edge/src/plan/entity_appearance.rs:177-199,405-416` forwards it as `EquipmentSlot::Saddle`. The
row was scene-gated, not server-gated (review B2 corrected this lane's earlier claim).

## Hunks carried by the pair

| commit | file | original hunk | content | disposition |
|---|---|---|---|---|
| `e8fe40c2` | `src/entities/equipment.rs` | `@@ -91,22 +164,21 @@ impl Cache` (the `resolve` selection) | `let horse = …; let wolf = …; let animal = horse \|\| wolf;` → `let overlay = animal_overlay(&e.type_name, slot); let animal = overlay.is_some();` and the `if wolf { … } else if horse { match slot { … } }` selection → `if let Some((key, kind)) = overlay` | REVERSED — the control restores the pre-fix horse/wolf selection from `e8fe40c2^`; the re-land restores HEAD's table lookup |
| `e8fe40c2` | `src/entities/equipment.rs` | `@@ -284,6 +357,33 @@ fn armor_rig` | adds `fn animal_overlay(species, slot)` with the fifteen-species table | REVERSED — deleted in the control, restored by the re-land; it has no other caller |
| `83994480` | `src/entities/equipment.rs` | `@@ -174,6 +174,10 @@ impl Cache` | `if baby && overlay.is_some_and(\|(_, kind)\| matches!(kind, "wolf_body" \| "llama_body")) { return Ok(true) }` (HEAD: `Ok(Worn::Omitted)`) | REVERSED — deleted in the control together with the table it reads; the pre-fix tree returned the baby error for a wolf, which is what the control does |

## a/-side adaptation

The control keeps HEAD's function shape and every later addition around the selection: the
`Worn` enum, the player-back hook, `battle_armor_rig` and `small_stand`/`small_stand_rig` in the
`else` branch (added by `20a12ac2f` after the fix) are untouched, because reverting them would be a
different commit's reversal. Only the animal-selection lines change, and every one of them is
`e8fe40c2^`'s own text. No stub, no flag, no early return: for a pig's slot 7 the control falls
through to the humanoid/battle lookup exactly as the pre-fix tree did, so the saddle resolves no rig
and is not drawn — the row's symptom.

The two tests that assert the fix (`received_animal_equipment_draws_and_strips_every_supported_species`
and `received_animal_body_equipment_follows_supported_adult_baby_transition`) are test hunks of the
same commits; the control deletes them so its suite is coherent (they name `animal_overlay` and the
baby omission), and the re-land restores them. `cargo test -p krasnokamen-client --lib
entities::equipment` is green on the control (17 passed) and on the candidate.

## Every omitted hunk

| commit | hunk | disposition |
|---|---|---|
| `e8fe40c2` | `@@ -22,6 +22,8 @@ struct Asset` / `@@ -31,6 +33,7 @@ struct Layer` (`use_player_texture`), `@@ -244,6 +316,7 @@ fn load_asset` | (a)/(b): the player-back/cape/elytra layers' field. Reversing it would red the elytra/cape rows and the kept `player_back_*` ref |
| `e8fe40c2` | `@@ -78,6 +81,76 @@ impl Cache` (`wings_asset`, `append_player_back`) | (a) MODEL-OPUS-CAPES / elytra behaviour; kept |
| `e8fe40c2` | its test hunks at `@@ -296,6 +396,7 @@`, `@@ -430,6 +531,7 @@`, `@@ -438,6 +540,8 @@`, `@@ -820,6 +924,7 @@`, `@@ -918,11 +1023,13 @@`, `@@ -932,6 +1039,7 @@`, `@@ -1023,6 +1131,7 @@`, `@@ -1061,6 +1170,7 @@` | test hunks: the equipment fixtures/literals. The two animal tests are reversed with the fix (above); the others cover the kept layers and stay |
| `e8fe40c2` | every other file of the commit (rigs, `render_living`, `world_fx`, `ui/item_bars`, model JSONs, …) | (a) other rows' behaviour; `CM1-REVERSIBILITY.md` records per file whether the hunk still reverses |
| `83994480` | `@@ -111,7 +111,7 @@` and `@@ -210,7 +214,7 @@ impl Cache` (`p::evaluate_rig(…, content, …)`) | (b): the `content` argument every worn layer passes; kept |
| `83994480` | the remaining equipment test hunks (`@@ -644,8 +648,8 @@`, `@@ -687,11 +691,11 @@`, `@@ -708,7 +712,7 @@`) | test expectations for the kept layers; kept (the control's green suite includes them) |

## Proving ref

`harness:scene-shot:native-pig-saddle` → `pig_saddle_drawn`
(`tools/harness/backends/scene_shot_checks.py::check_pig_saddle`). One NoAI pig is summoned at
`1000.5 160 1004.5`, captured from `(1000.5, 161.62, 1002.0)` yaw 0 pitch 20, then killed and
re-summoned at the same place with `equipment:{saddle:{id:"minecraft:saddle"}}`. Both frames must
show the pig (pink pixels > 1500; measured 32901/32967 on the control and 32818/31963 on the re-land),
and the saddle's leather — the only brown in the region, since the stone is grey and the sky blue —
must go from under 100 to over 300 pixels. Measured on the promotion receipts: control **0 → 0**
leather pixels (the shading difference between the two summons moved 138 pixels but none of them
brown), re-land **0 → 3810** leather pixels (4066 changed).

The unit ref `client:krasnokamen-client:entities::equipment::tests::wearables::received_animal_equipment_draws_and_strips_every_supported_species`
covers the table's fifteen species on the candidate.
