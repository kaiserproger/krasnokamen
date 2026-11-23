# D123Mechanics HANDOFF-4 — reviewer-delta wave (2026-11-24)

Status: **partial** — the review's N1 and N6 reopen/cleanup are done; N2/N3/N4/N5 remain and are listed
with exact next steps. Head: f818585a99 plus this delta commit (docs only).

## Done (N1)

Synthetic stub controls withdrawn, rows reopened (status open, promotion removed, records and stale
receipts deleted):

- FALL-002, INTERACT-001, SPONGE, COMMAND-CHILD-SENDER-DROPPED (the four the delta review named)
- TOTEM, DEATH-PROTECTION-COMPONENT, SPECIAL-001 (the three whose only "control" was an early-return
  stub I had just generated; `git apply -R` of those stubs was never used for a promotion)
- DATA-QUERY: stale `patch:3ac0a62…` patch and fix_commit removed

Grouped owner decision: `receipts/D123/OWNER-DECISIONS-NEEDED.md` (new-handler rows have no honest
control; accept an absent-handler stub or keep them open).

## Done (N6, partial)

- `promotions/{TOTEM,DEATH-PROTECTION-COMPONENT,SPECIAL-001}.{patch,hunks.md}` and
  `fix-patches/{…}.patch` deleted; the shared totem digest no longer exists in the tree.
- Reopened rows' `repro_check` no longer claim "fail at own assertions"; each names the blocker.
- Dead `fix-patches/{FALL-002,INTERACT-001,SPONGE,COMMAND-CHILD-SENDER-DROPPED,DATA-QUERY}.patch` removed.
- PROGRESS.md row count corrected below; the FALL-001 paragraph in HANDOFF-2 is corrected here: FALL-001
  stays open on the native probe red (`native-v2-ledger-no-command-edit`), it is not fixed.

## Remaining (not done in this wave)

- N2: the 8 gate-wave `hunks.md` files still lack an original commit sha (their controls were lane temp
  edits, never committed) and ANVIL/MINING lack the (a) omission lists. Next: for each row take the
  sha from the same probe plan the row's DIAGNOSIS cites, list the control's carried symbol with its
  HEAD file:line, and move the omitted-hunks note from my one-line form to the (a)/(b)/(c) form of
  SHELF-HYDRATE-PANIC.hunks.md. PROTO proving ref: `crates/mc-protocol/src/frame_tests.rs:110`.
- N3: COPPER and ENDER-CHEST still use whole-commit reversal. Next:
  `python3 -m tools.harness ledger promote COPPER --dry-run` and `… ENDER-CHEST --dry-run` on head; the
  review's concern is that reversal no longer reds its assertion — if either is green or breaks
  fixtures, convert it to a scoped patch-mode control like MINING/ANVIL.
- N4: the new menu closes (crafter slot-state, enchant/lectern button on a cold menu coordinate) still
  have no test; the ingress_proof phrase "every menu input" must be softened or the rows' ingress_proof
  must name place_recipe/select_bundle_item/beacon::configure as out of scope.
- N5: `take_finished` still scans 25 frames; the positive control "finish delivered after the chunk
  returns" and the player-chunk gate on the second source are not done.
