# Certificate wall-ring control

Row: `CERTIFY-CORNER-MARKS-USE-FEET-INSIDE-WALLS` is **fixed** (promoted on `wt/D123Civ`).

## Production-only patch

[Fix patch](../fix-patches/CERTIFY-CORNER-MARKS-USE-FEET-INSIDE-WALLS.patch)
SHA-256: `6a242fec0b5af8dad07ac5e9ad33363e38d26e7b34e65ef4d3df9489d162b9e0`.
The patch is forward-oriented; reversing it selects the discovered inner air
footprint without its one-block surrounding wall ring. It changes four
production statements in `settlements/building_discovery.rs`, and no tests,
fixtures, API, admission, floor/roof detection or certificate mutation.

## Original-to-current mapping

Original fix: `dc4a7be39b742b9742538304d6242069eafd6610`.

- Its added `include_marked_walls` helper expands the two marked inner floor
  corners by one horizontal block when surrounding wall columns exist.
  The current equivalent is `building_discovery::find`'s four checked
  min-minus-one/max-plus-one assignments. The flood fill already discovers
  the closed air footprint; native inspection still validates the whole box.
- Its call immediately before `buildings::certify` applies that expansion to
  walking marks. The current discovery supplies the inclusive outer bounds
  directly to the same native mutation, replacing that call.
- The marks-only branch, stored-corner comparison and exact typed-bounds
  distinction have no current equivalent: stage 2 removed the marking API
  (`4a91c38823195afe1795b251769a02831b814c8a`; omission reason **c**).
  Room-discovery/type-checklist interfaces used by the kept test remain
  intact (**b**); no a-side API construction is necessary.
- Bound/search guards remain part of the new bounded-discovery API, not a
  second behavior change. The control simply stops expanding the box; it
  does not disable bounds or native geometry validation.

## Proving check and observed red

`cargo:mc-core:settlements::ui::tests::standing_inside_a_finished_house_certifies_its_floor_wall_and_roof_bounds`

The proving assertion is the stored certificate's inclusive bounds comparison
in `ui_tests.rs` (line 845 at this run), after the real dialog-click path has
inspected and certified the finished workshop house. The unchanged test expects
`[1, 200, 1]..[5, 204, 5]`; reversing only the patch stored
`[2, 200, 2]..[4, 204, 4]` and failed that assertion. This is a native certificate
volume observation, not a compile error, setup failure or UI-text match.

Command, through Main's `mem-run.sh -m 6G`, background build lane:

```text
python3 -m tools.harness run test -p mc-core --lib -E 'test(=settlements::ui::tests::standing_inside_a_finished_house_certifies_its_floor_wall_and_roof_bounds)'
```

Red receipt (compiled, 1 test failed):
`/run/media/kaiserroman/games/solaris-work/wt/SettlementQuickWins/.analysis/validation/20261119T224709-test-us4vh5se/test.log`.

Restore receipt (136/136 UI/building checks passed, including this exact check):
`/run/media/kaiserroman/games/solaris-work/wt/SettlementQuickWins/.analysis/validation/20261119T225025-test-r053fjxz/test.log`.
Only the four production assignments were restored; the proving test was
unchanged between these runs. Both runs include the stage-1 plural/map-width
integration and kind-first Wall/Gate discovery correction. This unit control
is not real-client acceptance or ledger promotion.
