# BATTLE-FACTION-LOOKUP-QUADRATIC — fix-hunk map

The production-only patch `BATTLE-FACTION-LOOKUP-QUADRATIC.patch` is the fix's non-test hunks, SHA-256
`patch:b9c433bd7895940c376ebad7977b3295450639e04cb9dfafe2d42910ebec51be`. It reverse-applies on the
candidate (`git apply -R --check` on `071a68f8e4`, clean tree).

## The fix and the hunks the patch carries

| file | hunk | in the patch? |
|---|---|---|
| `crates/mc-core/src/battle/mod.rs` | `UnitCounters::record` scans `alive_per_faction` with `.iter_mut().find(...)`; the fix probes it with `binary_search_by` through `faction_cmp` and inserts at the slot; `finish`'s sort becomes a no-op and the doc states the ascending invariant | **yes** |
| `crates/mc-core/src/battle/targeting.rs` | `TargetGrid::rebuild` checked `factions.contains(...)` per unit then sorted; the fix `binary_search_by`-inserts at the slot through the same `faction_cmp` and drops the sort and the scratch set | **yes** |
| `crates/mc-core/src/battle/tests.rs` | the registered comparison-count test and the ascending-set test | no — test file (kept whole by the splitter) |

## The shared counted comparator (the seam both sides carry)

`faction_cmp(row, faction) -> Ordering` beside `FactionAlive` is one function with one text in the a/-
and b/-side; it is where every faction comparison of both tables happens:

- fixed `record`: `binary_search_by(|count| faction_cmp(count.faction, faction))` — at most
  ⌈log2(len+1)⌉ comparisons per sample;
- fixed `rebuild`: `binary_search_by(|row| faction_cmp(*row, unit.faction))` — same bound per unit;
- pre-fix `record`: `.position(|c| faction_cmp(c.faction, faction).is_eq())` — one comparison per visited
  faction;
- pre-fix `rebuild`: `.any(|row| faction_cmp(*row, unit.faction).is_eq())` — one per visited faction.

The counter (`FACTION_COMPARES`, `#[cfg(test)]`) lives inside `faction_cmp`, so it counts what each
implementation actually does on BOTH trees; there is no increment that exists only on the a/-side.

## The a/-side against the real pre-image

The reversed tree is compared with `5f3415769^:crates/mc-core/src/battle/{mod,targeting}.rs`. Every
difference:

1. **`faction_cmp` + `FACTION_COMPARES`** (the seam above) and the pre-fix call sites routed through it —
   the only adaptation (README rule (b): the registered test reads the counter, so the control needs the
   same seam). No `#[allow]`, no invented comment, no counter increment outside `faction_cmp`.
2. **`#[cfg(test)] impl TargetGrid { fn factions() }`** — the test-only accessor the registered test
   reads; identical text on both sides, no behaviour.
3. Nothing else: `record`'s body is the pre-fix scan, `rebuild`'s is the pre-fix `contains`+push+sort,
   `finish` keeps the pre-fix sort, and `UnitCounters`/`TargetGrid` keep exactly their pre-fix fields
   (no index map, no scratch set, no sorted-Vec bookkeeping beyond what the pre-fix table had).

## Why this control is honest

The registered ref is `cargo:mc-core:battle::tests::faction_probes_stay_logarithmic_per_sample_and_per_rebuild`.
At 64 factions with 1..=3 shuffled units each it asserts, for `record` and for `rebuild` separately,
`units.len() <= compares <= units.len() * 9` (⌈log2(192+1)⌉ = 8, so 9 with margin). On the control the
pre-fix scans visit a growing prefix of rows per unit (the first pass alone misses ~2016 rows), so the
upper bound fails by an order of magnitude — a real red on the algorithm, not on an increment. The
`faction_population_matches_a_btreemap_oracle_at_sixty_four_factions` oracle test stays and is not
evidence (it is green on both trees).

## Observed control and restore

Earlier control (counter increments invented on the a/-side; superseded): `.analysis/validation/20261124T012659-ledger-promote-srkqbp1l`.
This regenerated patch's control is recorded below once re-promoted.

## What stays out of the row

`nearest_enemy`/`any_enemy` still walk the published factions in order (observable visit order); the fix
removes only the per-unit faction scan of the two tables. No wall-clock claim is made.
