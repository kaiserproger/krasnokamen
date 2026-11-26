# MODEL-BLOCK-ENTITY-DRAW — hunk map for the tip reversal pair

Promotion receipt: `promotions/MODEL-BLOCK-ENTITY-DRAW.json` (written by `ledger promote`).
Client pair: control `client:7a653e5d84` → re-land `client:00c7efd111` (the row's `fix_commit`).
Both pins clean (`7a653e5d846a`, `00c7efd11192`); the pair's net tree is the lane tip `2563ce95`
plus the row's own layer fix `127089bba2` (CLIENT-OPERATOR-OVERLAY-HARD-FAIL, which keeps the overlay
geometry valid so the fixture's frames can be captured at all).

## Why the pair exists, and the commit that closed the path

The row's credited fix is `client:83994480` ("integrate ClientNext slices on RC5", 2026-11-09). Its
`src/entities/render_blocks.rs` hunks are: the two factory/filter entries
(`minecraft:structure_block`, `minecraft:test_instance_block`), the `StaticBlockSceneFrame`
`operator_overlay_visible` field plus its destructuring, the two `append_static_block_geometry` match
arms, and the `gateway` predicate that gives a test instance the 256-block limit of an end gateway.
The pre-image drew 22 of the 27 factory types: a structure block's wire box and a test instance's
box/error markers were never submitted.

Two later commits reworked these lines without changing their meaning:
- `54b79f73d` (2026-11-14) reflowed them (shorthand `StaticBlockSceneFrame` fields, split `matches!`
  and calls);
- `8347908e2` (2026-11-15) replaced the filter literal with
  `draws_static(name) = has_factory(name) && name != "minecraft:piston"`. **`has_factory` is not only
  the scene filter at HEAD: it is also the block-entity constructor registry**, so the credited
  commit's "remove the two entries from the literal" is not reversibly expressible there. The control
  removed the two entries from `has_factory` first and the client refused the join
  (`session failed: Play packet 0x08 … unknown block entity constructor type:
  minecraft:structure_block`, run `20261124T194650-native-structure-box-peqzk69t`), never reaching
  the row's assertion. `has_factory` therefore keeps both entries and the control excludes them from
  the *scene filter* instead — the HEAD equivalent of the pre-image literal list.

## Hunks carried by the pair

`git diff 7a653e5d84 00c7efd111 -U0` = 31 changed lines (25 added, 6 removed), one file.

| pair hunk | HEAD lines | content | disposition |
|---|---|---|---|
| `@@ -193 +193,4 @@` | `render_blocks.rs:193-196` | `let gateway = block.type_name == "minecraft:end_gateway";` → `matches!(block.type_name.as_str(), "minecraft:end_gateway" \| "minecraft:test_instance_block")` | REVERSED — the control restores `83994480^`'s `==` predicate verbatim (the `-` line of the credited commit) |
| `@@ -381,0 +385,20 @@` | `render_blocks.rs:382-401` | the `"minecraft:structure_block" => surfaces::structure(…)` and `"minecraft:test_instance_block" => surfaces::test_instance(…)` arms | REVERSED — deleted in the control, restored by the re-land |
| `@@ -565,5 +588 @@` | `render_blocks.rs:587-593` | `draws_static`: `has_factory(name) && name != "minecraft:piston"` ↔ `has_factory(name) && !matches!(name, "minecraft:piston" \| "minecraft:structure_block" \| "minecraft:test_instance_block")` | REVERSED — the control's exclusion list is the HEAD equivalent of `83994480^`'s filter literal (which listed every factory except the two types) |

## Pair-only lines (measured 2026-11-25)

Line-by-line comparison with `git show 83994480 -- src/entities/render_blocks.rs`: 31 pair lines, 12
byte-equal to an origin line, 19 not. Blame is `git blame -C -C 2563ce9514 <line>` on the pair's net
tree (blame at the pair commits names the pair by construction).

| pair lines (fix-tree numbers) | blame at `2563ce9514` | class |
|---|---|---|
| `render_blocks.rs:193` `let gateway = matches!(`, `:194`, `:195`, `:196` `);` | `54b79f73d` | reflow: the origin added the same expression on one line |
| `render_blocks.rs:385` `"minecraft:structure_block" => {`, `:386` the call, `:387` `}` | `54b79f73d` | reflow: the origin wrote `=> surfaces::structure(` with its arguments on the next line |
| `render_blocks.rs:392-397` (six shorthand `TestInstanceFrame` fields) | `54b79f73d` | reflow: the origin wrote the same six bindings on one line |
| `render_blocks.rs:588` `has_factory(name)`, `:589` `&& !matches!(`, `:590` `name,`, `:591` the three-type list, `:592` `)` | `8347908e2` (control side) / `83994480` (the two types in it) | (c) a later commit rewrote the filter literal into `draws_static`; the control's a/-side is that function minus the two types, i.e. exactly the pre-image's behaviour. The two type names are the credited commit's own strings |
| `render_blocks.rs:588` `has_factory(name) && name != "minecraft:piston"` (re-land side) | `8347908e2` | (c) the HEAD form of the filter; byte-identical to the pre-promotion tree |

Zero behavioural deviations of another row: every pair-only line is a reflow or the filter's HEAD
form. `has_factory`'s two entries are untouched (see above), so no other block-entity type moves.

## a/-side adaptation

One, named: the control's `draws_static` body `has_factory(name) && !matches!(name, "minecraft:piston"
| "minecraft:structure_block" | "minecraft:test_instance_block")` is the HEAD equivalent of
`83994480^`'s filter literal — the same exclusion, expressed against the function `8347908e2`
introduced. It changes no other type's behaviour and is not a stub or a re-break: the pre-image's own
literal listed all 22 types and omitted exactly these two. Keeping `has_factory` intact is forced by
the HEAD constructor registry (see the failed join above), which is the (b) class: API the kept
protocol path needs.

## Every omitted hunk

| commit | hunk | disposition |
|---|---|---|
| `83994480` | `src/entities/render_blocks.rs` `@@ -126,6 +126,8 @@` (`operator_overlay_visible` field + doc) and `@@ -156,7 +158,7 @@` (destructuring) | (b) API the kept tests and `session/observation.rs` pass; reversing it stops the control from compiling |
| `83994480` | `src/entities/render_blocks.rs` `@@ -1189,7 +1208,7 @@`, `@@ -1268,7 +1287,7 @@`, `@@ -1318,7 +1337,7 @@` (three inline-test literals) | test hunks inside `mod tests` (the kept literals' `operator_overlay_visible: false`) |
| `83994480` | every other file of the commit (mob rigs and `assets/solaris/models/entity/*`, `equipment.rs`, `presentation.rs`, `world_fx`, `ui/state.rs`, `protocol/models*.rs`, …) | (a) other rows' behaviour; nine other ledger rows carry their own pairs/maps, and `CM1-REVERSIBILITY.md` records per file whether each hunk still reverses |

## Proving ref

`harness:scene-shot:native-structure-box` → `structure_block_submitted`
(`tools/harness/backends/scene_shot_checks.py:663`; the control receipt's error reads
`scene-shot assertion failed: structure_block_submitted`). `structure_box_changed` is not an
assertion: it is the report key `structure_box_changed_pixels` (`scene_shot_checks.py:656`), and the
scenario prose still names it (`tools/harness/scenarios.py:1211`). The control joins and captures
both frames (the block entity is constructed and simply never submitted), so its red is an observed
assertion failure, not a crash: 0 changed pad pixels on the control versus 2565-5679 on the re-land.
The five unit refs in the row run on the candidate and stay green (`entities::render_blocks::village_tests`:
38 passed).
