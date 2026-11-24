# SETTLEMENT-NATURAL-LUMBER-ZONE-ONE-COLUMN — the grove fix and its control (CIV-6, 2026-11-24)

Row fix identity `patch:db2a7e6f824238b4faa0b7a094ae4990253a9d78605bb9aca3a0346544c7f3b4`
(`promotions/SETTLEMENT-NATURAL-LUMBER-ZONE-ONE-COLUMN.patch`), from commit
`8a93c14578` ("fix(settlements): certify a natural lumber camp from its grove",
branch `wt/D123Civ`). Reverse-applies at `8a93c14578` (`git apply -R --check`).

## Why the zone was one column

`natural_zones` probes a tree, then queues a candidate whose bounds are that
trunk column plus the chosen entrance cell; `prove_natural` certified those
bounds unchanged, so `certify_zone` saw a `1x8x1` (or `2x1`) rectangle that only
had to pass ">= 4 logs". The gate's phase log (run `20261124T020538`) found
exactly that: certified `[125,75,-181]..[125,82,-180]`, and a lumberjack whose
`plant_zone` walk could therefore only ever cover one trunk.

## The fix (one commit, one production file)

`crates/mc-core/src/settlements/autonomy.rs`:

- `GROVE_RADIUS = 6` and `grove_bounds()`: starting from the proved candidate
  rectangle, each horizontal side grows one column at a time while
  - the rectangle's eight corners stay inside the settlement sphere (the same
    rule `buildings::certify_zone`'s `within` applies),
  - the added columns are loaded (`bounds_loaded`), so no worker is ever routed
    into a chunk that never ticks, and
  - no certificate or generated village piece already stands in the rectangle
    (`queue_natural` refuses a tree inside a piece for the same reason).
- `prove_natural` grows only a `LumberCamp` candidate, at certification time, and
  keeps the plain column as the fallback: if `certify_zone` refuses the wider
  rectangle (overlap, fewer than four zone logs, entrance), it certifies the
  original bounds exactly as before. A grove is an improvement, never a new
  failure. The probe cost is bounded: at most `4 * GROVE_RADIUS` corner-and-chunk
  checks per certification, no world volume walk and no new per-tick work.
- The `site` parameter on `prove_natural` is the generated village's pieces; the
  kept test call sites pass `None`, and `grove_bounds` then clips only against
  certificates.

## Control map (a/-side, class (b) omission)

The patch's a/-side is `8a93c14578` minus this commit's own additions, with one
named adaptation: the `prove_natural` signature keeps
`_site: Option<&GeneratedVillageSite>`, because the kept tests
(`autonomy_tests.rs` direct `prove_natural` call sites) pass five arguments —
class (b) "API the kept tests call". The parameter is unused in the control
(`_site`), which changes no behaviour.

## Red and green (both observed)

- Reversed tree (same tree, this patch reversed): the new test
  `settlements::autonomy::tests::natural_lumber_camp_certifies_the_grove_not_one_tree_column`
  fails at its own assertion —
  `a natural camp's zone is the grove, not one tree column: 2x1 at [-15, 201, 4]..[-14, 206, 4]`
  (`20261124T125830-test-vvf5sfsu`, exit 100).
- Fixed tree: the same test passes (`20261124T121009-test-vgv1iyl7`).
- `natural_` filter: 114 tests, 114 passed (`20261124T124331-test-kd49vn1t`),
  including `natural_grove_ignores_structure_logs_and_finds_an_unblocked_side`
  (the grove stops before a generated house) and
  `natural_lumber_and_mine_zones_certify_after_deferred_route_proofs`.
- The earlier per-tree attempt's failure is not repeated:
  `villager::building_autonomy_tests::adopted_village_with_resident_shears_produces_stock_and_builds`
  passes (`20261124T124832-test-x91787h3`).

## Still missing

`harness:civilization-seed-qa:all` needs a passed gate receipt for this row.
The gate was started on the pin of `8a93c14578`
(`pins/8a93c14578a1-server-…`) with `MC_CORE_PHASE_LOG=1`; see the lane
HANDOFF for the run and its outcome.
