# Owner decisions needed — D123Mechanics (from the control reviews, 2026-11-24)

## 1. New-handler rows have no honest control — accept an absent-handler stub?

Rows whose fix introduced a brand-new handler/module (or whose only written patched fix is a digest
without a resolvable commit) cannot be reversed honestly: reversing the fix does not compile, and an
early-return stub invents an a/-side the reviewers rejected. These rows are open, not
fixed-unpromoted (the ledger has no such state):

- FALL-002 (brush handler introduced by `35308579c`; `brushing.rs` module)
- INTERACT-001 (axe handler; the pre-fix form of `vanilla_interact::axe` cannot be restored)
- SPONGE (absorption pass added by `35308579c`)
- COMMAND-CHILD-SENDER-DROPPED (the stored fix is `patch:d761f696…`; no commit to resolve)
- TOTEM / DEATH-PROTECTION-COMPONENT (old control red was a fixture panic; the rescue entry is a
  new-generation handler)
- SPECIAL-001 (old control red was a fixture panic; pot/shelf storage module)
- DATA-QUERY (its stored patch carried a synthetic a/-side; the HEAD pre-image needs archaeology of
  `5c060d5df`/`0c6f68d8`)

Question: accept an early-return stub as the *control* for these rows (documented as an adapted
re-break in hunks.md), or keep them open until a real pre-image control exists?

My recommendation: keep them open; stubs as controls weaken the negative-control rule. If you accept
stubs, each needs a one-line caveat in `promotions/<ID>.hunks.md` and I can re-promote them quickly.

# Civilization lane — remaining non-ready rows (D123CivD, 2026-11-24)

Head `797e6dbf33`; the lane's other 39 rows are READY in
`D123Civ/SWEEP-MANIFEST.md` (each with a HEAD control patch). These five need
someone above the lane:

## CIV-1. SPAWN-PEACEFUL-REMOVAL (owner item #9) — no honest narrow control

`e2e05b33e`'s despawn path was restructured by `2dfebd7ab`, so the fix's own
hunks no longer exist at HEAD and the only control that ever went red was a
synthetic mutation. Options: (a) keep the row open until the sweep runs the
`harness:entities:native-v2-peaceful-removal` gate on a candidate whose
`apply_peaceful_removal` call is deleted with an adapted pre-image; (b) accept a
documented adapted re-break. Recommendation: (a).

## CIV-2. ISSUE-1-17 (umbrella) — closes with its two sub-rows

Both sub-rows are READY (`ISSUE-1-17-AUTOLAYOUT-HOUSING-NOT-SEEN-IN-GAME`,
`ISSUE-1-17-NO-PERIMETER-WALL-GATE-OR-WELL`). The umbrella should be closed by
their sweep, never promoted alone.

## CIV-3. CIV-ADOPTED-CLOSURE-ROOT-TICK-COST — patch mode impossible

The cost is an emergent root-tick closure, not a reversible hunk; the row needs
an owner decision on the acceptable bound, not a control.

## CIV-4. SETTLEMENT-RESIDENTS-TRAPPED-IN-PLAZA-BASIN — vanilla evidence, provenance question

The vanilla server traps the same body; the row is an entry/provenance question
for the owner (see the row text).

## CIV-5. SETTLEMENT-AUTONOMY-SCAN-GROWTH-AFTER-PREP-UNEXPLAINED — attribution measured

Growth enters in `7e07afbb9..04b3ce757` (PrepPortions portion/ledger slice) and
HEAD closes at 47,189 / 1,837,299 against the 1,900,000 allowance
(`D123Civ/prep-scan-ab/`); the row has no production hunk and no registered
check, so it needs an owner decision on the allowance, not a control.
