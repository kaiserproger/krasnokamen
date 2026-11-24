# MODEL-PLACED-POT-DECORATIONS — hunk map for the tip reversal pair

Promotion receipt: `promotions/MODEL-PLACED-POT-DECORATIONS.json`.
Client pair: reversal `client:3faa03bb6c` → re-land `client:57d14c6316` (the row's `fix_commit`).
Both pins clean (`3faa03bb6c98`, `57d14c63167c`); the pair's net tree is client `main` `530556e8`.

## Original commit

`client:6e541f96` — "fix(entities): decorated pot sides read bare item ids as well as {id}
records", 2026-11-11, 3 files. 26.3 `PotDecorations` is a record of optional
`back`/`left`/`right`/`front` ItemStackTemplates whose CODEC also accepts a bare item id; the
server forwards the placing item's component verbatim, so a pot placed from a bare-form item
reaches the client as bare strings, and the item renderer meanwhile read a four-element array that
26.3 never sends. Both renderers now share `pot_side_texture()`.

## Hunks

| file | original hunk | content | disposition |
|---|---|---|---|
| `src/entities/presentation.rs` | `@@ -2515,24 +2515,16 @@ pub fn special_item(` | the decorated-pot item branch: the array-shaped `pot_decorations` read and the inline `_pottery_sherd` suffix match become `pot_side_texture(decorations, &bone.name)` | REVERSED — `3faa03bb6c` restores the pre-fix lines verbatim (array read, `"{pattern}_pottery_pattern"`/`decorated_pot_side` selection); `57d14c6316` restores the fix. This is the item renderer's half of the row |
| `src/entities/presentation.rs` | `@@ -2560,6 +2552,20 @@ pub fn special_item(` | `pot_side_texture` added | REVERSED — deleted in the control and restored by the re-land; it has no other caller, so the control keeps no unused copy |
| `src/entities/render_blocks/surfaces.rs` | `@@ -140,19 +140,7 @@ pub(super) fn pot(` | the placed pot's per-side texture: `e.data_value()["sherds"].get(side)["id"]` becomes `pot_side_texture(e.data_value().get("sherds"), side)`, which also accepts the bare string | REVERSED — `3faa03bb6c` restores the template-only read. This is the row's own symptom: a bare-id `back`/`left`/`right`/`front` fell back to brick |
| `src/entities/presentation.rs` | `@@ -3045,9 +3051,10 @@ mod tests {` | the item test's fixture component shape (array → record) | REVERSED — a test hunk (excluded from the non-test patch), reversed only so both trees' test suites are coherent |
| `src/entities/render_blocks/village_tests.rs` | `@@ -788,11 +788,12 @@ fn static_decorated_pot_uses_received_sherd_textures()` | the placed-pot test's fixture (record form with `id`) | NOT reversed — the test uses the `{id}` form, which the pre-fix read already handled, so it passes on both trees; the kept test is what the pair's control must not red |

## a/-side adaptation

None. Every restored line is the byte-exact pre-image from `6e541f96^`; the re-land restores the
fix's own lines. No stub, no flag, no early return.

## Proving ref

`harness:scene-shot:native-decorated-pot` → `pot_side_pattern_changed`
(`tools/harness/backends/scene_shot_checks.py:294`). The scene sets one `minecraft:decorated_pot`
at `1000 160 1004`, captures it from `(1000.5, 161.62, 1002.0)` yaw 0 pitch 20, then sets the same
block with `sherds:{back:…,left:…,right:…,front:"minecraft:angler_pottery_sherd"}` in the bare form
and captures again at the same camera; the side band must change by more than 300 pixels. Measured:
control **0** changed pixels (identical brick frames), re-land **7803** (46466 pot pixels in the ROI
in both frames). The item path stays unit-covered by
`entities::presentation::tests::decorated_pot_item_bakes_all_sides_and_received_sherds` and the
placement copy by the core `block_entity_defaults` test, both green on the candidate.
