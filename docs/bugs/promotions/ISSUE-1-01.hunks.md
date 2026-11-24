# ISSUE-1-01 — hunk map

Row: `ISSUE-1-01` (client, major) — "Villager leg model translucent and malformed". Refs:
`harness:scene-shot:issue1-01-villager-leg` (`villager-leg-vanilla_pixels`).

## Real fix and its pre-image

Client `97e988d2f8d3d7a85fc11d1d419833291822f2c0` ("fix(client): villager-family models use the
vanilla unwrap; held items use source display-transform order and replace the first-person arm",
2026-10-19) predates the client rename `f0a6abf6`, so the route is the committed-reversal pair on the
current tip:

- control `6069e262ff9e...` — "test(client): restore the pre-fix villager-family models for
  ISSUE-1-01's control". The four model files are byte-exact `97e988d2^` blobs at the HEAD paths
  (`git show 97e988d2^:assets/solaris/models/entity/<name>.json`); no line was written by hand. The
  `presentation.rs` hunks were hand-ported to the current `role_pose`/`evaluate_rig` shape (below).
- re-land `938ccc19...` — "fix(client): sample the villager-family models at the vanilla texture
  regions (ISSUE-1-01)", the row's `fix_commit`. Its tree is identical to `184fcf8ce5`.
- `git diff 6069e262 938ccc19` is exactly the re-land's own non-test patch over the five paths
  (`client_control_delta.fix_patch_id 6ae8d6e88a710be851b4b2cb9adc154a549851b6`).

## Carried hunks and their HEAD equivalents

| real hunk | what it closes | HEAD equivalent |
|---|---|---|
| `villager.json @@ -5,96 +5,69` (whole `roles`/`bones` block) | legs/jacket/arms/hat rim at the vanilla unwrap; the pre-image legs sample the arm region (`uv [40,38]`) and the hat rim is a horizontal plate | `assets/krasnokamen/models/entity/villager.json` at HEAD (legs `uv [0,22]`, vertical hat plate) |
| `wandering_trader.json @@ -5,76 +5,69` | same, trader | the HEAD model |
| `witch.json @@ -5,32 +5,55`, `@@ -40,43 +63,35` | same, witch (the nose bone arrives with this model) | the HEAD model |
| `zombie_villager.json @@ -4,17 +4,16`, `@@ -24,77 +23,57` | same, zombie villager | the HEAD model |
| `presentation.rs @@ -656,6 +656,15` | the family's half-amplitude leg swing (`leg_swing`) | the block at `presentation.rs:2199-2206` at HEAD |
| `presentation.rs @@ -687,9 +696,9` | `RightLeg`/`LeftLeg` multiply by `leg_swing` | `presentation.rs:2303` `let swing = 1.4 * amplitude * leg_swing;` (the pre-image is the same line without the factor) |

## a/-side adaptation lines

- The four model files are the real pre-image verbatim; only the stage0 rebrand paths
  (`assets/solaris` → `assets/krasnokamen`) are already applied by the rename.
- The ISSUE-1-23 `"crossed_arms": ["arms"]` role (client `586b2d07`, already promoted) stays in the
  a/-side `roles` block — the surviving half of a mixed hunk; the pre-image model has the `arms` bone
  it names.
- The witch's `"nose": ["nose"]` role is **not** in the a/-side: the pre-image witch model has no
  `nose` bone, so the role arrives with this fix's model (the b-side carries it).
- `presentation.rs`: the a/-side removes the `leg_swing` definition and its two uses; every other
  line of the surrounding function is unchanged at HEAD.

## Every other non-test hunk of 97e988d2 and why it is not in this pair

| file | hunks | what they do | reason |
|---|---|---|---|
| `src/entities/presentation.rs` | `@@ -848,16 +857,20` (`item_matrix` → `ui::items::display_matrix`, new `held_item_attachment`), `@@ -929,10 +942,7` (`append_equipment` uses it) | held-item pose and first-person hand | (a) ISSUE-1-02 (held tool at rest) and ISSUE-1-21 (tool clipping), each promoted as its own pair |
| `src/player/visual.rs` | `@@ -343,71` | first-person item alone from the source hand anchor | (a) ISSUE-1-02/ISSUE-1-21 |
| `src/render/frame.rs` | `@@ -78,6`, `@@ -97,11` | frustum rejection precedes instance/indirect submission | (a) ISSUE-1-03's direction claim and ISSUE-1-21 |
| `src/render/frame.rs` | `@@ -893,4 +896,71` | terrain translucency test module | test hunks (kept both sides) |
| `src/ui/items.rs` | `@@ -222,12`, `@@ -553,8` | `ItemTransform` rotation order and the display matrix | (a) ISSUE-1-02 |
| `src/entities/animation_tests.rs` | `@@ -184,6` | model/gaits unit tests | test file |
| `docs/AGENT_TESTING.md` | `@@ -446,7`, `@@ -461,6` | gate notes | documentation |

## Gate

`harness:scene-shot:issue1-01-villager-leg` on core `483a2d5ab` (server pin registered for that tree),
one control/restore pair of real native clients. The old window (`560,240,760,470`, brown
`40..130/30..100/20..80`, 8000 px) measured grass — its brown box contains the terrain green — so the
pre-fix legs passed it. The window now covers the leg band only (`596,322,700,346`) and counts the
vanilla leg texture's grey boot/hem (`55..100` per channel), which the arm-region pre-image cannot
produce: measured 756 px on the fixed client and 242 px on the pre-image, threshold 500.

- candidate (`938ccc19`, receipt `.analysis/validation/20261124T200348-issue1-01-villager-leg-azbc6oie`):
  PASSED — `villager-leg-vanilla_pixels` true, 756 px.
- control (`6069e262`, receipt `.analysis/validation/20261124T200212-issue1-01-villager-leg-iant1l0o`):
  FAILED at `villager-leg-vanilla_pixels` (242 px).

Known limitations (REVIEW-wave2-12 m6):

- The hand-ported `leg_swing` hunks are **not** exercised by this gate: the scene's villager is NoAI,
  so its walk speed — and therefore the leg amplitude the factor scales — is zero. The pair is a
  faithful pre-image of those lines, but only the model hunks are checked by pixels.
- The scene is static: the promotion pair measured candidate 756 px and pre-image 242 px against a
  threshold of 500 (a roughly threefold margin), and this pack re-ran the candidate three times at the
  tip (20261124T221450-family and 20261124T224104) without the arm window moving. Re-measure both pins
  before changing the window, the colour box or the client lighting.
