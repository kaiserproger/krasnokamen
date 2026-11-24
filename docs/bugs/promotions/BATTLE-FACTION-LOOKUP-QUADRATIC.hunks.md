# BATTLE-FACTION-LOOKUP-QUADRATIC — fix-hunk map

The production-only patch `BATTLE-FACTION-LOOKUP-QUADRATIC.patch` is the fix's non-test hunks at HEAD,
SHA-256 of its non-test hunks
`patch:61946875bf4fba8fca2e96d719d5748d160fc0399248ea2ee6f198104b3a3672`. It reverse-applies on the
candidate (`git apply -R --check` on `5f3415769`, clean tree).

## Scope of the fix (narrow form, Opus design verdict)

`review/REVIEW-ReviewD123BattleDesign-Opus-perf.md` §3 rejected the row's original faction table and
narrowed the fix to the three real quadratic spots: one linear `find` per body in `UnitCounters::record`
and one linear `contains` per body in `TargetGrid::rebuild`. The per-faction loop in `nearest_enemy` /
`any_enemy` is **not** touched: its visit order is observable in combat (`combat.rs:634`, `:775`,
`live_players.rs:599`). The residual O(F) per retarget is recorded on the row.

## The fix commit and the hunks the patch carries

The fix is `5f3415769`, contained in this tree. For this row it touched:

| file | hunk | in the patch? |
|---|---|---|
| `crates/mc-core/src/battle/mod.rs` | `UnitCounters::record` linear `find` → `index` hash probe | **yes** |
| `crates/mc-core/src/battle/targeting.rs` | `TargetGrid::rebuild` `factions.contains` → `present` `BTreeSet` insert, published list from the sorted set | **yes** |
| `crates/mc-core/src/battle/tests.rs` | the BTreeMap oracle test and the registered one-lookup-per-unit test | no — test file (kept whole by the splitter) |

Not carried by the patch (both sides identical, so they never reverse):

- the `UnitCounters::index` field and its maintenance in `clear`/`finish`;
- the `TargetGrid::present` field and its `clear`;
- the test-only `RECORD_LOOKUPS` / `REBUILD_LOOKUPS` statics and the `#[cfg(test)]` blocks that count.

## The a/-side against the real pre-image

Compared with the file at `5f3415769^`, the reversed tree differs in exactly these ways:

1. **Kept derived state.** The control keeps the `index` map (declared, cleared, rebuilt in `finish`)
   and the `present` set (declared, cleared) but `record`/`rebuild` no longer read them, so the
   lookup work is the pre-fix linear scan while the struct layout and every other method are HEAD's.
   This is the a/-side adaptation: dead state that changes no behaviour, kept so the patch is the
   lookup hunk only and the kept refs compile unchanged.
2. **Test-only counters, class (b).** `RECORD_LOOKUPS` / `REBUILD_LOOKUPS` and their `#[cfg(test)]`
   increments are API the registered refs read, so the control keeps them (rule (b)); the increments
   are placed where the pre-fix work happens:
   - `record`: `+1` on the linear-find hit, `+len+1` on the miss (the miss scans the whole published
     vector and then pushes one more entry, which is why the miss arm is not `+len`);
   - `rebuild`: `+factions.len()` per qualifying unit (the pre-fix `contains` has to visit the factions
     already present; it stops at a match, so this is an upper-bound count on comparisons, not the
     exact comparison count).
3. **Test-only accessor.** `TargetGrid::factions()` (`#[cfg(test)]`) is kept — the registered test reads
   it. It is a pure getter and does not exist in the pre-image.
4. `UnitCounters::record`'s `let Some(count) = self.alive_per_faction.iter_mut().find(…)` is spelled on
   one line in the a/-side (rustfmt collapsed the pre-image's five-line chain). Same expression, same
   order; formatting only.

## What the counters prove

- `UnitCounters::record`: the fixed tree performs **one** faction lookup per counted unit
  (`RECORD_LOOKUPS` grows by exactly `units`), the control performs a linear scan per unit, so its
  total is the sum of `len+1` over the first-arrival misses plus one per hit — O(F²) for F distinct
  factions, far above `units` at F=64.
- `TargetGrid::rebuild`: the fixed tree performs exactly one lookup per qualifying unit
  (`REBUILD_LOOKUPS` grows by `units.len()`); the control adds the present-faction count per unit, so
  its total is the sum of that count over every unit — again O(F²) in the arrival order, not
  `units.len()`.
- The oracle test `faction_population_matches_a_btreemap_oracle_at_sixty_four_factions` is an
  independent `BTreeMap` comparison (never `UnitCounters::sample`, which calls `record` itself) and is
  green on both sides: it proves the published `(faction, alive)` vector and `units` are unchanged by
  the index.
- Asserts are on counts, not wall clock. No insertion-order or sort claim beyond `finish`'s existing
  `sort_unstable_by_key`, which the index rebuild follows.

## Expected red

`crates/mc-core/src/battle/tests.rs:1909-1914` (`assert_eq!(grid_work, units.len(), "one faction lookup
per unit, not one per visited faction")`), and, if the grid assert is passed,
`tests.rs:1925-1929` for `record`. The oracle test stays green in the control — a green oracle is the
proof the reversal changes lookup work, not the answer.

## Observed control and restore

`ledger promote BATTLE-FACTION-LOOKUP-QUADRATIC`, receipt
`.analysis/validation/20261124T012659-ledger-promote-srkqbp1l` on `5f3415769` (`tree_dirty: false`,
`control_tree: head`, `reversed_files` = `battle/mod.rs`, `battle/targeting.rs`):

- control (`ledger-1-run.log:567-569`) **FAIL** on the registered lookup test —
  `assertion `left == right` failed: one faction lookup per unit, not one per visited faction`,
  `left: 6153`, `right: 144` — the grid's per-unit present-faction count against `units.len()`
  (144), i.e. the quadratic scan, at its own assertion; the oracle test stays **green**;
- restore green (`ledger-3-run.log`, exit 0; both refs).

## What stays out of the row

`nearest_enemy`/`any_enemy` keep their per-faction loop (observable order); `Search`'s cell walk and the
`stable_id` tie-break are untouched. `population` and `grid` are `serde(skip)`, so no persistence
surface changes.
