# COPPER-HONEYCOMB-WAXES-ACROSS-COLD-CHUNK-EDGE: patch-mode fix identity

Original fix: `c7bd44dd06` (`fix(residency): keep cold menu, mining, brush, copper and egg writes off the
overlay`) — the copper use guard (a cold cell must not contribute its overlay or debit an item) and the
double-chest partner pass guard (a cold partner is skipped instead of read/written from the overlay). The
whole commit no longer reverses on HEAD (the mining/menu/egg hunks were rewritten by `b460f313cc`/`ae56fe9f3`
and by this lane's doc edits), so the row moves to a scoped patch: `fix_commit =
"patch:96e7131930b6edfb…"` names the HEAD equivalents of the copper hunks only, in
`docs/bugs/promotions/COPPER-HONEYCOMB-WAXES-ACROSS-COLD-CHUNK-EDGE.patch` (a diff from the pre-fix form
`c7bd44dd06^:crates/mc-core/src/copper.rs` to the candidate; `git apply -R` restores the pre-fix code).
Re-audit:
`python3 -m tools.harness ledger promote COPPER-HONEYCOMB-WAXES-ACROSS-COLD-CHUNK-EDGE --fix-patch docs/bugs/promotions/COPPER-HONEYCOMB-WAXES-ACROSS-COLD-CHUNK-EDGE.patch --dry-run`.
Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's or
owner's behaviour, API the kept tests call, or code a later commit rewrote.

## Carried

- `copper.rs @@ -220,6 +220,14 @@ pub(crate) fn use_on` — the cell residency guard before any
  wax/scrape work; HEAD `crates/mc-core/src/copper.rs:223-230`.
- `copper.rs @@ -298,6 +306,9 @@ pub(crate) fn use_on` — the double-chest partner guard
  (`has_chunk(other...) && let Some(old) = ...`); HEAD `crates/mc-core/src/copper.rs:310-312`.

## Original non-test hunks (complete)

```
## c7bd44dd06 non-test hunks (kept test files: brushing_tests.rs, copper_tests.rs, ender_chest_tests.rs, interact_parity_tests.rs, tick/mining_tests.rs, tick/stations_tests.rs)
- crates/mc-core/src/brushing.rs: -284,6 +284,14; -296,3 +304,7
- crates/mc-core/src/copper.rs: -220,6 +220,14; -298,6 +306,9
- crates/mc-core/src/tick.rs: -75,6 +75,9
- crates/mc-core/src/tick/containers.rs: -4,6 +4,20
- crates/mc-core/src/tick/input/inventory.rs: -225,6 +225,10
- crates/mc-core/src/tick/mining.rs: -38,8 +38,46; -95,6 +133,20
- crates/mc-core/src/tick/stations.rs: -139,6 +139,11
- crates/mc-core/src/vanilla_interact.rs: -125,8 +125,11
```

## Not carried

- `brushing.rs`, `tick/containers.rs`, `tick/input/inventory.rs`, `tick/mining.rs`, `tick/stations.rs`,
  `vanilla_interact.rs`, `tick.rs` — (a) BRUSH-RESET-WRITES-DUSTED-INTO-COLD-CHUNK,
  ANVIL-DAMAGE-WRITES-AT-COLD-MENU-COORDINATE, ENDER-CHEST-MENU-CLICK-THROUGH-COLD-OVERLAY,
  MINING-PROGRESS-AND-DEFERRED-FINISH-WIPED-ON-COLD-TARGET and DRAGON-EGG-TELEPORTS-INTO-COLD-CHUNK-OVERLAY;
  each is carried by its own row's control. (c) for the parts those rows later rewrote.
- Test files — (b) kept so the control compiles and runs the ref.

## Proving ref (file:line of the failing assertion)

- `cargo:mc-core:copper::tests::copper_waxing_and_scraping_wait_for_a_resident_cell_and_spare_a_cold_partner`
  — with the two guards reversed the cold cell waxes (its overlay image changes and the honeycomb is
  spent) and the cold partner is read from the overlay; the test's own wax/scrape/partner assertions
  (the review's observed red: "not waxed" at `copper_tests.rs:1910`) fail.
