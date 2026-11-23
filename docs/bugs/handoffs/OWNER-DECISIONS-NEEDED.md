# Owner decisions needed — D123Mechanics (from the control reviews, 2026-11-24)

## 1. New-handler rows — resolved by a real pre-image; one row still needs the owner

The control review rejected the stub a/-sides of these rows. Re-examined against the branch, each fix's
*pre-image is the dispatch or decision point*, not a stub, and a HEAD-equivalent reverse of real prior
code reproduces the row's symptom with a red at the row's own assertion:

- `FALL-002` — the two `brushing::use_on` dispatch terms (`tick/input/interact.rs:330-332`,
  `:625-627`); pre-fix the brush use was never reached. Patch `patch:414ec86d…`.
- `INTERACT-001` — the `vanilla_interact::axe` term (`interact.rs:330-332`); pre-fix the axe use was
  never reached. Patch `patch:db1440f9…`.
- `SPONGE` — the `sponge::after_blocks` tick hook (`tick/dimension.rs:378-379`); pre-fix no absorption
  pass ran after placement. Patch `patch:057a21dc…`.
- `TOTEM` / `DEATH-PROTECTION-COMPONENT` — the rescue decision in `commit_damage`
  (`tick/damage.rs:660-675`): the patch restores the pre-fix `died |= player.vitals.hurt(...)` form. Patch
  `patch:6de54645…`. Their cargo refs pass and the control is ready, but the registered real-client
  scenario `native-v2-command-score-store` times out on the candidate
  (`timed out waiting for damage_command_received_health`,
  `.analysis/validation/20261123T211805-native-v2-command-score-store-5r_93kzn/result.json`), so no
  `passed` receipt exists; that scenario also does not exercise totems, so it cannot be the row's real
  proof even when green. Decision needed: a totem-specific real-client scenario (owner/gate owner).
- `DATA-QUERY` — no honest control found: `git show 0c6f68d8^:crates/mc-core/src/command_data.rs` does
  not exist and the pre-fix `command_blocks::data` handler is gone at HEAD. Dropping `data` from
  `command_context::native`'s root list does not red the row's refs (observed with the reverse applied:
  `.analysis/validation/20261123T212911-test-7u1iawnh`, both refs PASS). Decision needed: a ref that
  depends on the routing, a client-only proof, or keep open.
- `COMMAND-CHILD-SENDER-DROPPED` — resolved outright: the real fix commit is `334c3f686`; its diff no
  longer reverses after `76a539ea9`, and the HEAD-equivalent patch restores the root-context
  `source_is_current` check. Patch `patch:b8374c6a…`; promoted with five red cargo refs.

Promoted from this list with fresh real-client receipts on the pinned candidate `9c78eee8e` (clean):
`FALL-002` (brush receipt `20261123T212015`), `INTERACT-001` (axe `20261123T211938`), `SPONGE`
(sponge `20261123T212044`), `DEATH-NOTIFY` (death-messages `20261123T211907`). No stub was used anywhere.

**Still needs the owner: `SPECIAL-001`.** `special_storage.rs` does not exist at `35308579c^`
(`git show 35308579c^:crates/mc-core/src/special_storage.rs` = "does not exist"), and the pot/shelf
behaviour has no pre-image at all — the module *is* the fix. Its two refs (`shelf::tests::…`,
`interact_parity_tests::interact_parity_decorated_pot_stores_one_item`) cannot be red without an invented
stub. Owner decision: accept a documented absent-module stub for this one row, split the row into
"module exists + hydrates" (provable by a new loss test at HEAD) and "vanilla behaviour" (client gate), or
keep it open.

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
