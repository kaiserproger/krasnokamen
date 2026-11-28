# MODEL-OPUS-N3 — historical fix hunk map

Status: open. The local reversal pair has a real graphical red/green, but its restore `client:bc7429a36c29d09429a0c84bd66cf0ba0e6b983e` is not in published client main. The published original `client:e8fe40c2` cannot be pinned by the current client pin tool because that tree still names the `solaris-client` package. This map is not a promotion receipt.

The pair is control `client:035c0dadf4e122723d3624fb1416af3dae0f44db` and restore `client:bc7429a36c29d09429a0c84bd66cf0ba0e6b983e`, based on client `529ed2dfe4c1ec9cbcf5f43b7489f09625dc5d5a`. The preimage adaptation is solely the rename from `assets/solaris/models/entity/player_head.json` to `assets/krasnokamen/models/entity/player_head.json`: the control restores the historical file bytes verbatim; the restore re-lands the current fixed bytes. The rest of the bulk commit remains present.

The following lists every text hunk in the original bulk commit, including test and documentation hunks for completeness. Omission reason (a) means the unrelated behaviour is retained in both pair trees; it is not removed merely to reproduce this row. The profile lookup and spawner context are retained separately from the authored hat geometry.

| Original file | Original hunk header | Pair disposition |
|---|---|---|
| `assets/solaris/models/entity/allay.json` | `@@ -1,345 +1,50 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/axolotl.json` | `@@ -8,49 +8,70 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/axolotl_baby.json` | `@@ -8,49 +8,70 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/bee_baby.json` | `@@ -3,19 +3,77 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/camel_husk_saddle_equipment.json` | `@@ -0,0 +1,30 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/camel_saddle_equipment.json` | `@@ -0,0 +1,30 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/cape.json` | `@@ -0,0 +1,18 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/cod.json` | `@@ -4,27 +4,39 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/creeper_charge.json` | `@@ -0,0 +1,48 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/donkey_saddle_equipment.json` | `@@ -0,0 +1,24 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/elder_guardian.json` | `@@ -4,103 +4,105 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/elytra.json` | `@@ -0,0 +1,24 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/fox.json` | `@@ -1,328 +1,61 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/guardian.json` | `@@ -4,103 +4,105 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/happy_ghast_baby.json` | `@@ -4,57 +4,64 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/happy_ghast_body_equipment.json` | `@@ -0,0 +1,14 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/hoglin.json` | `@@ -1,571 +1,85 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/hoglin_baby.json` | `@@ -1,571 +1,85 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/llama_body_equipment.json` | `@@ -0,0 +1,30 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/mule_saddle_equipment.json` | `@@ -0,0 +1,24 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/nautilus_body_equipment.json` | `@@ -0,0 +1,13 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/nautilus_saddle_equipment.json` | `@@ -0,0 +1,15 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/panda.json` | `@@ -1,664 +1,57 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/panda_baby.json` | `@@ -1,702 +1,57 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/pig.json` | `@@ -1,204 +1,48 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/pig_baby.json` | `@@ -1,197 +1,48 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/pig_saddle_equipment.json` | `@@ -0,0 +1,15 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/player_head.json` | `@@ -1,12 +1,55 @@` | CARRIED: exact old blob at `assets/krasnokamen/models/entity/player_head.json`; restore is the fixed blob |
| `assets/solaris/models/entity/rabbit.json` | `@@ -1,389 +1,69 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/rabbit_baby.json` | `@@ -1,389 +1,69 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/salmon.json` | `@@ -4,33 +4,45 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/skeleton_horse_saddle_equipment.json` | `@@ -0,0 +1,24 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/strider.json` | `@@ -1,409 +1,166 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/strider_baby.json` | `@@ -1,345 +1,62 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/strider_saddle_equipment.json` | `@@ -0,0 +1,21 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/vex.json` | `@@ -1,638 +1,50 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/vindicator.json` | `@@ -1,399 +1,61 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/zoglin.json` | `@@ -1,571 +1,85 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/zoglin_baby.json` | `@@ -1,571 +1,85 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `assets/solaris/models/entity/zombie_horse_saddle_equipment.json` | `@@ -0,0 +1,24 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `docs/ENTITY_MODELS.md` | `@@ -40,6 +40,14 @@ explicitly unrendered. The candidate's native equip/strip replay and movement re` | Documentation; preserved |
| `docs/ENTITY_MODELS.md` | `@@ -94,8 +102,18 @@ Enchanting tables render their animated book and conduits render their inactive` | Documentation; preserved |
| `docs/ENTITY_MODELS.md` | `@@ -125,7 +143,8 @@ unrendered until the scene connects profile resolution and atlas publication.` | Documentation; preserved |
| `docs/ENTITY_MODELS.md` | `@@ -230,11 +249,9 @@ a fixed box). Placeholders are tinted in `presentation::emit_rig`.` | Documentation; preserved |
| `docs/ENTITY_MODELS.md` | `@@ -259,7 +276,14 @@ a fixed box). Placeholders are tinted in `presentation::emit_rig`.` | Documentation; preserved |
| `docs/ENTITY_MODELS.md` | `@@ -308,10 +332,9 @@ a fixed box). Placeholders are tinted in `presentation::emit_rig`.` | Documentation; preserved |
| `src/entities.rs` | `@@ -4,6 +4,7 @@ use crate::protocol::packet_ids::play::clientbound as cb;` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/animation_tests.rs` | `@@ -231,6 +231,156 @@ fn animation_zombie_authored_legs_follow_received_motion() {` | Test-only; preserved |
| `src/entities/creeper.rs` | `@@ -0,0 +1,75 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/equipment.rs` | `@@ -22,6 +22,8 @@ struct Asset {` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/equipment.rs` | `@@ -31,6 +33,7 @@ struct Layer {` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/equipment.rs` | `@@ -78,6 +81,76 @@ impl Cache {` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/equipment.rs` | `@@ -91,22 +164,21 @@ impl Cache {` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/equipment.rs` | `@@ -244,6 +316,7 @@ fn load_asset(content: &Content, asset: &str, kind: &str) -> Result<Option<Vec<L` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/equipment.rs` | `@@ -284,6 +357,33 @@ fn armor_rig(slot: usize) -> Option<&'static str> {` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/equipment.rs` | `@@ -296,6 +396,7 @@ mod tests {` | Test-only; preserved |
| `src/entities/equipment.rs` | `@@ -430,6 +531,7 @@ mod tests {` | Test-only; preserved |
| `src/entities/equipment.rs` | `@@ -438,6 +540,8 @@ mod tests {` | Test-only; preserved |
| `src/entities/equipment.rs` | `@@ -820,6 +924,7 @@ mod tests {` | Test-only; preserved |
| `src/entities/equipment.rs` | `@@ -918,11 +1023,13 @@ mod tests {` | Test-only; preserved |
| `src/entities/equipment.rs` | `@@ -932,6 +1039,7 @@ mod tests {` | Test-only; preserved |
| `src/entities/equipment.rs` | `@@ -1023,6 +1131,7 @@ mod tests {` | Test-only; preserved |
| `src/entities/equipment.rs` | `@@ -1061,6 +1170,7 @@ mod tests {` | Test-only; preserved |
| `src/entities/equipment_tests.rs` | `@@ -0,0 +1,289 @@` | Test-only; preserved |
| `src/entities/presentation.rs` | `@@ -418,6 +418,7 @@ pub(super) struct DrawStore<'a> {` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/presentation.rs` | `@@ -651,6 +652,7 @@ pub fn append(` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/presentation.rs` | `@@ -822,6 +824,10 @@ pub(super) fn draw_entity(` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/presentation.rs` | `@@ -947,18 +953,28 @@ pub(super) fn draw_entity(` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/presentation.rs` | `@@ -1047,9 +1063,20 @@ pub(super) fn draw_entity(` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/presentation.rs` | `@@ -1116,6 +1143,12 @@ pub(super) fn evaluate_rig(` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/presentation.rs` | `@@ -1156,12 +1189,8 @@ pub(super) fn evaluate_rig(` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/presentation.rs` | `@@ -1288,22 +1317,29 @@ pub(super) fn emit_rig(` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/presentation.rs` | `@@ -2341,7 +2377,6 @@ mod tests {` | Test-only; preserved |
| `src/entities/presentation.rs` | `@@ -2810,8 +2845,11 @@ mod tests {` | Test-only; preserved |
| `src/entities/profiles.rs` | `@@ -168,6 +168,35 @@ fn decode(r: &mut Reader<'_>) -> Result<Vec<Update>> {` | (a) Kept: profile lookup or spawner context, separate from hat geometry |
| `src/entities/render_blocks.rs` | `@@ -270,6 +270,7 @@ impl super::Store {` | (a) Kept: profile lookup or spawner context, separate from hat geometry |
| `src/entities/render_blocks.rs` | `@@ -430,17 +431,20 @@ impl super::Store {` | (a) Kept: profile lookup or spawner context, separate from hat geometry |
| `src/entities/render_blocks/village_tests.rs` | `@@ -723,6 +723,56 @@ fn static_skulls_render_and_report_unresolved_player_profiles() {` | Test-only; preserved |
| `src/entities/render_living.rs` | `@@ -4,4 +4,4 @@ mod motion;` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/render_living/state.rs` | `@@ -278,6 +278,10 @@ pub(crate) struct State {` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/render_living/state.rs` | `@@ -308,6 +312,10 @@ impl State {` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/render_living/state.rs` | `@@ -316,6 +324,37 @@ impl State {` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/render_living/state.rs` | `@@ -465,6 +504,9 @@ impl State {` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/render_living/state.rs` | `@@ -813,6 +855,14 @@ impl State {` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/render_special.rs` | `@@ -6,6 +6,7 @@ mod kernels;` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/render_special.rs` | `@@ -54,6 +55,7 @@ pub(crate) struct Cache {` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/render_special.rs` | `@@ -146,13 +148,26 @@ pub(crate) fn append(` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/render_special.rs` | `@@ -163,6 +178,10 @@ pub(crate) fn append(` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/render_special/kernels.rs` | `@@ -92,69 +92,3 @@ pub(super) fn cluster_offsets(seed: i32, count: i32, depth: f32) -> [Vec3; 5] {` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/render_special/timeline.rs` | `@@ -206,7 +206,7 @@ impl State {` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/render_special/timeline.rs` | `@@ -217,6 +217,9 @@ impl State {` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/render_special/world_fx.rs` | `@@ -0,0 +1,391 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/render_special/world_fx_tests.rs` | `@@ -0,0 +1,323 @@` | Test-only; preserved |
| `src/entities/solaris_models/tests.rs` | `@@ -470,3 +470,125 @@ fn baked_limbs_sample_visible_texels_of_the_default_skin() {` | Test-only; preserved |
| `src/entities/source_state.rs` | `@@ -263,6 +263,35 @@ fn water_contact(` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/entities/source_state.rs` | `@@ -401,6 +430,11 @@ pub(crate) fn tick(` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/ui/item_bar_tests.rs` | `@@ -0,0 +1,441 @@` | Test-only; preserved |
| `src/ui/item_bars.rs` | `@@ -0,0 +1,137 @@` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/ui/state.rs` | `@@ -20,6 +20,9 @@ use std::{` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/ui/state.rs` | `@@ -4144,7 +4147,7 @@ fn append_stack_item(` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `src/ui/state.rs` | `@@ -4671,3 +4674,7 @@ mod banner_item_order_tests {` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |
| `tools/entity_models.py` | `@@ -326,6 +326,9 @@ def preview(path, out, texture_override=None):` | (a) Kept: other model, equipment, animation, rendering or UI behaviour outside the isolated hat-model change |

The shared graphical assertion is `harness:scene-shot:native-player-head` → `placed_head_hat_layer_pixels`. The control/restore results and their scope are recorded in `receipts/D145ClientTail/MODEL-OPUS-N3/evidence.md`; neither a published restore nor promotion is claimed.
