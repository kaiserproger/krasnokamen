# SETTLEMENT-NATURAL-LUMBER-ZONE-ONE-COLUMN — the grove fix and its control (CIV-6, 2026-11-24)

Row fix identity `patch:27471e1c38040c213d243215c39e6360feec9cf74252ffe3c6d56e8caa940554`
(`promotions/SETTLEMENT-NATURAL-LUMBER-ZONE-ONE-COLUMN.patch`, 5 hunks, one
production file, `sha256` of the file = the identity). The patch reverse-applies
on the candidate (`git apply -R --check` at `8f29602ac4`) and the reversed tree
compiles: the control receipt below ran the row's test on it and reached the
row's own assertion.

## Commits the patch carries

| commit | what it closes |
|---|---|
| `8a93c14578` "fix(settlements): certify a natural lumber camp from its grove" | the fix: a proved natural lumber candidate's zone grows sideways at certification time |
| `544c763a44` "fix(settlements): grow a natural lumber grove only into simulated chunks" | review wave2-8 minor 1: growth requires simulated chunks (`bounds_active`, not `bounds_loaded`'s residency) and the `grove_bounds` doc states shape, cost and lot/base rules |

`8f29602ac4` ("test(settlements): assert the grove's coverage…") is tests only:
its hunks are not in the patch, because the control keeps the tests and reverses
the production code under them.

## Hunk map

| # | `@@` | a/-side (pre-image) | b/-side (candidate) |
|---|---|---|---|
| 1 | `@@ -24,6 +24,9 @@` | no `GROVE_RADIUS` | `const GROVE_RADIUS: i32 = 6` + doc |
| 2 | `@@ -727,7 +730,7 @@` | `_site: Option<&GeneratedVillageSite>,` | `site: Option<&GeneratedVillageSite>,` |
| 3 | `@@ -840,13 +843,30 @@` | plain `certify_zone(candidate.bounds)` | `grove_bounds` for `LumberCamp`, plain bounds as the fallback |
| 4 | `@@ -857,6 +877,104 @@` | no `grove_bounds` | `fn grove_bounds` + doc (doc and `bounds_active` check from `544c763a44`) |
| 5 | `@@ -894,6 +1012,16 @@` | no `bounds_active` | `fn bounds_active` (simulated chunks) |

The production call site `Some(&pending.site)` is identical on both sides and is
therefore not a hunk.

## a/-side adaptation (class b)

The a/-side is `git show 8a93c14578^:crates/mc-core/src/settlements/autonomy.rs`
plus one class-(b) adaptation: the `_site: Option<&GeneratedVillageSite>`
parameter and the `Some(&pending.site)` argument. The kept tests
(`autonomy_tests.rs`) call `prove_natural` with five arguments, so reversing the
parameter would stop the control from compiling. On the reversed tree `_site` is
unused and the argument is inert: no behaviour change. Nothing else in the
a/-side differs from the real pre-image.

(The previous revision of this patch carried only 3 hunks, omitted `grove_bounds`
and therefore reversed to a tree with `GROVE_RADIUS` deleted and `grove_bounds`
still referring to it — a build failure, and its red receipt came from another
tree. Review wave2-8 B1.)

## The fix (what it does)

`natural_zones` probes a tree and queues a candidate whose bounds are the trunk
column plus the entrance. `prove_natural` certified those bounds unchanged, so
`certify_zone` saw a `1x8x1` (or `2x1`) rectangle that only had to pass ">= 4
logs"; the gate's phase log (run `20261124T020538`) found exactly that. Now a
proved `LumberCamp` candidate's rectangle grows one column per side while

- its eight corners stay inside the settlement sphere (`certify_zone`'s `within`),
- every column of the rectangle is simulated (`bounds_active`),
- no certificate or generated village piece stands there,

and `certify_zone` still owns overlap, >= 4 zone logs and the open entrance; the
plain column is the fallback, so a grove can never turn a certification into a
new failure. Cost per certification: at most `4 * GROVE_RADIUS` = 24 probes,
each O(1) plus one clearance scan over `B` certificates and `P` pieces.

## Red and green (both observed)

- Red — reversed tree of `8f29602ac4` (this patch applied with `git apply -R`;
  head `8f29602ac4`, dirty `5085ee47…`):
  `.analysis/validation/20261124T160707-test-jywi4zet` FAIL, exit 100. The test
  `settlements::autonomy::tests::natural_lumber_camp_certifies_the_grove_not_one_tree_column`
  panics at its own assertion, `autonomy_tests.rs:685`,
  `a natural camp's zone is the grove, not one tree column: 2x1 at [-15, 201, 4]..[-14, 206, 4]`.
- Green — candidate `8f29602ac4` (clean head; `docs/bugs/` only in the diff):
  `.analysis/validation/20261124T161120-test-iq4zef1q` PASS, `natural_` filter
  (the grove test, the new `a_grove_sized_natural_zone_keeps_a_lot_beside_it` and
  the rest of the natural-zone suite).

## Still missing

`harness:civilization-seed-qa:all` — no passed gate receipt exists on any tree of
this row (20261124T131853 expired at 7239 s: the settlement still stopped
progressing before a house, an iron smelt or births). The row stays `open`; this
map fixes the control only.
