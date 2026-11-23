# SUMMON-AGE: patch-mode fix identity

Original fix: `69650602b` (`fix(summon): rebase onto 7fe49052 — Age for water/horse families, IsBaby for
zombie/piglin/zoglin`; the row's BUGS.md entry names the pre-rebase fix `1e5de7f5`) — it added
`skeleton_horse`/`zombie_horse` and `squid`/`glow_squid` to the `ageable()` family so `Age` is accepted
for them. `fix_commit = "patch:c9e6a2f3467c0e78…"` names the HEAD equivalents of this row's non-test
hunks in `docs/bugs/promotions/SUMMON-AGE.patch` (a diff from the pre-fix form to the candidate; `git
apply -R` restores the pre-fix code). Re-audit:
`python3 -m tools.harness ledger promote SUMMON-AGE --fix-patch docs/bugs/promotions/SUMMON-AGE.patch --dry-run`.
Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's or
owner's behaviour, API the kept tests call, or code a later commit rewrote.

## Carried

- `entity_life.rs @@ -854,6 +859,33 @@ pub(crate) fn ageable` (the family lists) — the two carried
  pairs, HEAD `crates/mc-core/src/entity_life.rs:1137-1138` (`skeleton_horse`, `zombie_horse`) and
  `:1146-1147` (`squid`, `glow_squid`). `ageable()` is what `command_summon`'s `"Age"` support calls.

## Original non-test hunks (complete)

```
## 69650602b non-test hunks (kept test files: crates/mc-core/src/command_summon_age_tests.rs)
- crates/mc-core/src/blueprint.rs: -1337,39 +1337,7
- crates/mc-core/src/command_summon.rs: -206,8 +206,14; -511,6 +517,15; -577,7 +592,7; -585,7 +600,7; -609,7 +624,7; -623,3 +638,7
- crates/mc-core/src/core.rs: -1532,7 +1532,15; -1561,7 +1569,7; -2670,6 +2678,11; -2894,6 +2907,12
- crates/mc-core/src/entity_life.rs: -160,7 +160,7; -737,7 +737,7; -820,8 +820,8; -832,6 +832,7; -840,6 +841,8; -847,6 +850,8; -854,6 +859,33
- crates/mc-core/src/journal.rs: -2385,7 +2385,7
```

## Not carried

- `entity_life.rs -854,6 +859,33` beyond the two carried pairs (the `uses_is_baby` family) — (a)
  SUMMON-ISBABY's own row; its control reverses that function's binding. The other `entity_life` hunks
  (`-160,7`, `-737,7`, `-820,8`, `-832,6`, `-840,6`, `-847,6`) are the same function's earlier lists and
  the `uses_is_baby` helpers; reversing them is not needed for this row's `Age` symptom and their rows
  are the summon-baby row / mob-ageability behaviour (a).
- `command_summon.rs` hunks — (a)/(b): they add the `Age` field plumbing and the `IsBaby` binding; the
  `IsBaby` line is SUMMON-ISBABY's (carried by its control), the `Age` plumbing is API the kept
  `command_summon_age_tests.rs` drives through `supports()` (b: reversing it stops the test from
  reaching the ageable families). The same applies to the `core.rs` summon-command plumbing and
  `journal.rs` persistence (b/a).
- `blueprint.rs -1337,39 +1337,7` — (a) a rebase artefact of the fix branch, no HEAD equivalent.
- Test files — (b) kept.

## Proving refs (file:line of the failing assertion)

- `cargo:mc-core:command_summon::age_tests::summon_age_water_families`
  (`crates/mc-core/src/command_summon_age_tests.rs:50`) and
  `cargo:mc-core:command_summon::age_tests::summon_age_horse_families` (`:65`) — the first summon with
  `Age:-1234` for `squid`/`glow_squid` resp. `skeleton_horse`/`zombie_horse` returns the
  `Entity does not support summon NBT field Age` error once the pairs are reversed (the review's observed
  red).
- Client ref `harness:entities:native-v2-summon-age` is green on the gate-wave tree
  (`20261123T163717-native-v2-summon-age-x8bekgh1`); unrunnable in the control.
