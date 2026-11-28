# SETTLEMENT-NATURAL-LUMBER-ZONE-ONE-COLUMN — the grove fix and its HEAD control (CIV-6)

Row fix identity `patch:113f3d8cdcd459341498bbacb7401c098a5ed5418ad72e8cf2163ea416e4aa43`
(`promotions/SETTLEMENT-NATURAL-LUMBER-ZONE-ONE-COLUMN.patch`, 5 hunks, one
production file). `sha256` of the patch's non-test hunks = the identity. The
patch reverse-applies on the current candidate: `git apply -R --check` passes at
`bbf5b71e5`, and the reversed tree compiles and reaches the row's assertion.

## Why this revision (2026-11-28)

The 2026-11-24 revision (`patch:27471e1c…`) no longer reversed at HEAD: hunk 3's
context was rewritten by `836f66b0b` "a mine zone holds no built piece in 3D",
which replaced the `if certify_zone(…).is_err() { let _ = certify_zone(…) }`
shape with the `refused`/logging match. The HEAD equivalent keeps that later
logging (another review wave's behaviour) and reverses only the grove growth:
`certify_zone` receives `candidate.bounds` instead of `grown`.

## Commits the patch carries

| commit | what it closes |
|---|---|
| `8a93c14578` "fix(settlements): certify a natural lumber camp from its grove" | the fix: a proved natural lumber candidate's zone grows sideways at certification time |
| `544c763a44` "fix(settlements): grow a natural lumber grove only into simulated chunks" | review wave2-8 minor 1: growth requires simulated chunks (`bounds_active`) and the `grove_bounds` doc states shape, cost and lot/base rules |

`8f29602ac4` ("test(settlements): assert the grove's coverage…") is tests only:
its hunks are not in the patch, because the control keeps the tests and reverses
the production code under them.

## Hunk map (at `bbf5b71e5`)

| # | `@@` | a/-side (pre-image) | b/-side (candidate) |
|---|---|---|---|
| 1 | `@@ -26,6 +26,9 @@` | no `GROVE_RADIUS` | `const GROVE_RADIUS: i32 = 6` + doc |
| 2 | `@@ -947,7 +950,7 @@` | `_site: Option<&GeneratedVillageSite>,` | `site: Option<&GeneratedVillageSite>,` |
| 3 | `@@ -1114,11 +1117,18 @@` | certify `candidate.bounds` (the later `refused` logging kept) | `grown` = `grove_bounds` for `LumberCamp`, plain bounds otherwise |
| 4 | `@@ -1156,6 +1166,104 @@` | no `grove_bounds` | `fn grove_bounds` + doc |
| 5 | `@@ -1204,6 +1312,16 @@` | no `bounds_active` | `fn bounds_active` (simulated chunks) |

The production call site `Some(&pending.site)` is identical on both sides and is
therefore not a hunk.

## a/-side adaptations

- Class (b): `_site: Option<&GeneratedVillageSite>` stays in the signature (with
  the original `Some(&pending.site)` argument) because the kept tests call
  `prove_natural` with five arguments. On the reversed tree it is unused: no
  behaviour change.
- Class (a)/(c): the `refused` match and its `tracing::info!`
  (`natural zone refused certification`) came in `836f66b0b`, another review
  wave's behaviour. The HEAD equivalent does not reverse it; only `grown` vs
  `candidate.bounds` flips. Reversing the logging would be someone else's
  hunk and no test needs it.
- Nothing else in the a/-side differs from the real pre-image.

## The fix (what it does)

`natural_zones` probes a tree and queues a candidate whose bounds are the trunk
column plus the entrance. Before the fix `prove_natural` certified those bounds
unchanged, so `certify_zone` saw a `1x8x1` (or `2x1`) rectangle that only had to
pass ">= 4 logs". Now a proved `LumberCamp` candidate's rectangle grows one
column per side while

- its eight corners stay inside the settlement sphere (`certify_zone`'s `within`),
- every column of the rectangle is simulated (`bounds_active`),
- no certificate or generated village piece stands there,

and `certify_zone` still owns overlap, >= 4 zone logs and the open entrance; the
plain column is the fallback, so a grove can never turn a certification into a
new failure. Cost per certification: at most `4 * GROVE_RADIUS` = 24 probes,
each O(1) plus one clearance scan over `B` certificates and `P` pieces.

## Red and green (both observed on `bbf5b71e5`)

- Red — this patch reversed with `git apply -R` (2 insertions, 120 deletions in
  `autonomy.rs`): `.analysis/validation/20261128T085525-test-h0jp3cnb` FAIL,
  exit 100. The test panics at its own assertion, `autonomy_tests.rs:993`,
  `a natural camp's zone is the grove, not one tree column: 2x1 at [-15, 201, 4]..[-14, 206, 4]`.
- Green — clean candidate: `.analysis/validation/20261128T092429-test-07wzg06v`
  PASS for
  `settlements::autonomy::tests::natural_lumber_camp_certifies_the_grove_not_one_tree_column`.

## Still missing

`harness:civilization-seed-qa:all` — no passed gate receipt exists on any tree of
this row (20261124T131853 expired at 7239 s). This map fixes the control only;
the graphical gate is outside this worker's reach.
