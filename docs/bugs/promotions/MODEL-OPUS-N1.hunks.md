# MODEL-OPUS-N1 — hunk map for the committed reversal pair

Promotion: `docs/bugs/promotions/MODEL-OPUS-N1.json` (written by `ledger promote`).
Scene: `native-baby-armor` (`tools/harness/baby_armor_scene.json`), proving assertion
`armour_z_over_x_in_range`.

| half | client commit | content |
|---|---|---|
| control | `client:25a210cb` (`wt/D123PackPairsCtl`) | `git apply -R` of `client:da704d9b`'s non-test hunks at `ba1971d76215`: the twenty `armor_baby_*` shells, the six baby humanoid bodies and `src/entities/krasnokamen_models.rs` |
| restore (= `fix_commit`) | `client:da704d9b` | the real fix; `git diff 25a210cb ba1971d76215` is that commit's non-test patch (patch id `12f4bc60…`) |

The three rows N1/N5/N7 share this pair (`da704d9b` rewrote the shells, the bodies and the selector
together), so the delta is the whole non-test patch; each row proves its own claim with its own scene
and assertion. `da704d9b`'s test file (`krasnokamen_models/tests.rs`) is a test hunk and is not part
of the patch.

## What the scene proves

The armour layer is its own material (`Armor*`), so one baby drowned in a diamond helmet and
chestplate (the head shell sets the depth, the chestplate adds the arm caps that widen the control)
makes the shells' bbox readable from `entity_extraction.entity_layer_geometry` (CM-4). The absolute
span scales with the kind's baby scale, which the frame does not report, so the assertion is the
scale-free `depth / width`: **measured on the pair's own pinned clients** the armour layer spans
9.02 x 12.86 x 9.66 px on the fixed tree (**1.067**, receipt
`20261126T135600-native-baby-armor-z2h0k_s7`) and 16.38 x 16.13 x 10.50 px on the control
(**0.640**, receipt `20261126T135539-native-baby-armor-5sefyjuk`); the window is 1.00..1.15. The
summon uses `Rotation:[0f,0f]` so the entity's yaw cannot swap x and z, and the fixtures (empty
frame without an armour layer, operator dump listing `minecraft:drowned`, one rendered entity) come
first.

**The mob must be a drowned, not the zombie the first version used.** `minecraft:zombie` is a
`crowd::KINDS` member: the GPU crowd renderer draws it, so the frame reports `crowd: 1`,
`submitted: 0` and no layer geometry at all on both sides (measured,
`20261126T134546-native-baby-armor-snehvdur`), and the armour layer can never be read. A baby
drowned stays on the CPU path (`baby_armor_cpu_path` asserts `crowd == 0`) and wears the same
`armor_baby_drowned_*` shells `da704d9b` rewrote.

The same class of fixture error hit MODEL-OPUS-N10: a `minecraft:skeleton` is in `crowd::KINDS` too,
so its scene now uses a `minecraft:stray` (not in KINDS, same `stray.json`).
