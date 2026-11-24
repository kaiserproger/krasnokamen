# BRUSH-RESET-WRITES-DUSTED-INTO-COLD-CHUNK: patch-mode fix identity

Original fix: `c7bd44dd0` (`fix(residency): keep cold menu, mining, brush, copper and egg writes off the
overlay`) — its `brushing.rs` hunk adds the cold-chunk guard to the reset pass in `run`. At HEAD the guard
is `crates/mc-core/src/brushing.rs:287-295`; the hunk reverse-applies verbatim. `fix_commit =
"patch:c823b5fb1e65415d…"` names it in
`docs/bugs/promotions/BRUSH-RESET-WRITES-DUSTED-INTO-COLD-CHUNK.patch` (also kept under
`docs/bugs/fix-patches/`); `git apply -R` restores the pre-fix code. Re-audit:
`python3 -m tools.harness ledger promote BRUSH-RESET-WRITES-DUSTED-INTO-COLD-CHUNK --fix-patch docs/bugs/promotions/BRUSH-RESET-WRITES-DUSTED-INTO-COLD-CHUNK.patch --dry-run`.
Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's or
owner's behaviour, API the kept tests call, or code a later commit rewrote.

## Original non-test hunks (complete)

```
## c7bd44dd0: only this hunk belongs to BRUSH-RESET-WRITES-DUSTED-INTO-COLD-CHUNK
- crates/mc-core/src/brushing.rs: -284,6 +284,14 (run(): pause the reset pass on a non-resident cell)
```

## Carried

- `brushing.rs @@ -284,6 +284,14 @@` — verbatim at HEAD `crates/mc-core/src/brushing.rs:287-295`: when
  `state.world.has_chunk(pos.x.div_euclid(16), pos.z.div_euclid(16))` is false the entry returns `true`
  (kept alive, no decrement, no write). Reversing it removes the guard, so `block_at` answers from the
  cold overlay and `set_dusted` decrements and writes into it — the row's symptom. The a/-side is the
  prior code of `c7bd44dd0^` (no invented line).

## Not carried

- The rest of `c7bd44dd0` (cold menu, mining, copper, egg, chest-custody hunks) — (a) those rows
  (`MINING-PROGRESS-AND-DEFERRED-FINISH-WIPED-ON-COLD-TARGET`, `COPPER-HONEYCOMB-WAXES-ACROSS-COLD-CHUNK-EDGE`,
  `ENDER-CHEST-MENU-CLICK-THROUGH-COLD-OVERLAY`, …) each own their own control; they are not the reset pass.
- `brushing_tests.rs` of that commit (the cold `brush_reset_pauses…` test) — (b) kept; the test is the
  proving ref and reversing it would stop the control from observing the assertion.

## Proving ref (file:line of the failing assertion)

- `cargo:mc-core:brushing::cold_residency_tests::brush_reset_pauses_on_a_cold_cell_instead_of_decrementing_its_overlay`
  (`crates/mc-core/src/brushing.rs:367`) — with the guard gone the cold-cell half of the test runs the
  reset against the overlay: the observed assertion is the cold-cell `count`/`block_at` pair after
  `run(&mut state)` (`crates/mc-core/src/brushing.rs:388-400`), which decrements 5→3 and writes the dusted
  stage when it must stay 5 and leave the overlay untouched.
- Negative control observed 2026-11-23 (promotion record `docs/bugs/promotions/BRUSH-RESET-WRITES-DUSTED-INTO-COLD-CHUNK.json`):
  the test `red` with the patch reversed, `green` restored. No client-surface ref is registered for this
  row, so no real-client receipt is claimed.
