# MODEL-BLOCK-ENTITY-DRAW — hunk map for the tip reversal pair

Promotion receipt: `promotions/MODEL-BLOCK-ENTITY-DRAW.json` (written by `ledger promote`).
Client pair: reversal `client:98f328d3be` → re-land `client:c2f98be2b3` (the row's `fix_commit`).
Both pins clean; the pair's net tree is the lane tip `2563ce95` plus the row's own layer fix
`127089bba2` (CLIENT-OPERATOR-OVERLAY-HARD-FAIL, which keeps the overlay geometry valid so the
control can render frames instead of crashing).

## Why the pair exists, and the commit that closed the path

The row's credited fix is `client:83994480` ("integrate ClientNext slices on RC5", 2026-11-09). For
`src/entities/render_blocks.rs` its non-test hunks are exactly: two `has_factory` entries
(`minecraft:structure_block`, `minecraft:test_instance_block`), the `StaticBlockSceneFrame`
`operator_overlay_visible` field and its destructuring, the two `append_static_block_geometry` match
arms, and the `gateway` predicate that gives a test instance the 256-block limit of an end gateway.
The pre-image drew 22 of the 27 factory types: a structure block's wire box and a test instance's
box/error markers were never submitted, however the world was set up.

A later commit `54b79f73d` (2026-11-14) reflowed these lines: `StaticBlockSceneFrame`'s fields
became shorthand (`origin` instead of `origin: origin`), the `gateway` `matches!` and the arm calls
were split across lines. Its semantic content is unchanged, and `8347908e2` (2026-11-15) replaced
the filter literal with `draws_static`/`has_factory` without changing the two entries.

`83994480^` already listed `brushable_block` and `shelf` in both the filter literal and the
`campfire | vault | brushable_block | shelf` display arm, so those two were never part of this fix;
the row's title claimed them and is corrected.

## Hunks carried by the pair

`git diff 98f328d3be c2f98be2b3 -U0` = 27 changed lines (26 added, 1 replaced), one file.

| pair hunk | HEAD lines | content | disposition |
|---|---|---|---|
| `@@ -193 +193,4 @@` | `render_blocks.rs:193-196` | `let gateway = block.type_name == "minecraft:end_gateway";` → `matches!(block.type_name.as_str(), "minecraft:end_gateway" \| "minecraft:test_instance_block")` | REVERSED — the control restores `83994480^`'s `==` predicate verbatim (the `-` line of the credited commit) |
| `@@ -381,0 +385,20 @@` | `render_blocks.rs:382-401` | the `"minecraft:structure_block" => surfaces::structure(…)` and `"minecraft:test_instance_block" => surfaces::test_instance(…)` arms | REVERSED — deleted in the control, restored by the re-land; both are `83994480`'s own text (14 of the 27 lines byte-equal) |
| `@@ -545,0 +569,2 @@` | `render_blocks.rs:569-570` | the two `has_factory` entries | REVERSED — deleted in the control; without them `draws_static` never yields the two entities, so the arms are unreachable as in the pre-image |

## Pair-only lines (measured 2026-11-25)

Line-by-line comparison with `git show 83994480 -- src/entities/render_blocks.rs`: 27 pair lines, 14
byte-equal to an origin line, 13 not. Blame is `git blame -C -C 2563ce9514 <line>` on the pair's net
tree (blame at the pair commits names the pair by construction).

| pair lines (fix-tree numbers) | blame at `2563ce9514` | class |
|---|---|---|
| `render_blocks.rs:193` `let gateway = matches!(`, `:194` `block.type_name.as_str(),`, `:195` `"minecraft:end_gateway" \| "minecraft:test_instance_block"`, `:196` `);` | `54b79f73d` | reflow: the origin added the same expression on one line; no behaviour change |
| `render_blocks.rs:385` `"minecraft:structure_block" => {`, `:386` the call, `:387` `}` | `54b79f73d` | reflow: the origin wrote `=> surfaces::structure(` with its arguments on the following line; same call |
| `render_blocks.rs:392-397` (`origin,` … `scoping,`, six shorthand fields of `TestInstanceFrame`) | `54b79f73d` | reflow: the origin wrote the same six bindings on one line (`origin, camera, camera_rotation, game_time, partial, scoping,`) |

Zero behavioural deviations: every pair-only line is the same tokens re-laid out by the reflow
commit, and the a/-side of every hunk is the credited commit's own removed line or its absence.

## a/-side adaptation

None. The control's only `+` line (`let gateway = block.type_name == "minecraft:end_gateway";`) is
byte-identical to the `-` line of `83994480`; every other change is a deletion of lines
`83994480` added. No stub, no flag, no early return, no constructed API: the control compiles
because `dead_code` is allowed crate-wide (`src/lib.rs`), and its static scene simply never visits a
structure block or a test instance.

## Every omitted hunk

| commit | hunk | disposition |
|---|---|---|
| `83994480` | `src/entities/render_blocks.rs` `@@ -126,6 +126,8 @@` (the `operator_overlay_visible` field + doc) | (b) API the kept tests call: `StaticBlockSceneFrame` literals in the kept tests and in `session/observation.rs` pass the field; reversing it stops the control from compiling |
| `83994480` | `src/entities/render_blocks.rs` `@@ -156,7 +158,7 @@` (destructuring) | (b) same kept API |
| `83994480` | `src/entities/render_blocks.rs` `@@ -1189,7 +1208,7 @@`, `@@ -1268,7 +1287,7 @@`, `@@ -1318,7 +1337,7 @@` (three inline-test literals) | test hunks inside `mod tests`: the kept literals' `operator_overlay_visible: false` |
| `83994480` | every other file of the commit (mob rigs and `assets/solaris/models/entity/*`, `equipment.rs`, `presentation.rs`, `world_fx`, `ui/state.rs`, `protocol/models*.rs`, …) | (a) other rows' behaviour; nine other ledger rows carry their own pairs/maps (`MODEL-USE-POSE`, `MODEL-OPUS-3`, `MODEL-ELEMENT-EMISSION`, …), and `CM1-REVERSIBILITY.md` records per file whether each hunk still reverses |

## Proving ref

`harness:scene-shot:native-structure-box` → `structure_box_lines_drawn` (plus
`structure_box_changed`). The control receives a clean, non-crashing run: the wireframe is absent in
both frames (`structure_box_lines_drawn` 0) because the two entity types never reach the static
scene; the re-land draws it. The five unit refs in the row run on the candidate and stay green.
